// Example of using the improved audio player
// This demonstrates the new API features and would be typically used by the main application

use crate::audio::{SimpleAudioPlayer, PlayerEvent};
use anyhow::Result;
use log::info;

#[allow(dead_code)]
pub async fn demo_improved_audio_player() -> Result<()> {
    info!("Demonstrating improved audio player features");
    
    // Create player
    let player = SimpleAudioPlayer::new()?;
    
    // Enable auto-reconnect
    player.set_auto_reconnect(true);
    
    // Get event receiver for monitoring
    let mut event_receiver = player.event_receiver();
    
    // Spawn task to monitor events
    let event_monitor = tokio::spawn(async move {
        while event_receiver.changed().await.is_ok() {
            let event = event_receiver.borrow().clone();
            match event {
                PlayerEvent::Started(url) => info!("🎵 Started playing: {}", url),
                PlayerEvent::Stopped => info!("⏹️  Playback stopped"),
                PlayerEvent::Paused => info!("⏸️  Playback paused"),
                PlayerEvent::Resumed => info!("▶️  Playback resumed"),
                PlayerEvent::Error(msg) => info!("❌ Error: {}", msg),
                PlayerEvent::StreamConnected => info!("🌐 Connected to stream"),
                PlayerEvent::BufferProgress(bytes) => {
                    if bytes > 0 && bytes % (64 * 1024) == 0 {
                        info!("📊 Buffer: {}KB", bytes / 1024);
                    }
                }
            }
        }
    });
    
    // Example usage
    let stream_url = "https://ice1.somafm.com/groovesalad-256-mp3";
    
    // Start playback
    player.play(stream_url.to_string())?;
    
    // Check status
    info!("Current URL: {:?}", player.current_url());
    info!("Playback state: {:?}", player.playback_state());
    info!("Is playing: {}", player.is_playing());
    
    // Wait a bit, then pause
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    player.pause()?;
    
    // Wait, then resume
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    player.resume()?;
    
    // Wait, then stop
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    player.stop()?;
    
    // Graceful shutdown
    player.shutdown()?;
    
    // Cancel event monitor
    event_monitor.abort();
    
    info!("Demo completed successfully");
    Ok(())
}