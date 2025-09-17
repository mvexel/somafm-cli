# Lesson 2: Ownership, Borrowing & Async Patterns

## Learning Objectives
By the end of this lesson, you'll understand:
- How Rust's ownership system works in practice
- Common borrowing patterns and how to resolve borrow checker issues
- Shared ownership patterns with `Arc<Mutex<T>>`
- How async/await interacts with the borrow checker
- Practical strategies for managing state in async applications

## 1. Ownership in Action: Avoiding Borrow Conflicts

### The Problem: Borrows Across Async Boundaries

Look at this pattern from `src/app.rs:75-104` in the `play_current_station` method:

```rust
async fn play_current_station(&mut self) -> Result<()> {
    if let Some(station) = self.ui_app.current_station() {
        // Clone needed data to avoid holding borrow across awaits/mut operations
        let station_id = station.id.clone();              // 🔑 Key pattern!
        let stream_url = self.client.get_stream_url(station);

        // If already playing this station, do nothing
        if self.ui_app.audio_player.is_playing() {
            if let Some(current) = &self.ui_app.currently_playing_station_id {
                if current == &station_id {
                    return Ok(());
                }
            }
        }

        // Request track info asynchronously (set flag before borrow ends)
        self.ui_app.is_fetching_track = true;
        let _ = self.req_tx.try_send(Request::LoadTrackForStation {
            station_id: station_id.clone()
        });

        // Now we can safely use station_id without borrowing issues
        if let Some(stream_url) = stream_url {
            match self.ui_app.audio_player.play(stream_url) {
                Ok(_) => {
                    self.ui_app.currently_playing_station_id = Some(station_id);
                }
                Err(_) => {}
            }
        }
    }
    Ok(())
}
```

### Why the Clone?

**The Issue**: If we tried to hold a reference to `station.id` across the async operations, we'd get this error:
```
error: cannot borrow `self.ui_app` as mutable because it is also borrowed as immutable
```

**The Solution**: Clone the data we need upfront (`let station_id = station.id.clone()`). This is a common and correct pattern in async Rust.

**Key Insight**: In async Rust, cloning small pieces of data (like strings and IDs) is often the right choice. It's better to clone a `String` than to fight the borrow checker with complex lifetime management.

## 2. Shared Ownership with Arc<Mutex<T>>

### Thread-Safe Shared State

The audio player uses a classic pattern for sharing mutable state between threads:

```rust
// From src/audio.rs:19-26
pub struct SimpleAudioPlayer {
    sink: Arc<Mutex<Option<Sink>>>,           // 🔑 Shared, mutable state
    _stream: OutputStream,                    // 🔑 Owned resource
    stream_handle: OutputStreamHandle,
    current_url: Arc<Mutex<Option<String>>>,  // 🔑 Shared, mutable state
    is_playing: Arc<Mutex<bool>>,             // 🔑 Shared, mutable state
    is_paused: Arc<Mutex<bool>>,              // 🔑 Shared, mutable state
}
```

### Breaking Down the Pattern:

1. **`Arc<T>`** = "Atomically Reference Counted" - allows multiple owners
2. **`Mutex<T>`** = "Mutual Exclusion" - ensures only one thread can access at a time
3. **Combined** = Thread-safe shared mutable state

### How It Works in Practice:

```rust
// From src/audio.rs - simplified for clarity
impl SimpleAudioPlayer {
    pub fn is_playing(&self) -> bool {
        // Lock the mutex, read the value, lock automatically released
        *self.is_playing.lock().unwrap()
    }

    pub fn play(&self, url: String) -> Result<()> {
        // Multiple locks can be acquired in sequence
        {
            let mut current_url = self.current_url.lock().unwrap();
            *current_url = Some(url.clone());
        }  // Lock released here

        {
            let mut is_playing = self.is_playing.lock().unwrap();
            *is_playing = true;
        }  // Lock released here

        // The Arc allows us to move clones into background tasks
        let is_playing_clone = self.is_playing.clone();
        tokio::spawn(async move {
            // This closure owns is_playing_clone
            // Original AudioPlayer can still access is_playing
        });

        Ok(())
    }
}
```

**Important**: The locks are held for very short periods. Don't hold a lock across an `await` point!

## 3. Async and Ownership Challenges

### Problem: Self-References in Async Methods

This is a **wrong** pattern that won't compile:

```rust
// ❌ This won't work - borrow checker error
async fn bad_example(&mut self) -> Result<()> {
    let station = self.ui_app.current_station(); // Borrow starts here

    some_async_operation().await;                // Borrow must be held across await

    self.ui_app.update_something();              // ❌ Another mutable borrow!
    Ok(())
}
```

### Solution: Extract Data First

The correct pattern from our codebase:

```rust
// ✅ This works - from src/app.rs:75-78
async fn play_current_station(&mut self) -> Result<()> {
    if let Some(station) = self.ui_app.current_station() {
        // Extract all data we need upfront
        let station_id = station.id.clone();
        let stream_url = self.client.get_stream_url(station);
        // Now 'station' borrow is finished

        // We can safely mutate self in async operations
        self.ui_app.is_fetching_track = true;
        some_async_operation().await;
        self.ui_app.currently_playing_station_id = Some(station_id);
    }
    Ok(())
}
```

**Key Rule**: Get all the data you need from `&self` before any `await` points, then use only that extracted data.

## 4. Practical Async Patterns

### Non-Blocking Channel Communication

From `src/main.rs:87-96`, the main loop processes messages without blocking:

```rust
// Process any incoming responses without blocking
loop {
    match resp_rx.try_recv() {                    // 🔑 try_recv() never blocks
        Ok(resp) => {
            app_controller.process_response(resp).await?;
        }
        Err(mpsc::error::TryRecvError::Empty) => break,      // No messages available
        Err(mpsc::error::TryRecvError::Disconnected) => break, // Channel closed
    }
}
```

**Key Insight**: `try_recv()` vs `recv().await`:
- `try_recv()` - Returns immediately, doesn't block the UI
- `recv().await` - Waits until a message arrives, would freeze the UI

### Debouncing with Ownership

From `src/app.rs:128-155`, here's how to implement debouncing while managing ownership:

```rust
fn maybe_request_track_for_selected(&mut self) {
    const DEBOUNCE_MS: u64 = 2000;

    // Extract the station_id to avoid borrowing issues
    if let Some(station_id) = self.ui_app.current_station().map(|s| s.id.clone()) {
        let now = Instant::now();

        let should_send = match self.last_track_req.get(&station_id) {
            Some(last) => now.duration_since(*last) >= Duration::from_millis(DEBOUNCE_MS),
            None => true,
        };

        if should_send {
            self.ui_app.is_fetching_track = true;
            if self.req_tx.try_send(Request::LoadTrackForStation {
                station_id: station_id.clone()
            }).is_ok() {
                self.last_track_req.insert(station_id, now);  // 🔑 Move ownership
            } else {
                self.ui_app.is_fetching_track = false;
            }
        }
    }
}
```

## 5. Exercises

### Exercise 1: Fix a Borrowing Issue
**Current Issue**: The error handling in the codebase could be improved.

**Task**: Try to implement this function and fix the borrowing errors:

```rust
// Add this to AppController
async fn refresh_current_station_info(&mut self) -> Result<()> {
    // Get current station
    let station = self.ui_app.current_station();  // Borrow starts

    if let Some(station) = station {
        // Try to get fresh data
        let fresh_station = self.client.get_station_details(&station.id).await?;

        // Update the station in our list
        for existing_station in &mut self.ui_app.stations {  // ❌ Borrowing issue!
            if existing_station.id == station.id {
                *existing_station = fresh_station;
                break;
            }
        }
    }

    Ok(())
}
```

**Hint**: Extract the station ID before the async call, similar to the patterns we've seen.

### Exercise 2: Add Caching with Shared State
**Goal**: Add a cache to avoid redundant API calls.

**Task**: Create a shared cache structure:

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ApiCache {
    tracks: Arc<Mutex<HashMap<String, (Track, Instant)>>>,
    stations: Arc<Mutex<Option<(Vec<Station>, Instant)>>>,
    cache_duration: Duration,
}

impl ApiCache {
    pub fn new() -> Self {
        Self {
            tracks: Arc::new(Mutex::new(HashMap::new())),
            stations: Arc::new(Mutex::new(None)),
            cache_duration: Duration::from_secs(30),
        }
    }

    pub fn get_track(&self, station_id: &str) -> Option<Track> {
        // TODO: Implement cache lookup with expiration
        None
    }

    pub fn cache_track(&self, station_id: String, track: Track) {
        // TODO: Implement cache storage
    }
}
```

**Challenge Points**:
1. How do you handle the lock properly?
2. How do you check if cached data is still valid?
3. How do you integrate this with the existing async worker pattern?

### Exercise 3: Improve Resource Management
**Current Issue**: The audio player doesn't handle resource cleanup well.

**Task**: Add proper Drop implementation and resource management:

```rust
impl Drop for SimpleAudioPlayer {
    fn drop(&mut self) {
        // TODO: Properly stop playback and clean up resources
        // Hint: You'll need to work with the Arc<Mutex<T>> pattern
    }
}
```

**Additional Challenge**: Add a method to gracefully shutdown the player:

```rust
impl SimpleAudioPlayer {
    pub async fn shutdown(&self) -> Result<()> {
        // TODO: Stop playback, wait for background tasks to finish
        // How do you coordinate between the async background task and this method?
    }
}
```

### Exercise 4: Channel Communication Pattern
**Goal**: Add a feature to show playback history.

**Task**: Extend the Request/Response pattern to track playback events:

```rust
// Add to actions.rs
#[derive(Debug, Clone)]
pub enum Request {
    LoadStations,
    LoadTrackForStation { station_id: String },
    LogPlaybackEvent { station_id: String, track: Track }, // New!
}

#[derive(Debug)]
pub enum Response {
    StationsLoaded(Result<Vec<Station>, Error>),
    TrackLoaded { station_id: String, result: Result<Option<Track>, Error> },
    PlaybackLogged, // New!
}
```

**Challenges**:
1. How do you send the playback event without blocking the audio playback?
2. Where do you store the playback history?
3. How do you handle the case where the channel is full?

## Common Patterns Summary

### 1. Extract Before Async
```rust
// ✅ Good
let data = self.extract_needed_data();
some_async_operation().await;
self.use_data(data);

// ❌ Bad
let data_ref = &self.some_field;
some_async_operation().await;  // Borrow held across await
self.mutate_something();       // Conflict!
```

### 2. Shared State
```rust
// For data shared between threads/tasks
Arc<Mutex<T>>     // Most common
Arc<RwLock<T>>    // When you have many readers, few writers
mpsc::channel     // For message passing (often better than shared state)
```

### 3. Non-Blocking Operations
```rust
// ✅ UI-friendly
match channel.try_recv() { ... }

// ❌ Blocks UI
let msg = channel.recv().await;
```

## Key Takeaways

1. **Clone Small Data**: It's often better to clone strings/IDs than fight the borrow checker
2. **Extract Before Async**: Get all data from `&self` before any `await` points
3. **Shared State Pattern**: `Arc<Mutex<T>>` for thread-safe shared mutable state
4. **Short Lock Duration**: Never hold a lock across an `await` point
5. **Non-Blocking UI**: Use `try_recv()` instead of `recv().await` in UI loops

## Next Steps

In the next lesson, we'll explore how to design robust data structures with serde, looking at the API integration patterns and custom serialization logic in the SomaFM client.