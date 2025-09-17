# Lesson 1: Rust Project Structure & Module System

## Learning Objectives
By the end of this lesson, you'll understand:
- How to organize Rust projects with Cargo.toml
- Rust's module system and visibility rules
- Dependency management and feature flags
- Code organization patterns for maintainable applications

## 1. Project Structure Overview

Let's examine how the SomaFM CLI project is organized:

```
somafm-cli/
├── Cargo.toml          # Project configuration and dependencies
├── src/
│   ├── main.rs         # Application entry point
│   ├── api.rs          # External API integration
│   ├── app.rs          # Application controller logic
│   ├── audio.rs        # Audio playback functionality
│   ├── ui.rs           # Terminal UI rendering
│   ├── actions.rs      # Message types for async communication
│   └── bin/
│       └── somafm-no-audio.rs  # Alternative binary without audio
└── target/             # Build artifacts (generated)
```

### Key Insight: Separation of Concerns
Notice how each module has a single, clear responsibility. This isn't just good practice—it's essential in Rust because the ownership system makes it harder to create tightly coupled code.

## 2. Cargo.toml Analysis

Let's examine the project configuration:

```toml
[package]
name = "somafm-tui"
version = "0.1.0"
edition = "2021"
description = "A retro-styled TUI for Soma FM radio stations"
license = "MIT"
default-run = "somafm-tui"  # Which binary to run by default

[dependencies]
# UI and terminal handling
ratatui = "0.28"
crossterm = "0.28"

# Async runtime and utilities
tokio = { version = "1.0", features = ["full"] }
tokio-util = { version = "0.7", features = ["io-util"] }

# HTTP client and serialization
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Error handling and CLI
anyhow = "1.0"
clap = { version = "4.0", features = ["derive"] }

# Audio dependencies
rodio = "0.19"
symphonia = { version = "0.5", features = ["all"] }

# Multiple binary targets
[[bin]]
name = "somafm-tui"
path = "src/main.rs"

[[bin]]
name = "somafm-no-audio"
path = "src/bin/somafm-no-audio.rs"
```

### Key Concepts:

**Feature Flags**: Notice `features = ["derive"]` for serde and `features = ["full"]` for tokio. This allows you to include only the parts of a crate you need, reducing compile time and binary size.

**Multiple Binaries**: The `[[bin]]` sections define two different executables from the same codebase—one with audio support and one without.

## 3. Module System in Practice

### Main Module Declaration (`src/main.rs:1-5`)
```rust
mod api;        // Declares the api module (src/api.rs)
mod app;        // Declares the app module (src/app.rs)
mod audio;      // Declares the audio module (src/audio.rs)
mod ui;         // Declares the ui module (src/ui.rs)
mod actions;    // Declares the actions module (src/actions.rs)
```

**Key Rule**: When you write `mod api;`, Rust looks for either:
- `src/api.rs` (what we have)
- `src/api/mod.rs` (alternative for larger modules)

### Import Patterns (`src/main.rs:7-24`)
```rust
use anyhow::Result;                    // External crate import
use app::AppController;                // Import from our app module
use actions::{Request, Response};      // Import multiple items
use crossterm::{                       // Nested import with grouping
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
```

**Best Practice**: Group related imports and use `self` when you need both the module and items from it.

### Cross-Module Communication (`src/app.rs:6-11`)
```rust
use crate::{                           // crate:: refers to our project root
    api::SomaFMClient,                 // Import from api module
    audio::SimpleAudioPlayer,          // Import from audio module
    ui::UIState as UIApp,              // Import with renaming
};
use crate::actions::{Request, Response}; // Import message types
```

**Key Insight**: `crate::` always refers to your project root, making imports explicit and clear.

## 4. Visibility and Encapsulation

### Public Interface (`src/actions.rs`)
```rust
// Requests from UI/controller to the worker
#[derive(Debug, Clone)]
pub enum Request {                     // pub = visible to other modules
    LoadStations,
    LoadTrackForStation { station_id: String },
}

// Responses from worker back to UI/controller
#[derive(Debug)]
pub enum Response {                    // pub = part of the module's API
    StationsLoaded(Result<Vec<Station>, Error>),
    TrackLoaded { station_id: String, result: Result<Option<Track>, Error> },
}
```

### Internal Implementation Details
In contrast, look at `src/app.rs:128` - the `maybe_request_track_for_selected` method has no `pub` keyword, making it private to the `AppController` implementation.

**Design Principle**: Only expose what other modules need to use. Everything else should remain private.

## 5. Exercises

### Exercise 1: Add Configuration Support
**Goal**: Add a configuration module to handle user settings.

**Task**: Create `src/config.rs` with:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub default_station: Option<String>,
    pub audio_buffer_size: usize,
    pub refresh_interval_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_station: None,
            audio_buffer_size: 8192,
            refresh_interval_seconds: 5,
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        // TODO: Load from ~/.config/somafm/config.toml
        // For now, return default
        Ok(Self::default())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        // TODO: Save to ~/.config/somafm/config.toml
        Ok(())
    }
}
```

**Steps**:
1. Add the `toml` crate to Cargo.toml: `toml = "0.8"`
2. Create the config module
3. Add `mod config;` to main.rs
4. Import and use it in AppController::new()

**Challenge**: Implement actual file I/O for loading/saving the config.

### Exercise 2: Improve Error Handling
**Current Issue**: Look at `src/app.rs:173-176` - errors are silently ignored.

**Task**: Create a proper error handling strategy:
1. Add an `errors` module with custom error types
2. Create specific error variants for network, audio, and parsing failures
3. Update the UI to show error messages to users

**Hint**: Use `thiserror` crate for better error definitions:
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SomaFMError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Audio playback error: {0}")]
    Audio(String),

    #[error("Failed to parse station data: {0}")]
    Parsing(#[from] serde_json::Error),
}
```

### Exercise 3: Module Reorganization
**Current Issue**: The `main.rs` file contains both the application entry point and the main event loop logic.

**Task**: Extract the event loop into a separate module:
1. Create `src/event_loop.rs`
2. Move the `run_app` and `worker_loop` functions there
3. Update imports appropriately
4. Make the module interface clean with proper `pub` declarations

**Learning Goal**: Practice separating concerns and designing module interfaces.

## Key Takeaways

1. **Module Organization**: Group related functionality together, keep modules focused on single responsibilities
2. **Dependency Management**: Use feature flags to include only what you need
3. **Visibility**: Default to private, only make things `pub` when other modules need them
4. **Import Strategy**: Use `crate::` for internal modules, group related imports
5. **Multiple Binaries**: You can have different entry points for different use cases

## Next Steps

In the next lesson, we'll explore how Rust's ownership system works in practice with async code, using examples from the AppController and audio player implementations.

The key insight is that Rust's module system isn't just about organization—it enforces good design by making dependencies explicit and encouraging loose coupling between components.