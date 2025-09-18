# Lesson 5: Real-World Application Architecture

## Learning Objectives
By the end of this lesson, you'll understand:
- How to design maintainable application architectures
- Resource management and graceful shutdown patterns
- Performance optimization strategies
- Testing approaches for async Rust applications
- Production deployment considerations

## 1. Overall Architecture Analysis

### High-Level Architecture

The SomaFM CLI demonstrates a layered architecture that separates concerns effectively:

```
┌─────────────────────────────────────────┐
│                main.rs                  │  ← Entry point, coordination
│  • Terminal setup/cleanup              │
│  • Panic handling                      │
│  • Main event loop                     │
└─────────────────────────────────────────┘
           ↓ coordinates ↓
┌─────────────────────────────────────────┐
│               app.rs                    │  ← Business logic controller
│  • User input handling                 │
│  • State management                    │
│  • Request coordination                │
└─────────────────────────────────────────┘
      ↓ uses ↓                    ↓ uses ↓
┌──────────────┐            ┌─────────────────┐
│    ui.rs     │            │   actions.rs    │  ← Message contracts
│ • Rendering  │            │ • Request types │
│ • Layout     │            │ • Response types│
│ • Styling    │            └─────────────────┘
└──────────────┘                     ↓ used by ↓
      ↓ uses ↓                 ┌─────────────────┐
┌──────────────┐              │     api.rs      │  ← External integration
│   audio.rs   │              │ • HTTP client   │
│ • Playback   │              │ • Serialization │
│ • Streaming  │              │ • URL generation│
└──────────────┘              └─────────────────┘
```

### Key Architectural Patterns

**1. Separation of Concerns**
- `main.rs`: System-level concerns (terminal, panic handling)
- `app.rs`: Business logic and state management
- `ui.rs`: Presentation layer
- `api.rs`: External integration
- `audio.rs`: Audio subsystem

**2. Message-Driven Communication**
- Async operations communicate via typed messages
- No direct coupling between UI and network layer
- Worker pattern isolates blocking operations

**3. Resource Ownership**
- Each module owns its specific resources
- Clear responsibility boundaries
- Graceful resource cleanup

## 2. Resource Management Patterns

### Terminal Resource Management

```rust
// From src/main.rs:29-35 - Panic-safe resource cleanup
let original_hook = std::panic::take_hook();
std::panic::set_hook(Box::new(move |panic| {
    let _ = disable_raw_mode();                    // 🔑 Always restore terminal
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    original_hook(panic);
}));

// From src/main.rs:61-64 - Normal cleanup
disable_raw_mode()?;
execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
terminal.show_cursor()?;
```

**Key Pattern**: Set up cleanup handlers early, ensure cleanup happens in all exit scenarios.

### Audio Resource Management

```rust
// Updated audio.rs architecture - Consolidated state management
pub struct SimpleAudioPlayer {
    state: Arc<Mutex<PlayerState>>,                // 🔑 Single mutex for all state
    _stream: OutputStream,                         // 🔑 Keep alive with naming
    stream_handle: OutputStreamHandle,
    event_sender: watch::Sender<PlayerEvent>,      // 🔑 Event broadcasting
    event_receiver: watch::Receiver<PlayerEvent>,
}

// Consolidated player state to avoid multiple mutex locks
struct PlayerState {
    current_url: Option<String>,
    playback_state: PlaybackState,
    sink: Option<Sink>,
    cancellation_token: Option<CancellationToken>, // 🔑 Task cancellation
    auto_reconnect: bool,
    reconnect_attempts: u32,
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Started(String),
    Stopped,
    Paused,
    Resumed,
    Error(String),
    StreamConnected,
    BufferProgress(usize),                         // 🔑 Memory monitoring
}
```

**Key Improvements**:
- **Single Mutex**: Eliminates deadlock risks from multiple `Arc<Mutex<_>>` fields
- **Event System**: `tokio::sync::watch` for broadcasting state changes without lock contention
- **Task Cancellation**: `CancellationToken` for proper cleanup of streaming tasks
- **State Consolidation**: All related state in one structure with clear ownership

### Graceful Shutdown

```rust
// From src/ui.rs:95-99
pub fn quit(&mut self) {
    let _ = self.audio_player.stop();              // 🔑 Stop audio first
    self.should_quit = true;                       // 🔑 Signal shutdown
    self.currently_playing_station_id = None;     // 🔑 Clear state
}
```

**Insight**: Graceful shutdown is a multi-step process: stop resources, set flags, clear state.

## 3. Performance Optimization Patterns

### UI Rendering Optimization

```rust
// From src/ui.rs:39-42 - Caching rendered content
station_items_cache: Option<Vec<String>>,
last_area_width: u16,

// Cache invalidation strategy
pub fn invalidate_station_cache(&mut self) {
    self.station_items_cache = None;              // 🔑 Invalidate when data changes
}
```

**Pattern**: Cache expensive computations, invalidate when underlying data changes.

### Non-Blocking Operations

```rust
// From src/main.rs:99-112 - Responsive input handling
if event::poll(Duration::from_millis(50))? {     // 🔑 Short timeout for responsiveness
    match event::read() {
        Ok(Event::Key(key)) => {
            if app_controller.handle_key_event(key.code).await? {
                break;
            }
        }
        Ok(Event::Resize(_, _)) => {
            // Terminal was resized, UI will automatically adjust
        }
        Ok(_) => {}     // Ignore other events
        Err(_) => {}    // Ignore read errors
    }
}
```

**Key Insight**: 50ms polling keeps the UI responsive (~20 FPS) without excessive CPU usage.

### Memory Management

```rust
// From src/audio.rs:8-17 - Memory usage monitoring
fn log_memory_usage(buffer_size: usize, operation: &str) {
    let kb = buffer_size as f64 / 1024.0;
    let mb = kb / 1024.0;
    if mb >= 1.0 {
        debug!("{}: Using {:.1} MB ({:.0} KB) of memory", operation, mb, kb);
    } else {
        debug!("{}: Using {:.0} KB of memory", operation, kb);
    }
}
```

**Pattern**: Monitor resource usage in debug builds to catch memory issues early.

### Modern Streaming Architecture

The audio system demonstrates a production-ready streaming architecture with separation of concerns and proper resource management:

```rust
// Improved streaming with bounded buffering and backpressure handling
async fn fetch_and_play_stream(
    url: &str,
    stream_handle: &OutputStreamHandle,
    state: &Arc<Mutex<PlayerState>>,
    event_sender: &watch::Sender<PlayerEvent>,
    cancellation_token: &CancellationToken,
) -> Result<()> {
    // Create bounded channel for audio chunks
    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(32); // 🔑 Bounded buffer

    // Spawn separate network fetching task
    let fetch_task = tokio::spawn(async move {
        Self::fetch_network_stream(url, audio_tx, cancellation_token, event_sender).await
    });

    // Main decoding loop with smart buffering and throttling
    let mut decode_buffer = VecDeque::new();
    const MIN_DECODE_SIZE: usize = 128 * 1024; // 🔑 Increased to 128KB for better decode success
    const MAX_BUFFER_SIZE: usize = 512 * 1024; // 🔑 Prevent unbounded growth

    loop {
        tokio::select! {
            chunk_opt = audio_rx.recv() => {
                match chunk_opt {
                    Some(chunk) => {
                        decode_buffer.extend(chunk);
                        
                        // Smart memory management with decode throttling
                        if decode_buffer.len() >= MIN_DECODE_SIZE {
                            // Decode throttling: Only attempt decode every few chunks to reduce CPU overhead
                            static mut DECODE_COUNTER: u32 = 0;
                            unsafe {
                                DECODE_COUNTER += 1;
                                if DECODE_COUNTER % 5 == 0 { // 🔑 Decode every 5th chunk
                                    Self::try_decode_and_play(&mut decode_buffer, state).await?;
                                }
                            }
                        }
                        
                        // Prevent memory leaks
                        if decode_buffer.len() > MAX_BUFFER_SIZE {
                            let drop_size = decode_buffer.len() / 4;
                            decode_buffer.drain(..drop_size); // 🔑 Drop old data
                        }
                    }
                    None => break, // Network stream ended
                }
            }
            _ = cancellation_token.cancelled() => {
                break; // 🔑 Proper cancellation
            }
        }
    }
}

// Network layer with backpressure handling
async fn fetch_network_stream(
    url: String,
    audio_tx: mpsc::Sender<Vec<u8>>,
    cancellation_token: CancellationToken,
) -> Result<()> {
    let mut stream = response.bytes_stream();
    
    while let Some(chunk_result) = stream.next().await {
        if cancellation_token.is_cancelled() { break; }
        
        let chunk = chunk_result?;
        
        // Apply backpressure when channel is full
        match audio_tx.try_send(chunk.to_vec()) {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Channel full - apply backpressure
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                    _ = cancellation_token.cancelled() => break,
                }
                audio_tx.send(chunk.to_vec()).await?; // 🔑 Retry after wait
            }
            Err(_) => break, // Channel closed
        }
    }
}
```

**Modern Architecture Benefits**:
1. **Separation of Concerns**: Network fetching and audio decoding in separate tasks
2. **Bounded Buffering**: `mpsc::channel` with fixed capacity prevents memory leaks
3. **Backpressure**: Network automatically slows when decoder can't keep up
4. **Proper Cancellation**: `CancellationToken` allows clean task termination
5. **Smart Memory Management**: Accumulate data before decode attempts, drop old data when needed
6. **Decode Throttling**: Process every 5th chunk to reduce CPU overhead while maintaining audio quality
7. **Event Broadcasting**: `watch::Sender` allows UI to react to streaming events

**Why This Architecture**: Traditional approaches either block on network or decode failures. This design provides continuous playback with predictable memory usage, optimized CPU utilization, and proper error boundaries.

## 4. Error Handling Architecture

### Layered Error Handling

```rust
// Error boundaries at each layer:

// 1. Network layer (api.rs) - wraps HTTP/parsing errors
pub async fn get_stations(&self) -> Result<Vec<Station>> {
    let response = self.client.get("...").send().await?;    // HTTP errors
    let channels: ChannelsResponse = response.json().await?; // Parse errors
    Ok(channels.channels)
}

// 2. Business logic (app.rs) - handles domain errors
pub async fn process_response(&mut self, resp: Response) -> Result<()> {
    match resp {
        Response::StationsLoaded(res) => match res {
            Ok(stations) => { /* success path */ }
            Err(_e) => {
                self.ui_app.is_fetching_stations = false;
                // TODO: surface error in UI                // 🔑 Current gap!
            }
        }
    }
}

// 3. UI layer (main.rs) - graceful degradation
let res = run_app(&mut terminal, &mut app_controller, req_tx, resp_rx).await;
let _ = res;  // Currently ignores all errors                // 🔑 Could be improved!
```

**Current Architecture Gap**: Errors are contained but not always surfaced to users.

### Error Recovery Strategies

```rust
// From src/api.rs:70 - Graceful parsing fallback
Err(_) => Ok(vec![]), // Default to empty vec on any error

// From src/app.rs:149-152 - Channel backpressure handling
if self.req_tx.try_send(Request::LoadTrackForStation {
    station_id: station_id.clone()
}).is_ok() {
    self.last_track_req.insert(station_id, now);
} else {
    self.ui_app.is_fetching_track = false;        // 🔑 Reset state on failure
}
```

**Pattern**: Always have a fallback plan. Reset UI state if operations fail.

### Automatic Retry and Resilience Patterns

The audio system demonstrates production-grade retry logic for handling network instability:

```rust
// Retry logic with exponential backoff and cancellation
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
        // Check retry conditions
        let should_retry = {
            let state_guard = state.lock()?;
            state_guard.auto_reconnect && state_guard.reconnect_attempts < MAX_RETRY_ATTEMPTS
        };

        if !should_retry || cancellation_token.is_cancelled() {
            break;
        }

        // Attempt streaming
        match Self::fetch_and_play_stream(&url, &stream_handle, &state, &event_sender, &cancellation_token).await {
            Ok(_) => break, // Success - exit retry loop
            Err(e) => {
                // Increment attempts and check limits
                {
                    let mut state_guard = state.lock()?;
                    state_guard.reconnect_attempts += 1;
                    
                    if state_guard.reconnect_attempts >= MAX_RETRY_ATTEMPTS {
                        state_guard.set_state(PlaybackState::Error(format!("Max retries: {}", e)));
                        let _ = event_sender.send(PlayerEvent::Error(format!("Max retries: {}", e)));
                        break;
                    }
                }

                // Send error event and wait before retry
                let _ = event_sender.send(PlayerEvent::Error(format!("Retrying... ({})", e)));
                
                // Cancellable delay
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)) => {},
                    _ = cancellation_token.cancelled() => break,
                }
            }
        }
    }
}
```

**Resilience Patterns**:
1. **Bounded Retries**: Prevent infinite retry loops with maximum attempt limits
2. **Cancellable Delays**: Respect user cancellation even during retry waits
3. **State Tracking**: Maintain retry count per connection attempt
4. **User Feedback**: Inform users about retry attempts and final failures
5. **Graceful Degradation**: Switch to error state after exhausting retries

**Real-World Benefits**: Radio streams often have temporary connectivity issues. This pattern ensures playback resumes automatically without user intervention while preventing resource waste on permanently failed streams.

## 5. Testing Strategies

### Unit Testing Approach

```rust
// Example test structure for the API client
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_station_parsing() {
        let json_data = r#"
        {
            "channels": [
                {
                    "id": "groovesalad",
                    "title": "Groove Salad",
                    "listeners": "142",
                    "genre": "Electronic"
                }
            ]
        }"#;

        let response: ChannelsResponse = serde_json::from_str(json_data).unwrap();
        assert_eq!(response.channels.len(), 1);
        assert_eq!(response.channels[0].listeners, 142);
        assert_eq!(response.channels[0].genre, vec!["Electronic"]);
    }

    #[test]
    fn test_custom_deserializers() {
        // Test string-to-number conversion
        let json = r#"{"listeners": "123"}"#;
        let station: Station = serde_json::from_str(json).unwrap();
        assert_eq!(station.listeners, 123);

        // Test number-to-number conversion
        let json = r#"{"listeners": 456}"#;
        let station: Station = serde_json::from_str(json).unwrap();
        assert_eq!(station.listeners, 456);
    }
}
```

### Integration Testing Patterns

```rust
// Test the message passing system
#[tokio::test]
async fn test_worker_communication() {
    let (req_tx, req_rx) = mpsc::channel(1);
    let (resp_tx, resp_rx) = mpsc::channel(1);

    // Spawn worker
    tokio::spawn(worker_loop(req_rx, resp_tx));

    // Send request
    req_tx.send(Request::LoadStations).await.unwrap();

    // Receive response
    let response = resp_rx.recv().await.unwrap();
    match response {
        Response::StationsLoaded(Ok(stations)) => {
            assert!(!stations.is_empty());
        }
        _ => panic!("Unexpected response"),
    }
}
```

### Property-Based Testing

```rust
// Using proptest for robust testing
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_station_selection_bounds(
        station_count in 0usize..100,
        selection_index in 0usize..200
    ) {
        let mut ui_state = UIState::new(audio_player);
        ui_state.stations = (0..station_count)
            .map(|i| create_test_station(i))
            .collect();

        // Should never panic regardless of input
        ui_state.select_station(selection_index);

        if station_count > 0 {
            assert!(ui_state.current_station_index < station_count);
        }
    }
}
```

## 6. Production Deployment Considerations

### Binary Optimization

```toml
# Add to Cargo.toml for release builds
[profile.release]
lto = true              # Link-time optimization
codegen-units = 1       # Better optimization
panic = "abort"         # Smaller binary size
strip = true            # Remove debug symbols
```

### Logging Strategy

```rust
// From src/main.rs:27
env_logger::init();

// Throughout the codebase
use log::{debug, warn, error};

debug!("Fetched {} tracks for station {}", tracks.len(), station_id);
warn!("Failed to send request: channel full");
error!("Critical system error: {}", e);
```

**Best Practice**: Use appropriate log levels. Debug for development, warn for recoverable issues, error for serious problems.

### Configuration Management

```rust
// Recommended: Add configuration support
#[derive(Debug, Deserialize)]
pub struct Config {
    pub api_base_url: String,
    pub audio_buffer_size: usize,
    pub refresh_interval_seconds: u64,
    pub log_level: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        // Try: environment variables, config file, defaults
        let config = config::Config::builder()
            .add_source(config::Environment::with_prefix("SOMAFM"))
            .add_source(config::File::with_name("somafm.toml").required(false))
            .set_default("api_base_url", "https://somafm.com")?
            .set_default("audio_buffer_size", 8192)?
            .build()?;

        config.try_deserialize()
    }
}
```

## 7. Exercises

### Exercise 1: Add Comprehensive Error Display
**Current Issue**: Errors are logged but not shown to users.

**Task**: Implement a complete error handling system:

```rust
// Add to ui.rs
#[derive(Debug, Clone)]
pub enum UserError {
    NetworkError { message: String, retry_available: bool },
    AudioError { message: String },
    DataError { message: String },
}

pub struct ErrorState {
    current_error: Option<UserError>,
    error_history: Vec<(UserError, Instant)>,
    auto_dismiss_timeout: Duration,
}

impl UIState {
    pub fn show_error(&mut self, error: UserError) {
        // TODO: Add error to state, determine display strategy
    }

    pub fn dismiss_current_error(&mut self) {
        // TODO: Clear current error, maybe show next in queue
    }

    pub fn update_error_display(&mut self) {
        // TODO: Handle auto-dismissal, error rotation
    }
}

// Update UI rendering to show errors
pub fn render_ui(frame: &mut Frame, ui_app: &mut UIState) {
    // TODO: Add error display area
    // Consider: Toast notifications, status bar, modal overlay
}
```

### Exercise 2: Add Performance Monitoring
**Goal**: Monitor and optimize application performance.

**Task**: Add comprehensive performance monitoring:

```rust
use std::time::{Duration, Instant};
use std::collections::VecDeque;

#[derive(Debug)]
pub struct PerformanceMonitor {
    frame_times: VecDeque<Duration>,
    api_call_times: HashMap<String, VecDeque<Duration>>,
    memory_samples: VecDeque<usize>,
    max_samples: usize,
}

impl PerformanceMonitor {
    pub fn new() -> Self {
        Self {
            frame_times: VecDeque::new(),
            api_call_times: HashMap::new(),
            memory_samples: VecDeque::new(),
            max_samples: 100,
        }
    }

    pub fn record_frame_time(&mut self, duration: Duration) {
        // TODO: Record frame rendering time
        // Calculate rolling average FPS
        // Warn if FPS drops below threshold
    }

    pub fn record_api_call(&mut self, endpoint: &str, duration: Duration) {
        // TODO: Track API performance by endpoint
        // Detect slow endpoints
        // Calculate percentiles
    }

    pub fn get_performance_stats(&self) -> PerformanceStats {
        // TODO: Return current performance metrics
    }
}

// Integration into main loop
let mut perf_monitor = PerformanceMonitor::new();

loop {
    let frame_start = Instant::now();

    // Render UI
    if let Err(_e) = terminal.draw(|f| ui::render_ui(f, &mut app_controller.ui_app)) {
        break;
    }

    perf_monitor.record_frame_time(frame_start.elapsed());

    // Show performance stats in debug mode
    if log_enabled!(log::Level::Debug) {
        let stats = perf_monitor.get_performance_stats();
        if stats.avg_fps < 10.0 {
            warn!("Low FPS detected: {:.1}", stats.avg_fps);
        }
    }
}
```

### Exercise 3: Add Configuration System
**Current Issue**: All settings are hardcoded.

**Task**: Add a flexible configuration system:

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub audio: AudioConfig,
    pub network: NetworkConfig,
    pub keybindings: KeybindingConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UiConfig {
    pub refresh_rate_fps: u16,
    pub show_debug_info: bool,
    pub color_scheme: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AudioConfig {
    pub buffer_size: usize,
    pub preferred_quality: String,
    pub volume: f32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct KeybindingConfig {
    pub quit: Vec<String>,
    pub play_pause: Vec<String>,
    pub next_station: Vec<String>,
    pub previous_station: Vec<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        // TODO: Implement configuration loading priority:
        // 1. Command line arguments
        // 2. Environment variables
        // 3. User config file (~/.config/somafm/config.toml)
        // 4. System config file (/etc/somafm/config.toml)
        // 5. Built-in defaults
    }

    pub fn save(&self) -> Result<()> {
        // TODO: Save to user config file
    }

    pub fn get_config_path() -> Result<PathBuf> {
        // TODO: Return appropriate config file path for the platform
    }
}

// Integration with existing code
impl AppController {
    pub fn new(audio_player: SimpleAudioPlayer, req_tx: mpsc::Sender<Request>, config: AppConfig) -> Self {
        // TODO: Use config throughout the application
    }
}
```

### Exercise 4: Add Plugin System
**Goal**: Make the application extensible.

**Task**: Design a plugin architecture:

```rust
// Plugin trait
pub trait SomaFMPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;

    // Lifecycle hooks
    fn on_startup(&mut self, ctx: &mut PluginContext) -> Result<()> { Ok(()) }
    fn on_shutdown(&mut self, ctx: &mut PluginContext) -> Result<()> { Ok(()) }

    // Event hooks
    fn on_station_change(&mut self, ctx: &mut PluginContext, station: &Station) -> Result<()> { Ok(()) }
    fn on_track_change(&mut self, ctx: &mut PluginContext, track: &Track) -> Result<()> { Ok(()) }
    fn on_playback_start(&mut self, ctx: &mut PluginContext) -> Result<()> { Ok(()) }
    fn on_playback_stop(&mut self, ctx: &mut PluginContext) -> Result<()> { Ok(()) }

    // Custom UI components
    fn render_status_widget(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {}
    fn handle_key_event(&mut self, key: KeyCode, ctx: &mut PluginContext) -> Result<bool> { Ok(false) }
}

pub struct PluginContext {
    pub current_station: Option<Station>,
    pub current_track: Option<Track>,
    pub is_playing: bool,
    // TODO: Add methods for plugins to interact with the application
}

pub struct PluginManager {
    plugins: Vec<Box<dyn SomaFMPlugin>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self { plugins: Vec::new() }
    }

    pub fn register_plugin(&mut self, plugin: Box<dyn SomaFMPlugin>) {
        self.plugins.push(plugin);
    }

    pub fn notify_station_change(&mut self, ctx: &mut PluginContext, station: &Station) {
        for plugin in &mut self.plugins {
            if let Err(e) = plugin.on_station_change(ctx, station) {
                error!("Plugin '{}' failed on station change: {}", plugin.name(), e);
            }
        }
    }
}

// Example plugin: Last.fm scrobbler
pub struct LastFmPlugin {
    api_key: String,
    session_key: Option<String>,
}

impl SomaFMPlugin for LastFmPlugin {
    fn name(&self) -> &str { "Last.fm Scrobbler" }
    fn version(&self) -> &str { "1.0.0" }

    fn on_track_change(&mut self, ctx: &mut PluginContext, track: &Track) -> Result<()> {
        if let Some(session_key) = &self.session_key {
            // TODO: Scrobble track to Last.fm
        }
        Ok(())
    }
}
```

### Exercise 5: Advanced Streaming Features
**Goal**: Build upon the improved streaming architecture to add advanced features.

**Background**: The streaming implementation now includes cancellation, backpressure, and retry logic. This exercise extends it further.

**Task 1**: Add adaptive quality control:

```rust
// From src/audio.rs - Quality monitoring
#[derive(Debug, Clone)]
pub struct StreamingMetrics {
    pub bytes_per_second: f64,
    pub decode_success_rate: f64,
    pub buffer_underruns: u32,
    pub average_latency: Duration,
}

impl SimpleAudioPlayer {
    pub fn get_streaming_metrics(&self) -> Option<StreamingMetrics> {
        // Track and return current performance metrics
    }
    
    async fn adaptive_decode(&mut self, buffer: &mut VecDeque<u8>) -> Result<()> {
        let metrics = self.calculate_recent_performance();
        
        // Adjust decode strategy based on performance
        let min_size = if metrics.decode_success_rate > 0.9 {
            32 * 1024  // Aggressive - small chunks for low latency
        } else {
            128 * 1024 // Conservative - larger chunks for stability
        };
        
        if buffer.len() >= min_size {
            self.try_decode_and_play(buffer).await
        } else {
            Ok(()) // Wait for more data
        }
    }
}
```

**Task 2**: Add ICY metadata parsing:

```rust
// Parse radio station metadata (song titles, etc.)
#[derive(Debug, Clone)]
pub struct IcyMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub station: Option<String>,
    pub bitrate: Option<u32>,
}

// Extend PlayerEvent for metadata
pub enum PlayerEvent {
    // ... existing events
    MetadataUpdate(IcyMetadata),
    QualityChange { bitrate: u32, sample_rate: u32 },
}
```

**Task 3**: Add network health monitoring:

```rust
pub struct NetworkHealth {
    connection_stability: f32,  // 0.0 - 1.0
    bandwidth_estimate: u32,    // bytes/sec
    jitter: Duration,           // connection consistency
}

impl SimpleAudioPlayer {
    async fn monitor_network_health(&self) -> NetworkHealth {
        // Implement network quality assessment
    }
    
    fn should_retry_with_quality(&self, error: &Error, health: &NetworkHealth) -> bool {
        // Smart retry decisions based on network conditions
        match error.kind() {
            ErrorKind::Timeout if health.connection_stability < 0.5 => false,
            ErrorKind::ConnectionReset if health.jitter > Duration::from_secs(5) => false,
            _ => true
        }
    }
}
                
                // Exponential backoff
                tokio::time::sleep(Duration::from_millis(
                    100 * 2u64.pow(consecutive_errors)
                )).await;
            }
        }
    }
    
    Ok(())
}

fn adapt_chunk_size(current: usize, quality: f32, config: &StreamingConfig) -> usize {
    // TODO: Implement adaptive sizing logic
    // - Increase chunk size for stable connections
    // - Decrease for poor network conditions
    // - Respect min/max bounds
    current
}
```

**Additional Challenges**:
1. Add network quality measurement based on download speeds
2. Implement codec-specific optimization (MP3 vs AAC frame boundaries)
3. Add prefetching for seamless station switching
4. Create metrics collection for streaming performance analysis

## Architecture Principles Summary

### 1. Separation of Concerns
```rust
// ✅ Clear responsibilities
main.rs    → System coordination
app.rs     → Business logic
ui.rs      → Presentation
api.rs     → External integration
audio.rs   → Audio subsystem

// ❌ Mixed concerns
main.rs    → Everything in one file
```

### 2. Resource Ownership
```rust
// ✅ Clear ownership
struct AudioPlayer {
    _stream: OutputStream,  // Owns the stream
    sink: Arc<Mutex<Sink>>, // Shared sink access
}

// ❌ Unclear ownership
fn play_audio(stream: &OutputStream, data: &[u8]) // Who owns what?
```

### 3. Error Boundaries
```rust
// ✅ Errors contained at layer boundaries
Worker: Result<Data, Error> → Response::DataLoaded(Result<Data, Error>)
UI: Process Response → Show error or update state

// ❌ Errors propagate everywhere
fn ui_function() -> Result<(), NetworkError> // UI shouldn't know about network errors
```

### 4. Message-Driven Communication
```rust
// ✅ Typed messages
enum Request { LoadData { id: String } }
enum Response { DataLoaded(Result<Data, Error>) }

// ❌ Direct coupling
ui.update_data(api.fetch_data().await?)  // UI blocked by network
```

## Key Takeaways

1. **Layered Architecture**: Separate system, business, presentation, and integration concerns
2. **Resource Management**: Own resources clearly, clean up gracefully, handle panics
3. **Performance**: Cache expensive operations, use appropriate polling intervals, monitor bottlenecks
4. **Error Handling**: Contain errors at appropriate boundaries, provide graceful degradation
5. **Testing**: Unit test pure functions, integration test message flows, property test edge cases
6. **Configuration**: Make settings configurable, support multiple configuration sources
7. **Extensibility**: Design for future enhancement with plugins or configuration

## Conclusion: Evolution of Production-Ready Code

This curriculum demonstrates how real-world Rust applications evolve from working prototypes to production-ready systems. The SomaFM CLI, particularly its audio player architecture, showcases this progression:

### Initial Implementation (Working Prototype)
- Multiple `Arc<Mutex<_>>` for state (deadlock-prone)
- Basic streaming with chunk-by-chunk decoding
- No task cancellation or retry logic
- Simple error propagation

### Production Implementation (Current)
- Single `PlayerState` mutex (deadlock-free)
- Bounded channels with backpressure handling
- `CancellationToken` for clean task termination
- Automatic retry with exponential backoff
- Event system for reactive state updates
- Memory monitoring and bounded buffers

### Key Architectural Evolution

1. **State Management**: From multiple mutexes to consolidated state reduces deadlock risks
2. **Concurrency**: From basic async to advanced cancellation and backpressure patterns
3. **Error Handling**: From simple propagation to retry logic and graceful degradation
4. **Resource Management**: From unbounded growth to careful memory and task management
5. **Observability**: From basic logging to event-driven monitoring and metrics

### Production-Ready Qualities

The SomaFM CLI now demonstrates patterns suitable for production use:
- **Responsive**: UI never blocks on network operations (message passing)
- **Robust**: Graceful error handling and automatic recovery (retry logic)
- **Maintainable**: Clear separation of concerns and module boundaries
- **Performant**: Efficient resource usage and smart caching
- **Observable**: Event system enables monitoring and debugging
- **Safe**: Proper resource cleanup prevents memory leaks and deadlocks

### Rust-Specific Benefits

Rust's type system and ownership model enforce these patterns:
- **Compile-time safety**: Prevents data races and memory leaks
- **Zero-cost abstractions**: High-level patterns with no runtime overhead
- **Fearless concurrency**: Complex async patterns without traditional pitfalls
- **Error handling**: `Result<T, E>` forces explicit error consideration

## Next Steps for Real-World Deployment

- **Metrics Collection**: Add prometheus/metrics integration
- **Configuration Management**: Environment-based configuration
- **Structured Logging**: Correlation IDs and log aggregation
- **Performance Profiling**: CPU and memory profiling in production
- **Circuit Breakers**: Prevent cascade failures from bad streams
- **Health Checks**: Application health endpoints for monitoring

These patterns apply beyond audio streaming to any production Rust application requiring robust concurrent processing, error handling, and resource management. The techniques demonstrated here scale from CLI tools to web services, desktop applications, and embedded systems.