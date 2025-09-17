use anyhow::Result;
use log::{debug, warn};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use futures_util::stream::StreamExt;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use std::collections::VecDeque;

/// Helper function to log memory usage for monitoring
fn log_memory_usage(buffer_size: usize, operation: &str) {
    let kb = buffer_size as f64 / 1024.0;
    let mb = kb / 1024.0;
    if mb >= 1.0 {
        debug!("{}: Using {:.1} MB ({:.0} KB) of memory", operation, mb, kb);
    } else {
        debug!("{}: Using {:.0} KB of memory", operation, kb);
    }
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Started(String),
    Stopped,
    Paused,
    Resumed,
    Error(String),
    StreamConnected,
    BufferProgress(usize), // bytes buffered
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Connecting,
    Error(String),
}

/// Consolidated player state to avoid multiple mutex locks
struct PlayerState {
    current_url: Option<String>,
    playback_state: PlaybackState,
    sink: Option<Sink>,
    cancellation_token: Option<CancellationToken>,
    auto_reconnect: bool,
    reconnect_attempts: u32,
}

impl std::fmt::Debug for PlayerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerState")
            .field("current_url", &self.current_url)
            .field("playback_state", &self.playback_state)
            .field("sink", &self.sink.as_ref().map(|_| "Some(Sink)"))
            .field("cancellation_token", &self.cancellation_token.as_ref().map(|_| "Some(Token)"))
            .field("auto_reconnect", &self.auto_reconnect)
            .field("reconnect_attempts", &self.reconnect_attempts)
            .finish()
    }
}

impl PlayerState {
    fn new() -> Self {
        Self {
            current_url: None,
            playback_state: PlaybackState::Stopped,
            sink: None,
            cancellation_token: None,
            auto_reconnect: true,
            reconnect_attempts: 0,
        }
    }

    fn is_playing(&self) -> bool {
        matches!(self.playback_state, PlaybackState::Playing)
    }

    fn is_paused(&self) -> bool {
        matches!(self.playback_state, PlaybackState::Paused)
    }

    fn set_state(&mut self, state: PlaybackState) {
        debug!("Player state changing from {:?} to {:?}", self.playback_state, state);
        self.playback_state = state;
    }
}

pub struct SimpleAudioPlayer {
    state: Arc<Mutex<PlayerState>>,
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    event_sender: watch::Sender<PlayerEvent>,
    event_receiver: watch::Receiver<PlayerEvent>,
}

impl SimpleAudioPlayer {
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) = OutputStream::try_default()?;
        let (event_sender, event_receiver) = watch::channel(PlayerEvent::Stopped);

        Ok(Self {
            state: Arc::new(Mutex::new(PlayerState::new())),
            _stream: stream,
            stream_handle,
            event_sender,
            event_receiver,
        })
    }

    /// Get a receiver for player events
    pub fn event_receiver(&self) -> watch::Receiver<PlayerEvent> {
        self.event_receiver.clone()
    }

    /// Get the current URL being played
    pub fn current_url(&self) -> Option<String> {
        if let Ok(state) = self.state.lock() {
            state.current_url.clone()
        } else {
            None
        }
    }

    /// Get the current playback state
    pub fn playback_state(&self) -> PlaybackState {
        if let Ok(state) = self.state.lock() {
            state.playback_state.clone()
        } else {
            PlaybackState::Error("Failed to acquire state lock".to_string())
        }
    }

    /// Enable or disable automatic reconnection
    pub fn set_auto_reconnect(&self, enabled: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.auto_reconnect = enabled;
        }
    }

    /// Graceful shutdown - stops playback and cancels all tasks
    pub fn shutdown(&self) -> Result<()> {
        debug!("Shutting down audio player");
        self.stop()?;
        
        // Clear any remaining state
        if let Ok(mut state) = self.state.lock() {
            state.cancellation_token = None;
        }
        
        debug!("Audio player shutdown complete");
        Ok(())
    }

    pub fn play(&self, url: String) -> Result<()> {
        debug!("Playing audio from URL: {}", url);

        // Stop any current playback first
        self.stop()?;

        // Create cancellation token for this playback session
        let cancellation_token = CancellationToken::new();

        // Update state to connecting
        {
            let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
            state.current_url = Some(url.clone());
            state.set_state(PlaybackState::Connecting);
            state.cancellation_token = Some(cancellation_token.clone());
            state.reconnect_attempts = 0;
        }

        // Send connecting event
        let _ = self.event_sender.send(PlayerEvent::Started(url.clone()));

        // Spawn the streaming task
        let state_clone = self.state.clone();
        let stream_handle = self.stream_handle.clone();
        let event_sender = self.event_sender.clone();
        let url_clone = url.clone();

        tokio::spawn(async move {
            let result = Self::stream_with_retry(
                url_clone,
                state_clone,
                stream_handle,
                event_sender,
                cancellation_token
            ).await;

            if let Err(e) = result {
                warn!("Streaming task failed: {}", e);
            }
        });

        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
        
        if let Some(sink) = state.sink.as_ref() {
            sink.pause();
            state.set_state(PlaybackState::Paused);
            let _ = self.event_sender.send(PlayerEvent::Paused);
            debug!("Audio paused");
        }
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
        
        if let Some(sink) = state.sink.as_ref() {
            sink.play();
            state.set_state(PlaybackState::Playing);
            let _ = self.event_sender.send(PlayerEvent::Resumed);
            debug!("Audio resumed");
        }
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
        
        // Cancel any ongoing streaming task
        if let Some(token) = state.cancellation_token.take() {
            token.cancel();
        }

        // Stop the sink
        if let Some(sink) = state.sink.take() {
            sink.stop();
        }

        // Reset state
        state.current_url = None;
        state.set_state(PlaybackState::Stopped);
        state.reconnect_attempts = 0;
        
        let _ = self.event_sender.send(PlayerEvent::Stopped);
        debug!("Audio stopped");
        Ok(())
    }

    pub fn is_playing(&self) -> bool {
        if let Ok(state) = self.state.lock() {
            state.is_playing()
        } else {
            false
        }
    }

    pub fn is_paused(&self) -> bool {
        if let Ok(state) = self.state.lock() {
            state.is_paused()
        } else {
            false
        }
    }

    /// Main streaming function with automatic retry logic
    async fn stream_with_retry(
        url: String,
        state: Arc<Mutex<PlayerState>>,
        stream_handle: OutputStreamHandle,
        event_sender: watch::Sender<PlayerEvent>,
        cancellation_token: CancellationToken,
    ) -> Result<()> {
        const MAX_RETRY_ATTEMPTS: u32 = 5;
        const RETRY_DELAY_MS: u64 = 2000;

        loop {
            // Check if we should retry
            let should_retry = {
                let state_guard = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
                state_guard.auto_reconnect && state_guard.reconnect_attempts < MAX_RETRY_ATTEMPTS
            };

            if !should_retry {
                break;
            }

            // Check for cancellation
            if cancellation_token.is_cancelled() {
                debug!("Streaming cancelled");
                break;
            }

            // Resolve the stream URL
            let actual_url = match resolve_stream_url(&url).await {
                Ok(resolved_url) => resolved_url,
                Err(e) => {
                    warn!("Failed to resolve stream URL: {}. Using original URL.", e);
                    url.clone()
                }
            };

            // Attempt to stream
            match Self::fetch_and_play_stream(
                &actual_url,
                &stream_handle,
                &state,
                &event_sender,
                &cancellation_token,
            ).await {
                Ok(_) => {
                    debug!("Stream ended normally");
                    break;
                }
                Err(e) => {
                    warn!("Stream failed: {}", e);
                    
                    // Increment retry attempts
                    {
                        let mut state_guard = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
                        state_guard.reconnect_attempts += 1;
                        
                        if state_guard.reconnect_attempts >= MAX_RETRY_ATTEMPTS {
                            state_guard.set_state(PlaybackState::Error(format!("Max retry attempts reached: {}", e)));
                            let _ = event_sender.send(PlayerEvent::Error(format!("Max retry attempts reached: {}", e)));
                            break;
                        }
                    }

                    // Send error event and wait before retry
                    let _ = event_sender.send(PlayerEvent::Error(format!("Stream error, retrying... ({})", e)));
                    
                    // Wait before retry, but respect cancellation
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)) => {},
                        _ = cancellation_token.cancelled() => {
                            debug!("Retry cancelled");
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Improved streaming with bounded buffering and backpressure handling
    async fn fetch_and_play_stream(
        url: &str,
        stream_handle: &OutputStreamHandle,
        state: &Arc<Mutex<PlayerState>>,
        event_sender: &watch::Sender<PlayerEvent>,
        cancellation_token: &CancellationToken,
    ) -> Result<()> {
        debug!("Fetching stream from URL: {}", url);

        // Create bounded channel for audio chunks
        let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(32); // Buffer up to 32 chunks

        // Create sink for this stream
        let sink = Sink::try_new(stream_handle)?;

        // Update state with the new sink
        {
            let mut state_guard = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
            state_guard.sink = Some(sink);
            state_guard.set_state(PlaybackState::Playing);
        }

        let _ = event_sender.send(PlayerEvent::StreamConnected);

        // Spawn network fetching task
        let url_clone = url.to_string();
        let cancellation_clone = cancellation_token.clone();
        let event_sender_clone = event_sender.clone();
        
        let fetch_task = tokio::spawn(async move {
            Self::fetch_network_stream(url_clone, audio_tx, cancellation_clone, event_sender_clone).await
        });

        // Main decoding and playback loop
        let mut decode_buffer = VecDeque::new();
        let mut total_bytes = 0usize;
        const MIN_DECODE_SIZE: usize = 64 * 1024; // 64KB minimum before attempting decode
        const MAX_BUFFER_SIZE: usize = 512 * 1024; // 512KB maximum buffer

        loop {
            tokio::select! {
                // Receive audio data from network
                chunk_opt = audio_rx.recv() => {
                    match chunk_opt {
                        Some(chunk) => {
                            total_bytes += chunk.len();
                            decode_buffer.extend(chunk);
                            
                            log_memory_usage(decode_buffer.len(), "Audio buffer");
                            let _ = event_sender.send(PlayerEvent::BufferProgress(decode_buffer.len()));

                            // Try to decode when we have enough data
                            if decode_buffer.len() >= MIN_DECODE_SIZE {
                                if let Err(e) = Self::try_decode_and_play(&mut decode_buffer, state).await {
                                    debug!("Decode attempt failed: {}", e);
                                }
                            }

                            // Prevent buffer from growing too large
                            if decode_buffer.len() > MAX_BUFFER_SIZE {
                                warn!("Audio buffer too large ({}KB), dropping old data", decode_buffer.len() / 1024);
                                // Drop the first quarter of the buffer
                                let drop_size = decode_buffer.len() / 4;
                                decode_buffer.drain(..drop_size);
                            }
                        }
                        None => {
                            debug!("Network stream ended");
                            break;
                        }
                    }
                }
                
                // Check for cancellation
                _ = cancellation_token.cancelled() => {
                    debug!("Stream playback cancelled");
                    break;
                }
            }
        }

        // Wait for fetch task to complete
        let _ = fetch_task.await;

        debug!("Stream playback ended, total bytes: {}KB", total_bytes / 1024);
        Ok(())
    }

    /// Network fetching task with backpressure handling
    async fn fetch_network_stream(
        url: String,
        audio_tx: mpsc::Sender<Vec<u8>>,
        cancellation_token: CancellationToken,
        _event_sender: watch::Sender<PlayerEvent>,
    ) -> Result<()> {
        // Create HTTP client with proper settings for streaming
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("HTTP error: {}", response.status()));
        }

        debug!("Connected to network stream");
        let mut stream = response.bytes_stream();
        let mut total_bytes = 0usize;

        while let Some(chunk_result) = stream.next().await {
            // Check for cancellation
            if cancellation_token.is_cancelled() {
                debug!("Network fetch cancelled");
                break;
            }

            let chunk = chunk_result?;
            total_bytes += chunk.len();

            // Send chunk with backpressure handling
            match audio_tx.try_send(chunk.to_vec()) {
                Ok(_) => {
                    // Successfully sent
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Channel is full, apply backpressure by waiting a bit
                    debug!("Audio buffer full, applying backpressure");
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
                        _ = cancellation_token.cancelled() => break,
                    }
                    // Try again after waiting
                    if let Err(e) = audio_tx.send(chunk.to_vec()).await {
                        warn!("Failed to send audio chunk after backpressure: {}", e);
                        break;
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    debug!("Audio channel closed");
                    break;
                }
            }

            // Log progress periodically
            if total_bytes % (512 * 1024) == 0 && total_bytes > 0 {
                debug!("Network fetched {} KB so far", total_bytes / 1024);
            }
        }

        debug!("Network stream fetch completed, total bytes: {}KB", total_bytes / 1024);
        Ok(())
    }

    /// Try to decode accumulated buffer and add to sink
    async fn try_decode_and_play(
        decode_buffer: &mut VecDeque<u8>,
        state: &Arc<Mutex<PlayerState>>,
    ) -> Result<()> {
        // Convert buffer to Vec for cursor - we need to own the data
        let buffer_vec: Vec<u8> = decode_buffer.iter().copied().collect();
        let cursor = Cursor::new(buffer_vec);

        match Decoder::new(cursor) {
            Ok(source) => {
                debug!("Successfully decoded {} bytes", decode_buffer.len());
                
                // Add to sink
                if let Ok(state_guard) = state.lock() {
                    if let Some(sink) = state_guard.sink.as_ref() {
                        sink.append(source);
                        
                        // Start playback if not already playing
                        if !sink.is_paused() {
                            sink.play();
                        }
                    }
                }
                
                // Clear the buffer after successful decode
                decode_buffer.clear();
                Ok(())
            }
            Err(e) => {
                // If decode fails with insufficient data, keep the buffer
                // If it's a real error, we might want to drop some data
                Err(anyhow::anyhow!("Decode failed: {}", e))
            }
        }
    }
}

async fn resolve_stream_url(url: &str) -> Result<String> {
    // If it's a direct stream URL, return as is
    if url.ends_with(".mp3") || url.ends_with(".aac") || url.contains("/live") {
        return Ok(url.to_string());
    }

    // If it's a playlist file (.pls, .m3u, etc.), fetch and parse it
    if url.ends_with(".pls") || url.ends_with(".m3u") || url.ends_with(".m3u8") {
        return parse_playlist(url).await;
    }

    // Default: return the original URL
    Ok(url.to_string())
}

async fn parse_playlist(playlist_url: &str) -> Result<String> {
    debug!("Parsing playlist from URL: {}", playlist_url);

    let client = reqwest::Client::new();
    let response = client.get(playlist_url).send().await?;
    let content = response.text().await?;

    debug!("Playlist content: {}", content);

    // Parse .pls format
    if playlist_url.ends_with(".pls") {
        for line in content.lines() {
            if line.starts_with("File") && line.contains("=") {
                if let Some(url) = line.split('=').nth(1) {
                    debug!("Found stream URL in playlist: {}", url);
                    return Ok(url.trim().to_string());
                }
            }
        }
    }

    // Parse .m3u/.m3u8 format
    if playlist_url.ends_with(".m3u") || playlist_url.ends_with(".m3u8") {
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                debug!("Found stream URL in m3u playlist: {}", line);
                return Ok(line.to_string());
            }
        }
    }

    Err(anyhow::anyhow!("No stream URL found in playlist"))
}