use anyhow::Result;
use log::{debug, warn};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use futures_util::stream::StreamExt;

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

        // Create new sink
        let sink = Sink::try_new(&self.stream_handle)?;

        // Spawn a background task to fetch and play the stream
        let sink_clone = Arc::new(Mutex::new(Some(sink)));
        let url_clone = url.clone();
        let is_playing = self.is_playing.clone();
        let is_paused = self.is_paused.clone();
        let sink_ref = self.sink.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // First resolve the stream URL if it's a playlist
                let actual_url = match resolve_stream_url(&url_clone).await {
                    Ok(resolved_url) => resolved_url,
                    Err(e) => {
                        warn!("Failed to resolve stream URL: {}. Using original URL.", e);
                        url_clone.clone()
                    }
                };

                match fetch_and_play_stream(&actual_url, &sink_clone).await {
                    Ok(_) => {
                        if let Ok(mut playing) = is_playing.lock() {
                            *playing = true;
                        }
                        if let Ok(mut paused) = is_paused.lock() {
                            *paused = false;
                        }
                        // Move sink to main storage
                        if let Ok(mut sink_guard) = sink_clone.lock() {
                            if let Ok(mut main_sink) = sink_ref.lock() {
                                *main_sink = sink_guard.take();
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to fetch and play stream: {}", e);
                    }
                }
            });
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
    sink: &Arc<Mutex<Option<Sink>>>,
) -> Result<()> {
    debug!("Fetching stream from URL: {}", url);

    // Download a much larger buffer for continuous playback (5MB)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP error: {}", response.status()));
    }

    // Get a large chunk to ensure continuous playback
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();

    // Collect enough chunks for several minutes of audio
    let mut chunks_collected = 0;
    const MAX_INITIAL_CHUNKS: usize = 200; // Much larger for continuous streaming
    const MAX_BUFFER_SIZE: usize = 5 * 1024 * 1024; // 5MB

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.extend_from_slice(&chunk);
        chunks_collected += 1;

        // Break when we have enough data for continuous playback
        if chunks_collected >= MAX_INITIAL_CHUNKS || buffer.len() > MAX_BUFFER_SIZE {
            break;
        }
    }

    let buffer_len = buffer.len();
    debug!("Received {} bytes from stream for decoding", buffer_len);

    if buffer.is_empty() {
        return Err(anyhow::anyhow!("No data received from stream"));
    }

    // Create a decoder from the large buffer
    let cursor = Cursor::new(buffer);

    match Decoder::new(cursor) {
        Ok(source) => {
            debug!("Successfully decoded audio source");
            // Add source to sink and play
            if let Ok(sink_guard) = sink.lock() {
                if let Some(sink) = sink_guard.as_ref() {
                    sink.append(source);
                    sink.play();
                    debug!("Audio source added to sink and playing");
                }
            }
        }
        Err(e) => {
            warn!("Failed to decode audio stream: {} (received {} bytes)", e, buffer_len);
            return Err(e.into());
        }
    }

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