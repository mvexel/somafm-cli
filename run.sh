#!/bin/bash
# Simple launcher script for Soma FM TUI

echo "🎵 Starting Soma FM TUI..."
echo "Make sure your volume is up and enjoy the underground radio experience!"
echo ""
echo "Controls:"
echo "  ↑/↓ - Navigate stations"
echo "  ENTER - Play station"
echo "  SPACE - Pause/Resume"
echo "  R - Refresh"
echo "  Q - Quit"
echo ""
echo "Note: If you see weird characters, your terminal may not support all features."
echo "Try a different terminal or resize the window if needed."
echo ""

# Set terminal to a good state
export TERM=xterm-256color

# Build and run the application
if cargo run --bin somafm-tui; then
    echo ""
    echo "Thanks for listening to Soma FM! 🎵"
else
    echo ""
    echo "❌ Full version failed. This might be due to:"
    echo "   • Audio device not available"
    echo "   • Audio drivers not configured"
    echo "   • Permissions issue"
    echo ""
    echo "🔧 Try these alternatives:"
    echo "   1. Test version (no audio):  ./test.sh"
    echo "   2. Check audio settings:     System Preferences > Sound"
    echo "   3. Terminal compatibility:   Try iTerm2 or resize window"
    echo ""
    echo "The test version works without audio and shows real Soma FM data!"
fi

# Restore terminal
stty sane 2>/dev/null || true