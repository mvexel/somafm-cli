mod api;
mod app;
mod audio;
mod audio_demo;
mod ui;
mod actions;

use anyhow::Result;
use app::AppController;
use actions::{Request, Response};
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
use std::time::Duration;
use tokio::time::sleep;
use tokio::sync::mpsc;

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

    // Create channels for background worker
    let (req_tx, req_rx) = mpsc::channel::<Request>(64);
    let (resp_tx, resp_rx) = mpsc::channel::<Response>(64);

    // Spawn background worker task
    tokio::spawn(worker_loop(req_rx, resp_tx));

    // Initialize app controller with request sender
    let mut app_controller = AppController::new(audio_player, req_tx.clone());
    app_controller.initialize().await?; // will enqueue initial loads

    // Run the main loop
    let res = run_app(&mut terminal, &mut app_controller, req_tx, resp_rx).await;

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
    _req_tx: mpsc::Sender<Request>,
    mut resp_rx: mpsc::Receiver<Response>,
) -> Result<()> {
    // Track updates are requested on selection/play with debounce; also light periodic refresh when playing
    let mut last_play_refresh = std::time::Instant::now();
    let play_refresh_interval = Duration::from_secs(5);

    loop {
        // Render UI
        if let Err(_e) = terminal.draw(|f| ui::render_ui(f, &mut app_controller.ui_app)) {
            break;
        }

        // Process any incoming responses without blocking
        loop {
            match resp_rx.try_recv() {
                Ok(resp) => {
                    app_controller.process_response(resp).await?;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
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

        // Light periodic refresh of current track if playing
        if app_controller.ui_app.audio_player.is_playing() {
            if last_play_refresh.elapsed() >= play_refresh_interval {
                if let Some(station) = app_controller.ui_app.current_station() {
                    let _ = _req_tx.try_send(actions::Request::LoadTrackForStation { station_id: station.id.clone() });
                }
                last_play_refresh = std::time::Instant::now();
            }
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

// Background worker: performs API calls and sends responses
async fn worker_loop(mut req_rx: mpsc::Receiver<Request>, resp_tx: mpsc::Sender<Response>) {
    let client = api::SomaFMClient::new();
    while let Some(req) = req_rx.recv().await {
        match req {
            Request::LoadStations => {
                let res = client.get_stations().await;
                let _ = resp_tx.send(Response::StationsLoaded(res)).await;
            }
            Request::LoadTrackForStation { station_id } => {
                let res = client.get_current_track(&station_id).await;
                let _ = resp_tx.send(Response::TrackLoaded { station_id, result: res }).await;
            }
        }
    }
}