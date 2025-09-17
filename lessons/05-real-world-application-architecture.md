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
// From src/audio.rs:19-26
pub struct SimpleAudioPlayer {
    sink: Arc<Mutex<Option<Sink>>>,
    _stream: OutputStream,                         // 🔑 Keep alive with naming
    stream_handle: OutputStreamHandle,
    current_url: Arc<Mutex<Option<String>>>,
    is_playing: Arc<Mutex<bool>>,
    is_paused: Arc<Mutex<bool>>,
}
```

**Key Pattern**: The `_stream` field uses underscore naming to indicate it's intentionally unused but must be kept alive. This prevents the audio output stream from being dropped.

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

## Conclusion

The SomaFM CLI demonstrates how to build a real-world Rust application that is:
- **Responsive**: UI never blocks on network operations
- **Robust**: Graceful error handling and recovery
- **Maintainable**: Clear separation of concerns and module boundaries
- **Performant**: Efficient resource usage and smart caching
- **User-Friendly**: Smooth interaction patterns and appropriate feedback

These patterns scale well beyond CLI applications to web services, desktop apps, and embedded systems. The key is applying Rust's ownership system and type safety to enforce good architectural boundaries while leveraging async programming for responsive user experiences.