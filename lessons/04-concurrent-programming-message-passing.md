# Lesson 4: Concurrent Programming & Message Passing

## Learning Objectives
By the end of this lesson, you'll understand:
- How to build responsive applications with background workers
- Message passing patterns with tokio channels
- Non-blocking UI design with async communication
- Debouncing and rate limiting strategies
- Error handling in concurrent environments

## 1. The Responsive UI Challenge

### The Problem

Without concurrency, a UI application would look like this:

```rust
// ❌ This blocks the UI thread
loop {
    handle_user_input();

    // UI freezes here while waiting for network
    let stations = api_client.get_stations().await?;  // 500ms+ network delay

    update_ui(stations);
    render_ui();
}
```

Every API call would freeze the interface. Users expect applications to remain responsive even during network operations.

### The Solution: Background Workers

The SomaFM CLI uses a worker pattern to keep the UI responsive:

```rust
// From src/main.rs:25-68 - Overview of the architecture
#[tokio::main]
async fn main() -> Result<()> {
    // Create channels for background worker communication
    let (req_tx, req_rx) = mpsc::channel::<Request>(64);    // 🔑 UI → Worker
    let (resp_tx, resp_rx) = mpsc::channel::<Response>(64);  // 🔑 Worker → UI

    // Spawn background worker task
    tokio::spawn(worker_loop(req_rx, resp_tx));              // 🔑 Runs independently

    // Initialize app controller with request sender
    let mut app_controller = AppController::new(audio_player, req_tx.clone());

    // Run the main UI loop
    let res = run_app(&mut terminal, &mut app_controller, req_tx, resp_rx).await;
    // ... cleanup
}
```

**Key Architecture**: The UI never calls API functions directly. Instead, it sends requests to a background worker and processes responses asynchronously.

## 2. Channel Communication Patterns

### Message Types

The communication uses typed messages defined in `src/actions.rs`:

```rust
// Requests from UI/controller to the worker
#[derive(Debug, Clone)]
pub enum Request {
    LoadStations,
    LoadTrackForStation { station_id: String },
}

// Responses from worker back to UI/controller
#[derive(Debug)]
pub enum Response {
    StationsLoaded(Result<Vec<Station>, Error>),
    TrackLoaded { station_id: String, result: Result<Option<Track>, Error> },
}
```

**Design Insight**: Each request has a corresponding response type. This makes the communication protocol explicit and type-safe.

### The Worker Loop

```rust
// From src/main.rs:137-151
async fn worker_loop(mut req_rx: mpsc::Receiver<Request>, resp_tx: mpsc::Sender<Response>) {
    let client = api::SomaFMClient::new();                      // 🔑 Worker owns the client

    while let Some(req) = req_rx.recv().await {                // 🔑 Wait for requests
        match req {
            Request::LoadStations => {
                let res = client.get_stations().await;          // 🔑 Blocking operation
                let _ = resp_tx.send(Response::StationsLoaded(res)).await;
            }
            Request::LoadTrackForStation { station_id } => {
                let res = client.get_current_track(&station_id).await;
                let _ = resp_tx.send(Response::TrackLoaded {
                    station_id,
                    result: res
                }).await;
            }
        }
    }
}
```

**Key Patterns**:
1. **Worker Owns Resources**: The HTTP client lives in the worker thread
2. **Blocking Operations**: Network calls can take as long as needed without affecting UI
3. **Error Handling**: Errors are wrapped in `Response` and sent back to UI

### Non-Blocking UI Processing

```rust
// From src/main.rs:87-96
// Process any incoming responses without blocking
loop {
    match resp_rx.try_recv() {                                  // 🔑 Non-blocking receive
        Ok(resp) => {
            app_controller.process_response(resp).await?;       // 🔑 Handle response
        }
        Err(mpsc::error::TryRecvError::Empty) => break,         // 🔑 No messages available
        Err(mpsc::error::TryRecvError::Disconnected) => break,  // 🔑 Worker died
    }
}
```

**Critical Pattern**: `try_recv()` instead of `recv().await`:
- `try_recv()` returns immediately, allowing UI to stay responsive
- `recv().await` would block until a message arrives, freezing the UI

## 3. Debouncing and Rate Limiting

### The Problem: Too Many Requests

Without debouncing, rapid user actions create spam:

```rust
// User rapidly presses arrow keys:
// ↓ ↓ ↓ ↓ ↓ ↓ ↓
// LoadTrackForStation(station1)
// LoadTrackForStation(station2)
// LoadTrackForStation(station3)
// LoadTrackForStation(station4)
// ... 20 more requests
```

### The Solution: Smart Debouncing

```rust
// From src/app.rs:128-155
fn maybe_request_track_for_selected(&mut self) {
    const DEBOUNCE_MS: u64 = 2000;                              // 🔑 2 second cooldown

    if let Some(station_id) = self.ui_app.current_station().map(|s| s.id.clone()) {
        // Only fetch if certain conditions are met
        if !self.ui_app.audio_player.is_playing() {
            return;                                             // 🔑 Skip if not playing
        } else if let Some(current) = &self.ui_app.currently_playing_station_id {
            if current != &station_id {
                return;                                         // 🔑 Skip if different station
            }
        }

        let now = Instant::now();
        let should_send = match self.last_track_req.get(&station_id) {
            Some(last) => now.duration_since(*last) >= Duration::from_millis(DEBOUNCE_MS),
            None => true,                                       // 🔑 First request always allowed
        };

        if should_send {
            self.ui_app.is_fetching_track = true;
            if self.req_tx.try_send(Request::LoadTrackForStation {
                station_id: station_id.clone()
            }).is_ok() {
                self.last_track_req.insert(station_id, now);   // 🔑 Record request time
            } else {
                self.ui_app.is_fetching_track = false;         // 🔑 Channel full, reset flag
            }
        }
    }
}
```

**Multi-Layer Protection**:
1. **Business Logic**: Only fetch for currently playing station
2. **Time-Based Debouncing**: Minimum 2 seconds between requests per station
3. **Channel Backpressure**: Use `try_send()` to avoid blocking if worker is busy

### Channel Capacity and Backpressure

```rust
// From src/main.rs:48-49
let (req_tx, req_rx) = mpsc::channel::<Request>(64);    // 🔑 Bounded channel
let (resp_tx, resp_rx) = mpsc::channel::<Response>(64);
```

**Why Bounded Channels**:
- **Backpressure**: When the channel is full, `try_send()` fails instead of buffering infinitely
- **Memory Control**: Prevents unlimited memory usage if worker falls behind
- **Natural Rate Limiting**: Fast producers are slowed down by slow consumers

## 4. Periodic Tasks and State Management

### Periodic Refresh Pattern

```rust
// From src/main.rs:114-122
if app_controller.ui_app.audio_player.is_playing() {
    if last_play_refresh.elapsed() >= play_refresh_interval {    // 🔑 Time-based trigger
        if let Some(station) = app_controller.ui_app.current_station() {
            let _ = _req_tx.try_send(actions::Request::LoadTrackForStation {
                station_id: station.id.clone()
            });
        }
        last_play_refresh = std::time::Instant::now();          // 🔑 Reset timer
    }
}
```

**Pattern**: Combine user actions with periodic updates for fresh data without spam.

### State Synchronization

```rust
// From src/app.rs:162-202
pub async fn process_response(&mut self, resp: Response) -> Result<()> {
    match resp {
        Response::StationsLoaded(res) => match res {
            Ok(stations) => {
                self.ui_app.stations = stations;                // 🔑 Update app state
                self.ui_app.invalidate_station_cache();
                if !self.ui_app.stations.is_empty() {
                    self.ui_app.select_station(0);
                }
                self.ui_app.is_fetching_stations = false;       // 🔑 Clear loading flag
            }
            Err(_e) => {
                self.ui_app.is_fetching_stations = false;       // 🔑 Clear on error too
            }
        },
        Response::TrackLoaded { station_id, result } => {
            // Only update if this track belongs to the currently relevant station
            let apply = if let Some(current_playing) = &self.ui_app.currently_playing_station_id {
                &station_id == current_playing                  // 🔑 Check relevance
            } else if let Some(selected) = self.ui_app.current_station().map(|s| s.id.clone()) {
                station_id == selected
            } else {
                false
            };

            if apply {
                match result {
                    Ok(track) => {
                        self.ui_app.current_track = track;      // 🔑 Update only if relevant
                    }
                    Err(_e) => {
                        // Keep previous track on error
                    }
                }
            }
            self.ui_app.is_fetching_track = false;             // 🔑 Always clear loading flag
        }
    }
    Ok(())
}
```

**Key Insight**: Only update UI state if the response is still relevant. User might have switched stations while a request was in flight.

## 5. Error Handling in Concurrent Systems

### Worker Error Handling

```rust
// Worker can't crash - it just sends errors back
async fn worker_loop(mut req_rx: mpsc::Receiver<Request>, resp_tx: mpsc::Sender<Response>) {
    let client = api::SomaFMClient::new();

    while let Some(req) = req_rx.recv().await {
        match req {
            Request::LoadStations => {
                let res = client.get_stations().await;         // 🔑 May fail
                let _ = resp_tx.send(Response::StationsLoaded(res)).await; // 🔑 Send Result
            }
            // ... other cases
        }
    }
}
```

**Pattern**: Worker never panics. All errors are wrapped in `Result` and sent to UI for handling.

### UI Error Handling

```rust
// From src/app.rs:173-176
Err(_e) => {
    self.ui_app.is_fetching_stations = false;
    // TODO: surface error in UI                              // 🔑 Current gap!
}
```

**Current Issue**: Errors are silently ignored. This is a real improvement opportunity!

## 6. Exercises

### Exercise 1: Improve Error Handling
**Current Issue**: Network errors disappear silently.

**Task**: Add proper error display to the UI:

```rust
// Add to UIState
pub struct UIState {
    // ... existing fields
    pub error_message: Option<String>,
    pub error_displayed_at: Option<Instant>,
}

// Add error processing
impl AppController {
    pub async fn process_response(&mut self, resp: Response) -> Result<()> {
        match resp {
            Response::StationsLoaded(res) => match res {
                Ok(stations) => {
                    // ... success handling
                    self.ui_app.clear_error();  // TODO: Implement
                }
                Err(e) => {
                    self.ui_app.show_error(format!("Failed to load stations: {}", e)); // TODO: Implement
                    self.ui_app.is_fetching_stations = false;
                }
            },
            // ... handle other error cases
        }
        Ok(())
    }
}

// Auto-clear errors after timeout
impl UIState {
    pub fn update_error_display(&mut self) {
        if let Some(displayed_at) = self.error_displayed_at {
            if displayed_at.elapsed() > Duration::from_secs(5) {
                self.error_message = None;
                self.error_displayed_at = None;
            }
        }
    }
}
```

**Challenges**:
1. How do you show errors without blocking the UI?
2. When should errors auto-clear vs. require user dismissal?
3. How do you prioritize multiple simultaneous errors?

### Exercise 2: Add Request Prioritization
**Current Issue**: All requests are treated equally.

**Task**: Add priority levels to requests:

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,      // Background refresh
    Normal,   // User selection
    High,     // User play action
    Critical, // Error recovery
}

#[derive(Debug, Clone)]
pub struct PriorityRequest {
    pub request: Request,
    pub priority: Priority,
    pub submitted_at: Instant,
}

// Use a priority queue instead of simple channel
use std::collections::BinaryHeap;

async fn priority_worker_loop(
    mut req_rx: mpsc::Receiver<PriorityRequest>,
    resp_tx: mpsc::Sender<Response>
) {
    let client = api::SomaFMClient::new();
    let mut pending_requests = BinaryHeap::new();

    loop {
        // Collect all available requests
        while let Ok(req) = req_rx.try_recv() {
            pending_requests.push(req);
        }

        if let Some(priority_req) = pending_requests.pop() {
            // Process highest priority request
            // TODO: Implement request processing with priority
        } else {
            // Wait for new requests
            if let Some(req) = req_rx.recv().await {
                pending_requests.push(req);
            }
        }
    }
}
```

**Challenge**: How do you prevent low-priority requests from starving?

### Exercise 3: Add Request Cancellation
**Current Issue**: Stale requests can't be cancelled.

**Task**: Add cancellation tokens to requests:

```rust
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub enum Request {
    LoadStations {
        cancel_token: CancellationToken,
    },
    LoadTrackForStation {
        station_id: String,
        cancel_token: CancellationToken,
    },
}

// Usage in app controller
impl AppController {
    async fn play_current_station(&mut self) -> Result<()> {
        // Cancel any pending track requests for other stations
        self.cancel_pending_track_requests();

        if let Some(station) = self.ui_app.current_station() {
            let cancel_token = CancellationToken::new();
            self.current_track_cancel_token = Some(cancel_token.clone());

            let _ = self.req_tx.try_send(Request::LoadTrackForStation {
                station_id: station.id.clone(),
                cancel_token,
            });
        }
        Ok(())
    }

    fn cancel_pending_track_requests(&mut self) {
        if let Some(token) = &self.current_track_cancel_token {
            token.cancel();
        }
    }
}

// Worker checks cancellation
async fn worker_loop(mut req_rx: mpsc::Receiver<Request>, resp_tx: mpsc::Sender<Response>) {
    let client = api::SomaFMClient::new();

    while let Some(req) = req_rx.recv().await {
        match req {
            Request::LoadTrackForStation { station_id, cancel_token } => {
                // Check if cancelled before starting
                if cancel_token.is_cancelled() {
                    continue;
                }

                let res = tokio::select! {
                    result = client.get_current_track(&station_id) => result,
                    _ = cancel_token.cancelled() => {
                        continue; // Request was cancelled
                    }
                };

                // Check again before sending response
                if !cancel_token.is_cancelled() {
                    let _ = resp_tx.send(Response::TrackLoaded {
                        station_id,
                        result: res
                    }).await;
                }
            }
            // ... other cases
        }
    }
}
```

### Exercise 4: Add Retry Logic with Exponential Backoff
**Current Issue**: Network failures are not retried.

**Task**: Add smart retry logic to the worker:

```rust
use std::time::Duration;

#[derive(Debug)]
struct RetryConfig {
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
        }
    }
}

async fn worker_loop_with_retry(
    mut req_rx: mpsc::Receiver<Request>,
    resp_tx: mpsc::Sender<Response>
) {
    let client = api::SomaFMClient::new();
    let retry_config = RetryConfig::default();

    while let Some(req) = req_rx.recv().await {
        match req {
            Request::LoadStations => {
                let res = retry_with_backoff(
                    || client.get_stations(),
                    &retry_config
                ).await;

                let _ = resp_tx.send(Response::StationsLoaded(res)).await;
            }
            // ... other cases
        }
    }
}

async fn retry_with_backoff<F, Fut, T, E>(
    mut operation: F,
    config: &RetryConfig
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempts = 0;
    let mut delay = config.base_delay;

    loop {
        attempts += 1;

        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if attempts >= config.max_attempts => return Err(e),
            Err(e) => {
                log::warn!("Attempt {} failed: {}. Retrying in {:?}", attempts, e, delay);
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, config.max_delay);
            }
        }
    }
}
```

**Challenges**:
1. Which errors should be retried vs. which should fail immediately?
2. How do you handle cancellation during retry delays?
3. Should retry attempts be per-request or per-session?

## Architecture Patterns Summary

### 1. Worker Pattern
```rust
// ✅ Responsive UI
UI Thread:  [Handle Input] → [Send Request] → [Process Response] → [Render]
                ↓                               ↑
Worker Thread:       [Receive] → [API Call] → [Send Response]

// ❌ Blocking UI
UI Thread: [Handle Input] → [API Call - BLOCKS!] → [Render]
```

### 2. Message Passing
```rust
// ✅ Type-safe communication
enum Request { LoadData { id: String } }
enum Response { DataLoaded(Result<Data, Error>) }

// ❌ Loosely typed
mpsc::channel::<String>()  // What format? What responses?
```

### 3. Non-blocking Processing
```rust
// ✅ Responsive
match channel.try_recv() { ... }

// ❌ Blocking
let msg = channel.recv().await;
```

### 4. Smart Debouncing
```rust
// ✅ Multiple protection layers
if !should_request(business_logic) { return; }
if !debounce_timer.should_send() { return; }
if channel.try_send().is_err() { handle_backpressure(); }

// ❌ Naive
channel.send(request).await;  // Spam!
```

## Key Takeaways

1. **Separation of Concerns**: UI handles interaction, workers handle I/O
2. **Type-Safe Communication**: Use enums for request/response protocols
3. **Non-Blocking Operations**: `try_recv()` and bounded channels prevent UI freezing
4. **Smart Rate Limiting**: Combine business logic, time-based debouncing, and backpressure
5. **Error Boundaries**: Wrap errors in messages, handle them in the UI layer
6. **State Relevance**: Only apply responses that are still relevant to current UI state

## Next Steps

In the final lesson, we'll examine the overall application architecture, exploring how all these patterns work together to create a maintainable, responsive application. We'll also look at testing strategies and deployment considerations.