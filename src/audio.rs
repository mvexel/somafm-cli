use anyhow::Result;
use log::{debug, warn};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use futures_util::stream::StreamExt;

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

pub struct SimpleAudioPlayer {
    sink: Arc<Mutex<Option<Sink>>>,
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    current_url: Arc<Mutex<Option<String>>>,
    is_playing: Arc<Mutex<bool>>,
    is_paused: Arc<Mutex<bool>>,
}

impl SimpleAudioPlayer {
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) = OutputStream::try_default()?;

        Ok(Self {
            sink: Arc::new(Mutex::new(None)),
            _stream: stream,
            stream_handle,
            current_url: Arc::new(Mutex::new(None)),
            is_playing: Arc::new(Mutex::new(false)),
            is_paused: Arc::new(Mutex::new(false)),
        })
    }

    pub fn play(&self, url: String) -> Result<()> {
        debug!("Playing audio from URL: {}", url);

        // Stop any current playback
        self.stop()?;

        // Spawn a background task to fetch and play the stream
        let url_clone = url.clone();
        let is_playing = self.is_playing.clone();
        let is_paused = self.is_paused.clone();
        let sink_ref = self.sink.clone();
        let stream_handle = self.stream_handle.clone();

        // Use tokio::spawn instead of creating a new runtime
        tokio::spawn(async move {
            // First resolve the stream URL if it's a playlist
            let actual_url = match resolve_stream_url(&url_clone).await {
                Ok(resolved_url) => resolved_url,
                Err(e) => {
                    warn!("Failed to resolve stream URL: {}. Using original URL.", e);
                    url_clone.clone()
                }
            };

            match fetch_and_play_stream(&actual_url, &stream_handle, &sink_ref).await {
                Ok(_) => {
                    if let Ok(mut playing) = is_playing.lock() {
                        *playing = true;
                    }
                    if let Ok(mut paused) = is_paused.lock() {
                        *paused = false;
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch and play stream: {}", e);
                }
            }
        });

        // Update state immediately
        if let Ok(mut url_guard) = self.current_url.lock() {
            *url_guard = Some(url);
        }

        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        if let Ok(sink_guard) = self.sink.lock() {
            if let Some(sink) = sink_guard.as_ref() {
                sink.pause();
                if let Ok(mut paused) = self.is_paused.lock() {
                    *paused = true;
                }
                if let Ok(mut playing) = self.is_playing.lock() {
                    *playing = false;
                }
                debug!("Audio paused");
            }
        }
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        if let Ok(sink_guard) = self.sink.lock() {
            if let Some(sink) = sink_guard.as_ref() {
                sink.play();
                if let Ok(mut paused) = self.is_paused.lock() {
                    *paused = false;
                }
                if let Ok(mut playing) = self.is_playing.lock() {
                    *playing = true;
                }
                debug!("Audio resumed");
            }
        }
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        if let Ok(mut sink_guard) = self.sink.lock() {
            if let Some(sink) = sink_guard.take() {
                sink.stop();
            }
        }
        if let Ok(mut url_guard) = self.current_url.lock() {
            *url_guard = None;
        }
        if let Ok(mut playing) = self.is_playing.lock() {
            *playing = false;
        }
        if let Ok(mut paused) = self.is_paused.lock() {
            *paused = false;
        }
        debug!("Audio stopped");
        Ok(())
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.lock().map(|guard| *guard).unwrap_or(false)
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.lock().map(|guard| *guard).unwrap_or(false)
    }
}

async fn fetch_and_play_stream(
    url: &str,
    stream_handle: &OutputStreamHandle,
    sink: &Arc<Mutex<Option<Sink>>>,
) -> Result<()> {
    debug!("Fetching stream from URL: {}", url);

    // Create sink for this stream
    let new_sink = Sink::try_new(stream_handle)?;

    // Store the sink in the shared storage first
    if let Ok(mut sink_guard) = sink.lock() {
        *sink_guard = Some(new_sink);
    }

    // Create HTTP client with proper settings for streaming
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP error: {}", response.status()));
    }

    debug!("Connected to stream, starting continuous playback");
    
    // Get the sink reference for continuous streaming
    let sink_clone = sink.clone();
    
    // Create a streaming source that reads directly from the HTTP stream
    let mut stream = response.bytes_stream();
    let mut total_bytes = 0usize;
    
    // Buffer for accumulating chunks before decoding
    let mut decode_buffer = Vec::new();
    const DECODE_CHUNK_SIZE: usize = 32 * 1024; // 32KB chunks for decoding
    const MAX_DECODE_BUFFER: usize = 256 * 1024; // 256KB max buffer
    
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        total_bytes += chunk.len();
        decode_buffer.extend_from_slice(&chunk);
        
        // When we have enough data, try to decode and play it
        if decode_buffer.len() >= DECODE_CHUNK_SIZE {
            // Check if sink is still available and get a reference to it
            let sink_available = {
                if let Ok(sink_guard) = sink_clone.lock() {
                    sink_guard.is_some()
                } else {
                    false
                }
            };
            
            if sink_available {
                // Only decode if sink is still alive
                if decode_buffer.len() >= MAX_DECODE_BUFFER || total_bytes < DECODE_CHUNK_SIZE * 2 {
                    // Try to decode this chunk
                    let cursor = Cursor::new(decode_buffer.clone());
                    
                    match Decoder::new(cursor) {
                        Ok(source) => {
                            debug!("Decoded {} bytes, adding to sink (total: {} KB)", 
                                   decode_buffer.len(), total_bytes / 1024);
                            
                            // Add to sink safely
                            if let Ok(sink_guard) = sink_clone.lock() {
                                if let Some(current_sink) = sink_guard.as_ref() {
                                    current_sink.append(source);
                                    
                                    // Start playback if this is the first chunk
                                    if total_bytes < DECODE_CHUNK_SIZE * 3 {
                                        current_sink.play();
                                        debug!("Started audio playback");
                                    }
                                }
                            }
                            
                            // Clear the buffer after successful decode
                            decode_buffer.clear();
                        }
                        Err(_) => {
                            // If decode fails, it might be incomplete data
                            // Keep accumulating unless buffer is too large
                            if decode_buffer.len() >= MAX_DECODE_BUFFER {
                                warn!("Decode buffer too large ({} bytes), discarding and continuing", 
                                      decode_buffer.len());
                                decode_buffer.clear();
                            }
                        }
                    }
                }
            } else {
                // Sink was dropped, stop streaming
                debug!("Sink was dropped, stopping stream");
                break;
            }
        }
        
        // Log progress periodically
        if total_bytes % (512 * 1024) == 0 && total_bytes > 0 {
            debug!("Streamed {} KB so far", total_bytes / 1024);
        }
    }

    debug!("Stream ended, total bytes received: {} KB", total_bytes / 1024);
    Ok(())
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