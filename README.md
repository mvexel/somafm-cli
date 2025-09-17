# Soma FM TUI

A retro-styled terminal user interface for streaming Soma FM radio stations, built with Rust and inspired by the early 2000s aesthetic of the original Soma FM website.

## Features

- 🎵 Browse and play Soma FM stations
- 📊 Stations sorted by popularity (listener count)
- 🎨 ASCII art representation of station images
- ⏯️ Play/pause controls
- 🎼 Real-time now playing information
- 🌈 Retro early 2000s styling with ratatui
- 🔊 High-quality audio streaming with rodio

## Installation

### Prerequisites

- Rust (install via [mise](https://mise.jdx.dev/) or [rustup](https://rustup.rs/))
- Audio system libraries (ALSA on Linux, CoreAudio on macOS, WASAPI on Windows)

### Using mise

```bash
# Install mise if you haven't already
curl https://mise.jdx.dev/install.sh | sh

# Clone and enter the project
git clone <repository-url>
cd somafm-cli

# Install Rust via mise
mise install

# Build and run
cargo run
```

### Manual Installation

```bash
# Install Rust via rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone <repository-url>
cd somafm-cli
cargo build --release

# Run
./target/release/somafm-tui
```

## Usage

### Controls

- `↑/↓` - Navigate station list
- `ENTER` - Play selected station
- `SPACE` - Pause/Resume playback
- `R` - Refresh station data
- `1-9` - Jump to station by number
- `Q` or `ESC` - Quit application

### Interface Layout

```
┌─────────────────────────────────────────────────────────────────┐
│                  🎵 SOMA FM TUI - Underground Radio             │
├─────────────────────────┬───────────────────────────────────────┤
│  ┌─ 📻 Current Station ─┐ │ ┌─ 📻 Stations (by popularity) ──────┐ │
│  │                      │ │ │  1 │ Groove Salad    │  1,352 │... │ │
│  │    ASCII ART HERE    │ │ │► 2 │ Space Station   │    847 │... │ │
│  │                      │ │ │  3 │ Lush            │    632 │... │ │
│  └──────────────────────┘ │ │  ...                             │ │
│  ┌─ 📊 Station Info ─────┐ │ └─────────────────────────────────────┘ │
│  │ 🎼 Genre: Ambient      │ │                                   │ │
│  │ 👥 Listeners: 1,352    │ │                                   │ │
│  │ 🎵 Now Playing:        │ │                                   │ │
│  │ Artist - Song Title    │ │                                   │ │
│  │ Status: ▶️ Playing     │ │                                   │ │
│  └────────────────────────┘ │                                   │ │
└─────────────────────────┴───┴─────────────────────────────────────┘
│     Controls: ↑/↓ Select • ENTER Play • SPACE Pause • Q Quit     │
└─────────────────────────────────────────────────────────────────┘
```

## Architecture

### Core Components

- **`api.rs`** - Soma FM API client for fetching stations and metadata
- **`ascii_art.rs`** - Image to ASCII art conversion
- **`audio.rs`** - Audio streaming with rodio
- **`ui.rs`** - Terminal UI rendering with ratatui
- **`main.rs`** - Application entry point and event loop

### Dependencies

- **`ratatui`** - Terminal UI framework
- **`crossterm`** - Cross-platform terminal handling
- **`tokio`** - Async runtime
- **`reqwest`** - HTTP client for API calls
- **`serde`** - JSON serialization
- **`image`** - Image processing for ASCII art
- **`rodio`** - Audio playback
- **`anyhow`** - Error handling

## API Integration

Uses the official Soma FM JSON API:
- **Stations**: `https://somafm.com/channels.json`

## Early 2000s Aesthetic

The interface recreates the retro Soma FM experience with:
- ASCII art station logos
- Terminal-based interface reminiscent of early web browsers
- Retro color schemes (cyan, magenta, green, yellow)
- Simple, functional layout with borders and panels
- Monospace fonts throughout
- Nostalgic UI elements and symbols

## Development

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run with debug output
RUST_LOG=debug cargo run
```

### Code Structure

```
src/
├── main.rs        # Entry point and main loop
├── api.rs         # Soma FM API client
├── ascii_art.rs   # ASCII art conversion
├── audio.rs       # Audio streaming
└── ui.rs          # Terminal UI rendering
```

## Troubleshooting

### Audio Issues

**No audio output:**
- Ensure your system has working audio drivers
- Check volume levels
- Try different stations (some may have connectivity issues)

**Build errors with audio dependencies:**
- Install system audio libraries:
  - **Linux**: `sudo apt install libasound2-dev` (Ubuntu/Debian)
  - **macOS**: Audio libraries are included with Xcode Command Line Tools
  - **Windows**: Audio support is included with Windows SDK

### Network Issues

**Stations won't load:**
- Check internet connectivity
- Some corporate networks may block streaming audio
- Try refreshing with `R` key

### Terminal Issues

**Display corruption:**
- Ensure terminal supports ANSI colors and Unicode
- Try resizing terminal window
- Use a modern terminal emulator (iTerm2, Windows Terminal, etc.)

## Contributing

This is a demonstration project showcasing Rust TUI development with ratatui. Feel free to fork and extend with additional features like:

- Playlist management
- Favorites system
- Audio equalizer
- Recording functionality
- Keyboard shortcuts customization

## License

MIT License - see LICENSE file for details.

## Acknowledgments

- [Soma FM](https://somafm.com) for providing the excellent underground radio service
- [ratatui](https://github.com/ratatui-org/ratatui) for the fantastic TUI framework
- The Rust community for amazing audio and networking libraries