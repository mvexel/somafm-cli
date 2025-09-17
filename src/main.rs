mod api;
mod app;
mod audio;
mod ui;

use anyhow::Result;
use app::AppController;
use audio::SimpleAudioPlayer;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;
use std::time::{Duration, Instant};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // Set up panic handler to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize audio player
    let audio_player = SimpleAudioPlayer::new()?;

    // Initialize app controller
    let mut app_controller = AppController::new(audio_player);
    app_controller.initialize().await?;

    // Run the main loop
    let res = run_app(&mut terminal, &mut app_controller).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    let _ = res;

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app_controller: &mut AppController,
) -> Result<()> {
    let mut last_track_update = Instant::now();
    let track_update_interval = Duration::from_secs(10); // Update every 10 seconds for responsiveness

    loop {
        // Render UI
        if let Err(_e) = terminal.draw(|f| ui::render_ui(f, &app_controller.ui_app)) {
            break;
        }

        // Handle input with shorter timeout for better responsiveness
        if event::poll(Duration::from_millis(50))? {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if app_controller.handle_key_event(key.code).await? {
                        break; // Quit was requested
                    }
                }
                Ok(Event::Resize(_, _)) => {
                    // Terminal was resized, UI will automatically adjust on next render
                }
                Ok(_) => {} // Ignore other events
                Err(_) => {} // Ignore read errors
            }
        }

        // Update current track periodically for responsive updates
        if last_track_update.elapsed() >= track_update_interval {
            let _ = app_controller.update_current_track_for_selected_station().await;
            last_track_update = Instant::now();
        }

        // Small delay to prevent high CPU usage but keep responsive
        sleep(Duration::from_millis(16)).await; // ~60 FPS

        // Check if we should quit
        if app_controller.should_quit() {
            break;
        }
    }

    Ok(())
}