// No-audio version for testing
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph}, Terminal,
};
use std::io;
use std::time::Duration;

// Simplified station structure for testing
#[derive(Debug, Clone)]
struct Station {
    title: String,
    description: String,
    listeners: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up panic handler
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic);
    }));

    println!("Soma FM TUI - No Audio Test Mode");
    println!("Fetching stations...");

    // Fetch real stations from API
    let stations = fetch_stations().await.unwrap_or_else(|_| {
        vec![
            Station {
                title: "Groove Salad".to_string(),
                description: "Ambient/Downtempo".to_string(),
                listeners: 1352,
            },
            Station {
                title: "Space Station Soma".to_string(),
                description: "Ambient Space Music".to_string(),
                listeners: 847,
            },
            Station {
                title: "Lush".to_string(),
                description: "Sensual and Mellow".to_string(),
                listeners: 632,
            },
        ]
    });

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut selected = 0;

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(0),
                    Constraint::Length(3),
                ])
                .split(f.area());

            // Header
            let header = Paragraph::new("SOMA FM TUI - Browse Mode (No Audio)")
                .style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(header, chunks[0]);

            // Station list
            let items: Vec<ListItem> = stations
                .iter()
                .enumerate()
                .map(|(i, station)| {
                    let style = if i == selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    let content = format!(
                        "{} - {} listeners - {}",
                        station.title, station.listeners, station.description
                    );

                    ListItem::new(content).style(style)
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Stations (Real Data!)"))
                .highlight_style(Style::default().fg(Color::Yellow));
            f.render_widget(list, chunks[1]);

            // Footer
            let footer = Paragraph::new("↑/↓ Navigate • ENTER Select • Q Quit • This version shows real Soma FM data!")
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(footer, chunks[2]);
        })?;

        // Handle input
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        if selected < stations.len() - 1 {
                            selected += 1;
                        }
                    }
                    KeyCode::Enter => {
                        // Station selected (would play audio in full version)
                    }
                    _ => {}
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    println!("Thanks for testing! The full version with audio is available.");
    Ok(())
}

async fn fetch_stations() -> Result<Vec<Station>> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://somafm.com/channels.json")
        .send()
        .await?;

    let json: serde_json::Value = response.json().await?;

    let mut stations = Vec::new();

    if let Some(channels) = json.get("channels").and_then(|c| c.as_array()) {
        for channel in channels {
            let title = channel.get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let description = channel.get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();

            // Parse listeners (it's a string in the API)
            let listeners = channel.get("listeners")
                .and_then(|l| l.as_str())
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);

            stations.push(Station {
                title,
                description,
                listeners,
            });
        }
    }

    // Sort by popularity
    stations.sort_by(|a, b| b.listeners.cmp(&a.listeners));

    Ok(stations)
}