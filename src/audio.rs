use anyhow::Result;
use log::{debug, warn};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use futures_util::stream::StreamExt;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::codecs::DecoderOptions;
// use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::audio::Signal;
use symphonia::default::{get_codecs, get_probe};

/// A wrapper so we can feed network chunks into Symphonia
struct StreamingSource {
    buffer: Arc<tokio::sync::Mutex<Vec<u8>>>,
    pos: Arc<Mutex<usize>>,
}

impl StreamingSource {
    fn new() -> (Self, Arc<tokio::sync::Mutex<Vec<u8>>>, Arc<Mutex<usize>>) {
        let buffer = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let pos = Arc::new(Mutex::new(0));
        let source = Self {
            buffer: buffer.clone(),
            pos: pos.clone(),
        };
        (source, buffer, pos)
    }
}

impl Read for StreamingSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Use try_lock to avoid blocking since Read trait is synchronous
        let buffer = self.buffer.try_lock().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "buffer locked")
        })?;

        let mut pos = self.pos.lock().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "failed to lock position")
        })?;

        if *pos >= buffer.len() {
            return Ok(0); // no new data yet
        }

        let n = std::cmp::min(buf.len(), buffer.len() - *pos);
        buf[..n].copy_from_slice(&buffer[*pos..*pos + n]);
        *pos += n;

        // Note: Buffer cleanup is now handled entirely by the network task
        // This read function only advances position

        Ok(n)
    }
}

impl Seek for StreamingSource {
    fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "seek not supported"))
    }
}

impl MediaSource for StreamingSource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Custom rodio Source that streams directly from Symphonia AudioBufferRef
/// Avoids per-packet Vec<f32> allocations for better performance
pub struct SymphoniaStreamSource {
    samples: std::collections::VecDeque<f32>,
    sample_rate: u32,
    channels: u16,
    finished: bool,
}

impl SymphoniaStreamSource {
    fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            samples: std::collections::VecDeque::new(),
            sample_rate,
            channels,
            finished: false,
        }
    }

    /// Push samples directly from AudioBufferRef without intermediate Vec allocation
    fn push_audio_buffer(&mut self, audio_buf: &symphonia::core::audio::AudioBufferRef) {
        let spec = *audio_buf.spec();
        let chans = spec.channels.count();
        let frames = audio_buf.frames();

        // Direct conversion without intermediate Vec allocation
        match audio_buf {
            symphonia::core::audio::AudioBufferRef::F32(buf) => {
                for frame in 0..frames {
                    for ch in 0..chans {
                        let plane = buf.chan(ch);
                        self.samples.push_back(plane[frame]);
                    }
                }
            }
            symphonia::core::audio::AudioBufferRef::F64(buf) => {
                for frame in 0..frames {
                    for ch in 0..chans {
                        let plane = buf.chan(ch);
                        self.samples.push_back(plane[frame] as f32);
                    }
                }
            }
            symphonia::core::audio::AudioBufferRef::S16(buf) => {
                for frame in 0..frames {
                    for ch in 0..chans {
                        let plane = buf.chan(ch);
                        self.samples.push_back(plane[frame] as f32 / i16::MAX as f32);
                    }
                }
            }
            symphonia::core::audio::AudioBufferRef::S32(buf) => {
                for frame in 0..frames {
                    for ch in 0..chans {
                        let plane = buf.chan(ch);
                        self.samples.push_back(plane[frame] as f32 / i32::MAX as f32);
                    }
                }
            }
            symphonia::core::audio::AudioBufferRef::U8(buf) => {
                for frame in 0..frames {
                    for ch in 0..chans {
                        let plane = buf.chan(ch);
                        let sample = (plane[frame] as i16 - 128) as f32 / 128.0;
                        self.samples.push_back(sample);
                    }
                }
            }
            symphonia::core::audio::AudioBufferRef::U24(buf) => {
                for frame in 0..frames {
                    for ch in 0..chans {
                        let plane = buf.chan(ch);
                        let u24_bytes = plane[frame].to_ne_bytes();
                        let u24_val = u32::from_ne_bytes([u24_bytes[0], u24_bytes[1], u24_bytes[2], 0]);
                        let sample = (u24_val as i32 - 0x800000) as f32 / 0x800000 as f32;
                        self.samples.push_back(sample);
                    }
                }
            }
            symphonia::core::audio::AudioBufferRef::U32(buf) => {
                for frame in 0..frames {
                    for ch in 0..chans {
                        let plane = buf.chan(ch);
                        let sample = (plane[frame] as i64 - 0x80000000i64) as f32 / 0x80000000i64 as f32;
                        self.samples.push_back(sample);
                    }
                }
            }
            _ => {
                debug!("Unsupported audio format in streaming source");
            }
        }
    }

    fn mark_finished(&mut self) {
        self.finished = true;
    }

    fn has_data(&self) -> bool {
        !self.samples.is_empty() || !self.finished
    }
}

impl Iterator for SymphoniaStreamSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.samples.pop_front()
    }
}

impl Source for SymphoniaStreamSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}


#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Connecting(String),    // Starting connection to URL
    Connected,             // Successfully connected and decoding started
    Stopped,
    Paused,
    Resumed,
    Error(String),
    BufferProgress(usize), // bytes buffered
    Metadata(String),      // ICY metadata (track titles, etc.)
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
        let _ = self.event_sender.send(PlayerEvent::Connecting(url.clone()));

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

    /// Improved streaming with Symphonia continuous decoding
    async fn fetch_and_play_stream(
        url: &str,
        stream_handle: &OutputStreamHandle,
        state: &Arc<Mutex<PlayerState>>,
        event_sender: &watch::Sender<PlayerEvent>,
        cancellation_token: &CancellationToken,
    ) -> Result<()> {
        debug!("Fetching stream from URL (symphonia): {}", url);

        // Create sink for this stream
        let new_sink = Sink::try_new(stream_handle)?;

        // Update state with the new sink
        {
            let mut state_guard = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
            state_guard.sink = Some(new_sink);
            state_guard.set_state(PlaybackState::Playing);
        }

        let _ = event_sender.send(PlayerEvent::Connected);

        // Create HTTP client with proper settings for streaming
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;

        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("HTTP error: {}", response.status()));
        }

        // Shared buffer for new data
        let (media_source, shared_buf, read_pos) = StreamingSource::new();

        // Spawn a task that keeps filling the buffer with network bytes
        {
            let shared_buf = shared_buf.clone();
            let read_pos = read_pos.clone();
            let cancellation_token = cancellation_token.clone();
            let event_sender_clone = event_sender.clone();
            tokio::spawn(async move {
                let mut stream = response.bytes_stream();
                let mut total_bytes = 0usize;
                const MAX_BUFFER_SIZE: usize = 8 * 1024 * 1024; // 8MB buffer limit
                const BACKPRESSURE_THRESHOLD: usize = 6 * 1024 * 1024; // Start backpressure at 6MB
                const CLEANUP_THRESHOLD: usize = 2 * 1024 * 1024; // Clean up after 2MB read

                while let Some(chunk_result) = stream.next().await {
                    // Tighter cancellation check with select
                    tokio::select! {
                        _ = cancellation_token.cancelled() => {
                            debug!("Network fetch cancelled");
                            break;
                        }
                        chunk_result = async { chunk_result } => {
                            if let Ok(chunk) = chunk_result {
                                total_bytes += chunk.len();

                                // Consolidated buffer management based on read position
                                loop {
                                    let (buffer_size, cleanup_needed) = {
                                        let buf = shared_buf.lock().await;
                                        let pos = read_pos.lock().unwrap();
                                        (buf.len(), *pos > CLEANUP_THRESHOLD)
                                    };

                                    if cleanup_needed {
                                        // Clean up read data to prevent unbounded growth
                                        let mut buf = shared_buf.lock().await;
                                        let mut pos = read_pos.lock().unwrap();
                                        if *pos > 0 {
                                            buf.drain(..*pos);
                                            debug!("Cleaned up {}KB of read data", *pos / 1024);
                                            *pos = 0;
                                        }
                                    } else if buffer_size > MAX_BUFFER_SIZE {
                                        // Emergency cleanup if buffer gets too large despite position tracking
                                        let mut buf = shared_buf.lock().await;
                                        let drop_size = buf.len() / 4;
                                        buf.drain(..drop_size);
                                        debug!("Emergency cleanup: dropped {}KB of old data", drop_size / 1024);

                                        // Reset read position since we dropped data
                                        let mut pos = read_pos.lock().unwrap();
                                        *pos = (*pos).saturating_sub(drop_size);
                                    } else if buffer_size > BACKPRESSURE_THRESHOLD {
                                        // Apply backpressure by waiting briefly
                                        tokio::select! {
                                            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
                                            _ = cancellation_token.cancelled() => return,
                                        }
                                    } else {
                                        break; // Buffer size is acceptable
                                    }
                                }

                                // Add new data to buffer
                                {
                                    let mut buf = shared_buf.lock().await;
                                    buf.extend_from_slice(&chunk);

                                    // Emit buffer progress periodically
                                    if total_bytes % (256 * 1024) == 0 { // Every 256KB
                                        let _ = event_sender_clone.send(PlayerEvent::BufferProgress(buf.len()));
                                    }
                                }

                                // Log progress periodically
                                if total_bytes % (512 * 1024) == 0 && total_bytes > 0 {
                                    debug!("Network fetched {} KB so far", total_bytes / 1024);
                                }
                            }
                        }
                    }
                }
                debug!("Network stream ended, total bytes: {}KB", total_bytes / 1024);
            });
        }

        // Wait for some initial data before trying to decode
        while {
            let buf = shared_buf.lock().await;
            buf.len() < 64 * 1024 // Wait for 64KB before starting
        } && !cancellation_token.is_cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        if cancellation_token.is_cancelled() {
            return Ok(());
        }

        // Attach symphonia to our streaming source
        let mss = MediaSourceStream::new(
            Box::new(media_source) as Box<dyn MediaSource>,
            MediaSourceStreamOptions::default(),
        );

        let hint = Hint::new(); // can set extension if known
        let probed = get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let mut format = probed.format;

        let track = format
            .default_track()
            .ok_or_else(|| anyhow::anyhow!("no default track"))?;

        let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

        debug!("Found audio track: codec={:?}, sample_rate={:?}, channels={:?}",
            track.codec_params.codec,
            track.codec_params.sample_rate,
            track.codec_params.channels
        );

        // Create channel for sending decoded audio samples to sink
        let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<rodio::buffer::SamplesBuffer<f32>>(16);

        // Spawn blocking task for CPU-heavy decoding
        let decode_task = {
            let cancellation_token = cancellation_token.clone();
            tokio::task::spawn_blocking(move || {
                Self::decode_blocking_task(format, decoder, audio_tx, cancellation_token)
            })
        };

        // Main async task handles sink management
        loop {
            tokio::select! {
                // Receive decoded audio from blocking task
                audio_source = audio_rx.recv() => {
                    match audio_source {
                        Some(source) => {
                            if let Ok(state_guard) = state.lock() {
                                if let Some(current_sink) = state_guard.sink.as_ref() {
                                    current_sink.append(source);
                                    current_sink.play();
                                }
                            }
                        }
                        None => {
                            debug!("Decode task ended");
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

        // Wait for decode task to complete
        let _ = decode_task.await;

        Ok(())
    }

    /// CPU-heavy blocking task for Symphonia decoding
    fn decode_blocking_task(
        mut format: Box<dyn FormatReader>,
        mut decoder: Box<dyn symphonia::core::codecs::Decoder>,
        audio_tx: tokio::sync::mpsc::Sender<rodio::buffer::SamplesBuffer<f32>>,
        cancellation_token: CancellationToken,
    ) -> Result<()> {
        let mut consecutive_errors = 0;
        let mut backoff_delay = std::time::Duration::from_millis(10);
        const MAX_CONSECUTIVE_ERRORS: u32 = 15;
        const MAX_BACKOFF_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

        loop {
            // Check for cancellation (this is a blocking task, so check periodically)
            if cancellation_token.is_cancelled() {
                debug!("Decode task cancelled");
                break;
            }

            match format.next_packet() {
                Ok(packet) => {
                    consecutive_errors = 0;
                    backoff_delay = std::time::Duration::from_millis(10);

                    match decoder.decode(&packet) {
                        Ok(audio_buf) => {
                            // Convert to rodio samples (f32 PCM)
                            let spec = *audio_buf.spec();
                            let chans = spec.channels.count();
                            let frames = audio_buf.frames();

                            let mut samples = Vec::with_capacity(frames * chans);

                            // Extract samples based on the format and interleave properly
                            match audio_buf {
                                symphonia::core::audio::AudioBufferRef::F32(buf) => {
                                    for frame in 0..frames {
                                        for ch in 0..chans {
                                            let plane = buf.chan(ch);
                                            samples.push(plane[frame]);
                                        }
                                    }
                                }
                                symphonia::core::audio::AudioBufferRef::F64(buf) => {
                                    for frame in 0..frames {
                                        for ch in 0..chans {
                                            let plane = buf.chan(ch);
                                            samples.push(plane[frame] as f32);
                                        }
                                    }
                                }
                                symphonia::core::audio::AudioBufferRef::S16(buf) => {
                                    for frame in 0..frames {
                                        for ch in 0..chans {
                                            let plane = buf.chan(ch);
                                            samples.push(plane[frame] as f32 / i16::MAX as f32);
                                        }
                                    }
                                }
                                symphonia::core::audio::AudioBufferRef::S32(buf) => {
                                    for frame in 0..frames {
                                        for ch in 0..chans {
                                            let plane = buf.chan(ch);
                                            samples.push(plane[frame] as f32 / i32::MAX as f32);
                                        }
                                    }
                                }
                                symphonia::core::audio::AudioBufferRef::U8(buf) => {
                                    for frame in 0..frames {
                                        for ch in 0..chans {
                                            let plane = buf.chan(ch);
                                            let sample = (plane[frame] as i16 - 128) as f32 / 128.0;
                                            samples.push(sample);
                                        }
                                    }
                                }
                                symphonia::core::audio::AudioBufferRef::U24(buf) => {
                                    for frame in 0..frames {
                                        for ch in 0..chans {
                                            let plane = buf.chan(ch);
                                            let u24_bytes = plane[frame].to_ne_bytes();
                                            let u24_val = u32::from_ne_bytes([u24_bytes[0], u24_bytes[1], u24_bytes[2], 0]);
                                            let sample = (u24_val as i32 - 0x800000) as f32 / 0x800000 as f32;
                                            samples.push(sample);
                                        }
                                    }
                                }
                                symphonia::core::audio::AudioBufferRef::U32(buf) => {
                                    for frame in 0..frames {
                                        for ch in 0..chans {
                                            let plane = buf.chan(ch);
                                            let sample = (plane[frame] as i64 - 0x80000000i64) as f32 / 0x80000000i64 as f32;
                                            samples.push(sample);
                                        }
                                    }
                                }
                                _ => {
                                    debug!("Unsupported audio format in packet, skipping");
                                    continue;
                                }
                            }

                            // Create rodio source and send to async task
                            let source = rodio::buffer::SamplesBuffer::new(
                                chans as u16,
                                spec.rate,
                                samples,
                            );

                            // Send to async task (non-blocking)
                            if audio_tx.try_send(source).is_err() {
                                // Channel full or closed, decoder is faster than playback
                                debug!("Audio channel full, decoder waiting");
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                        }
                        Err(symphonia::core::errors::Error::DecodeError(_)) => {
                            // Non-fatal, skip bad frame
                            continue;
                        }
                        Err(e) => {
                            debug!("Decoder error: {}", e);
                            break;
                        }
                    }
                }
                Err(symphonia::core::errors::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    consecutive_errors += 1;

                    if consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                        debug!("Too many consecutive EOF errors, applying backoff");

                        std::thread::sleep(backoff_delay);

                        // Exponential backoff with jitter
                        let jitter = std::time::Duration::from_millis((consecutive_errors % 10) as u64);
                        backoff_delay = std::cmp::min(
                            backoff_delay * 2 + jitter,
                            MAX_BACKOFF_DELAY
                        );
                        consecutive_errors = 0;
                    } else {
                        // Brief yield
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
                Err(symphonia::core::errors::Error::ResetRequired) => {
                    warn!("Decoder reset required (unsupported)");
                    break;
                }
                Err(e) => {
                    debug!("Format error: {}", e);
                    break;
                }
            }
        }

        debug!("Decode blocking task ended");
        Ok(())
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