# Lesson 3: Data Modeling & Serialization

## Learning Objectives
By the end of this lesson, you'll understand:
- How to design robust data structures with serde
- Custom serialization and deserialization patterns
- Handling inconsistent external APIs gracefully
- Error handling strategies for data processing
- Type-safe API integration patterns

## 1. Modeling External Data

### The Challenge: Inconsistent APIs

Real-world APIs are messy. Look at how the SomaFM API represents data inconsistently:

```json
// Sometimes listeners is a number
{"listeners": 142}

// Sometimes it's a string
{"listeners": "142"}

// Sometimes genre is a string
{"genre": "Electronic"}

// Sometimes it's an array
{"genre": ["Electronic", "Ambient"]}
```

In languages like JavaScript, you'd just hope for the best. In Rust, we handle this explicitly.

### Robust Data Structures

Here's how the SomaFM client handles these inconsistencies:

```rust
// From src/api.rs:30-48
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Station {
    pub id: String,
    pub title: String,
    #[serde(default)]                                    // 🔑 Handle missing fields
    pub description: String,
    #[serde(deserialize_with = "deserialize_listeners")] // 🔑 Custom deserializer
    pub listeners: u32,
    #[serde(default)]                                    // 🔑 Handle missing fields
    pub image: String,
    #[serde(rename = "lastPlaying", default)]            // 🔑 Field name mapping
    pub last_playing: String,
    #[serde(deserialize_with = "deserialize_genre")]     // 🔑 Custom deserializer
    pub genre: Vec<String>,
    #[serde(default)]                                    // 🔑 Handle missing fields
    pub dj: String,
    #[serde(default)]                                    // 🔑 Handle missing fields
    pub playlists: Vec<Playlist>,
}
```

### Key Patterns:

1. **`#[serde(default)]`** - Uses the type's `Default` implementation if the field is missing
2. **`#[serde(rename = "...")]`** - Maps JSON field names to Rust field names
3. **`#[serde(deserialize_with = "...")]`** - Uses custom logic for complex transformations

## 2. Custom Deserializers

### Handling String-or-Number Fields

The API sometimes sends numbers as strings. Here's how to handle it robustly:

```rust
// From src/api.rs:74-90
fn deserialize_listeners<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]                    // 🔑 Try variants without type tags
    enum StringOrNumber {
        String(String),
        Number(u32),
    }

    match StringOrNumber::deserialize(deserializer) {
        Ok(StringOrNumber::String(s)) => {
            s.parse().map_err(serde::de::Error::custom)  // 🔑 Parse string to number
        }
        Ok(StringOrNumber::Number(n)) => Ok(n),         // 🔑 Use number directly
        Err(_) => Ok(0),                                // 🔑 Graceful fallback
    }
}
```

### The `untagged` Pattern

The `#[serde(untagged)]` attribute tells serde to try deserializing each variant in order until one succeeds. This is perfect for APIs that send inconsistent types.

### Handling String-or-Array Fields

```rust
// From src/api.rs:56-72
fn deserialize_genre<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),                   // Single genre as string
        Vec(Vec<String>),                 // Multiple genres as array
    }

    match StringOrVec::deserialize(deserializer) {
        Ok(StringOrVec::String(s)) => Ok(vec![s]),        // 🔑 Wrap single item
        Ok(StringOrVec::Vec(v)) => Ok(v),                 // 🔑 Use array directly
        Err(_) => Ok(vec![]),                             // 🔑 Empty on error
    }
}
```

**Key Insight**: Always normalize to the most useful format for your application. Here, we always return `Vec<String>` regardless of input format.

## 3. Error Handling Strategies

### Graceful Degradation

Notice the pattern in all custom deserializers:

```rust
Err(_) => Ok(0),        // Default to 0 listeners on error
Err(_) => Ok(vec![]),   // Default to empty genres on error
```

**Philosophy**: For a music player, it's better to show a station with missing/default data than to fail completely.

### When to Fail vs. When to Default

```rust
// Critical data - should fail if missing
pub struct Track {
    pub title: String,     // No #[serde(default)] - must be present
    pub artist: String,    // No #[serde(default)] - must be present

    // Optional/enhancement data - graceful defaults
    #[serde(default)]
    pub album: String,     // Empty string if missing
    #[serde(rename = "albumArt", default)]
    pub album_art: String, // Empty string if missing
}
```

**Rule**: Fail for data essential to core functionality, default for enhancement data.

## 4. API Integration Patterns

### Client Structure

```rust
// From src/api.rs:110-119
pub struct SomaFMClient {
    client: reqwest::Client,    // 🔑 Reusable HTTP client
}

impl SomaFMClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),  // 🔑 Connection pooling built-in
        }
    }
}
```

**Best Practice**: Reuse the `reqwest::Client` instance. It handles connection pooling, DNS caching, and other optimizations internally.

### Async API Methods

```rust
// From src/api.rs:121-136
pub async fn get_stations(&self) -> Result<Vec<Station>> {
    let response = self
        .client
        .get("https://somafm.com/channels.json")
        .send()                              // 🔑 Send request
        .await?;                             // 🔑 Await response

    let mut channels_response: ChannelsResponse = response.json().await?; // 🔑 Parse JSON

    // Sort by listener count (popularity)
    channels_response
        .channels
        .sort_by(|a, b| b.listeners.cmp(&a.listeners));  // 🔑 Post-process data

    Ok(channels_response.channels)
}
```

### Error Propagation with `?`

The `?` operator automatically converts errors and returns early:
- `send().await?` - Converts `reqwest::Error` to `anyhow::Error`
- `response.json().await?` - Converts JSON parsing errors
- Function returns `Result<Vec<Station>>` - Caller can handle or propagate further

## 5. Data Processing Pipeline

### From JSON to Business Logic

```rust
// 1. Raw JSON from API
{
  "id": "groovesalad",
  "title": "Groove Salad",
  "listeners": "142",          // String!
  "genre": "Electronic"        // Single string!
}

// 2. Intermediate representation (internal struct)
ChannelsResponse {
    channels: Vec<Station>
}

// 3. Application-ready data
Station {
    id: "groovesalad",
    title: "Groove Salad",
    listeners: 142,             // Parsed to u32
    genre: vec!["Electronic"],  // Normalized to Vec<String>
    // ... other fields with defaults
}

// 4. Sorted by popularity
Vec<Station> // Sorted by listener count, ready for UI
```

### Quality Selection Logic

```rust
// From src/api.rs:138-154 - Business logic in the client
pub fn get_stream_url(&self, station: &Station) -> Option<String> {
    let mut best_playlist = None;

    for playlist in &station.playlists {
        if playlist.format == "mp3" {           // 🔑 Filter by format
            match playlist.quality.as_str() {
                "highest" => return Some(playlist.url.clone()), // 🔑 Best quality wins
                "high" if best_playlist.is_none() => {
                    best_playlist = Some(&playlist.url)         // 🔑 Fallback option
                }
                _ if best_playlist.is_none() => {
                    best_playlist = Some(&playlist.url)         // 🔑 Any MP3 as last resort
                }
                _ => {}
            }
        }
    }

    best_playlist.cloned()
}
```

**Key Pattern**: Prefer highest quality, fallback gracefully, return `Option` for safety.

## 6. Exercises

### Exercise 1: Handle More API Inconsistencies
**Current Issue**: The API sometimes sends invalid date formats.

**Task**: Improve the date deserializer to handle more formats:

```rust
// Current implementation only handles u64 and string numbers
fn deserialize_date<'de, D>(deserializer: D) -> Result<u64, D::Error> {
    // TODO: Handle these additional formats:
    // - "2024-01-15T10:30:00Z" (ISO 8601)
    // - "Jan 15, 2024" (human readable)
    // - 1704123000 (unix timestamp)
    // - "invalid" (should default to current time)
}
```

**Hints**:
- Use the `chrono` crate for date parsing
- Consider using a custom enum with more variants
- Think about time zones and current time defaults

### Exercise 2: Add Response Caching
**Goal**: Reduce API calls with intelligent caching.

**Task**: Create a caching layer for API responses:

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct CachedResponse<T> {
    data: T,
    cached_at: Instant,
    expires_in: Duration,
}

impl<T> CachedResponse<T> {
    fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > self.expires_in
    }
}

pub struct CachingClient {
    inner: SomaFMClient,
    station_cache: Option<CachedResponse<Vec<Station>>>,
    track_cache: HashMap<String, CachedResponse<Vec<Track>>>,
}

impl CachingClient {
    pub fn new() -> Self {
        // TODO: Implement
    }

    pub async fn get_stations(&mut self) -> Result<Vec<Station>> {
        // TODO: Check cache first, only hit API if expired
    }

    pub async fn get_current_track(&mut self, station_id: &str) -> Result<Option<Track>> {
        // TODO: Implement with per-station track caching
    }
}
```

**Challenge Points**:
1. How do you handle the `&mut self` requirement for caching?
2. What cache expiration times make sense for different data types?
3. How do you handle cache invalidation?

### Exercise 3: Add Data Validation
**Current Issue**: The app assumes API data is valid, but what if it's not?

**Task**: Add validation to the data structures:

```rust
use serde::{Deserialize, Deserializer};

// Add validation for Station data
impl Station {
    fn validate(&self) -> Result<(), ValidationError> {
        // TODO: Implement validation rules:
        // - ID should not be empty
        // - Title should not be empty
        // - At least one playlist should exist
        // - All playlist URLs should be valid
        // - Listener count should be reasonable (0-100000?)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Station ID cannot be empty")]
    EmptyId,
    #[error("Station title cannot be empty")]
    EmptyTitle,
    // TODO: Add more specific error types
}

// Custom deserializer that validates
fn deserialize_station<'de, D>(deserializer: D) -> Result<Station, D::Error>
where
    D: Deserializer<'de>,
{
    let station = Station::deserialize(deserializer)?;
    station.validate().map_err(serde::de::Error::custom)?;
    Ok(station)
}
```

### Exercise 4: Stream Processing
**Current Issue**: The current track API returns all recent tracks, but we only use the first one.

**Task**: Add a streaming API that processes tracks as they arrive:

```rust
use futures_util::stream::Stream;
use std::pin::Pin;

impl SomaFMClient {
    pub fn track_stream(&self, station_id: &str) -> Pin<Box<dyn Stream<Item = Result<Track>>>> {
        // TODO: Return a stream that:
        // 1. Polls the API periodically (every 30 seconds)
        // 2. Yields only new tracks (not duplicates)
        // 3. Handles API errors gracefully
        // 4. Backs off on repeated failures
    }
}

// Usage in the app:
async fn handle_track_stream(client: &SomaFMClient, station_id: &str) {
    let mut stream = client.track_stream(station_id);

    while let Some(track_result) = stream.next().await {
        match track_result {
            Ok(track) => {
                // Update UI with new track
            }
            Err(e) => {
                // Handle error (log, show in UI, etc.)
            }
        }
    }
}
```

**Challenge**: How do you detect duplicate tracks? Should you compare by title+artist, or use the timestamp?

## Design Principles Summary

### 1. Robustness Over Strictness
```rust
// ✅ Graceful handling
#[serde(default)]
pub field: String,

// ❌ Fragile - fails on missing field
pub field: String,
```

### 2. Normalize Inconsistent Data
```rust
// ✅ Always Vec<String> regardless of input
#[serde(deserialize_with = "normalize_to_vec")]
pub genres: Vec<String>,

// ❌ Forces caller to handle inconsistency
pub genres: serde_json::Value,
```

### 3. Fail Fast for Critical Data
```rust
// ✅ Must be present for core functionality
pub id: String,
pub title: String,

// ✅ Can be missing without breaking functionality
#[serde(default)]
pub description: String,
```

### 4. Type Safety at Boundaries
```rust
// ✅ Parse and validate at API boundary
pub async fn get_stations(&self) -> Result<Vec<Station>>

// ❌ Pass raw JSON throughout application
pub async fn get_stations(&self) -> Result<serde_json::Value>
```

## Key Takeaways

1. **Custom Deserializers**: Handle API inconsistencies with `#[serde(deserialize_with)]`
2. **Graceful Degradation**: Use `#[serde(default)]` for non-critical fields
3. **Untagged Enums**: Perfect for APIs that send inconsistent types
4. **Error Boundaries**: Validate and transform data at API boundaries
5. **Business Logic**: Keep domain logic in your types, not scattered throughout the app

## Next Steps

In the next lesson, we'll explore how to build responsive applications using channels and background workers, examining the async communication patterns that keep the UI smooth while handling API calls and audio streaming.