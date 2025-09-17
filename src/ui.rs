use crate::{api::{Station, Track}, audio::SimpleAudioPlayer};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;

// Layout constants for better maintainability
const HEADER_HEIGHT: u16 = 5;
const FOOTER_HEIGHT: u16 = 3;
const STATUS_HEIGHT: u16 = 3;
const MARGIN: u16 = 1;

// Station list layout constants
const HIGHLIGHT_WIDTH: usize = 3; // width of highlight symbol " > "
const LISTENERS_WIDTH: usize = 6; // " 1339 "
const SEPARATORS_WIDTH: usize = 6; // " │ " * 2 separators
const MIN_GENRE_WIDTH: usize = 8;
const MIN_DESCRIPTION_WIDTH: usize = 20;
const MIN_STATION_WIDTH: usize = 15;

pub struct UIState {
    pub stations: Vec<Station>,
    pub current_station_index: usize,
    pub audio_player: SimpleAudioPlayer,
    pub list_state: ListState,
    pub should_quit: bool,
    pub current_track: Option<Track>,
    pub currently_playing_station_id: Option<String>,
    // Status and loading flags
    pub status_message: String,
    pub is_fetching_stations: bool,
    pub is_fetching_track: bool,
    // Cache for rendered station items to improve performance
    station_items_cache: Option<Vec<String>>,
    last_area_width: u16,
}

impl UIState {
    pub fn new(audio_player: SimpleAudioPlayer) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Self {
            stations: Vec::new(),
            current_station_index: 0,
            audio_player,
            list_state,
            should_quit: false,
            current_track: None,
            currently_playing_station_id: None,
            status_message: String::new(),
            is_fetching_stations: false,
            is_fetching_track: false,
            station_items_cache: None,
            last_area_width: 0,
        }
    }

    pub fn current_station(&self) -> Option<&Station> {
        self.stations.get(self.current_station_index)
    }

    pub fn select_station(&mut self, index: usize) {
        if index < self.stations.len() {
            self.current_station_index = index;
            self.list_state.select(Some(index));
            // Do NOT invalidate cache on selection change; selection is rendered via highlight
        }
    }

    pub fn next_station(&mut self) {
        if !self.stations.is_empty() {
            let next = (self.current_station_index + 1) % self.stations.len();
            self.select_station(next);
        }
    }

    pub fn previous_station(&mut self) {
        if !self.stations.is_empty() {
            let prev = if self.current_station_index == 0 {
                self.stations.len() - 1
            } else {
                self.current_station_index - 1
            };
            self.select_station(prev);
        }
    }

    pub fn quit(&mut self) {
        let _ = self.audio_player.stop();
        self.should_quit = true;
        self.currently_playing_station_id = None;
    }

    /// Invalidate the station items cache when stations data changes
    pub fn invalidate_station_cache(&mut self) {
        self.station_items_cache = None;
    }
}

pub fn render_ui(f: &mut Frame, app: &mut UIState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(MARGIN)
        .constraints([
            Constraint::Length(HEADER_HEIGHT),  // Header with station info
            Constraint::Min(10),                // Main station browser
            Constraint::Length(STATUS_HEIGHT),  // Status bar
            Constraint::Length(FOOTER_HEIGHT),  // Footer
        ])
        .split(f.area());

    // Header with current station info
    render_header_with_current_station(f, chunks[0], &*app);

    // Main station browser (full width)
    render_station_list(f, chunks[1], app);

    // Status bar
    render_status(f, chunks[2], app);

    // Footer
    render_footer(f, chunks[3]);
}

fn render_header_with_current_station(f: &mut Frame, area: Rect, app: &UIState) {
    let content = if let Some(station) = app.current_station() {
        let status = if app.audio_player.is_playing() {
            "PLAYING"
        } else if app.audio_player.is_paused() {
            "PAUSED"
        } else {
            "STOPPED"
        };

        let genre = station.genre.join(", ");
        let genre_display = if genre.is_empty() { "Various".to_string() } else { genre };

        vec![
            Line::from(vec![
                Span::styled("AMOS", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                Span::styled(" - your friendly SOMA FM player ", Style::default().fg(Color::Cyan)),
                Span::styled(status, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Station: ", Style::default()),
                Span::styled(&station.title, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(" • ", Style::default().fg(Color::Gray)),
                Span::styled(format!("{} listeners", station.listeners), Style::default().fg(Color::Green)),
                Span::styled(" • ", Style::default().fg(Color::Gray)),
                Span::styled(genre_display, Style::default().fg(Color::Cyan)),
                Span::styled(
                    if !station.dj.is_empty() {
                        format!(" • DJ: {}", station.dj)
                    } else {
                        String::new()
                    },
                    Style::default().fg(Color::Blue)
                ),
            ]),
            Line::from(vec![
                Span::styled("Now Playing: ", Style::default()),
                Span::styled(
                    if let Some(track) = &app.current_track {
                        if track.artist.is_empty() && track.title.is_empty() {
                            "Loading track info...".to_string()
                        } else if track.artist.is_empty() {
                            track.title.clone()
                        } else if track.title.is_empty() {
                            track.artist.clone()
                        } else {
                            format!("{} - {}", track.artist, track.title)
                        }
                    } else {
                        "Loading track info...".to_string()
                    },
                    Style::default().fg(Color::White)
                ),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("SOMA FM TUI ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
                Span::styled("- Loading stations...", Style::default().fg(Color::Gray)),
            ]),
        ]
    };

    let header = Paragraph::new(Text::from(content))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title("Now Playing")
        );

    f.render_widget(header, area);
}

fn render_station_list(f: &mut Frame, area: Rect, app: &mut UIState) {
    // Regenerate cache if width changed or cache is empty
    if app.last_area_width != area.width || app.station_items_cache.is_none() {
        app.last_area_width = area.width;
        let new_rows = create_station_rows(app, area.width);
        app.station_items_cache = Some(new_rows);
    }

    // We can safely unwrap here because the logic above ensures the cache is populated.
    let cached_rows = app.station_items_cache.as_ref().unwrap();

    // Build ListItems that borrow from cached strings and subtly highlight the currently playing row
    let playing_id = app.currently_playing_station_id.as_deref();
    let items: Vec<ListItem> = app
        .stations
        .iter()
        .zip(cached_rows.iter())
        .map(|(station, row)| {
            let item = ListItem::new(row.as_str());
            if Some(station.id.as_str()) == playing_id {
                item.style(Style::default().fg(Color::Green).add_modifier(Modifier::DIM))
            } else {
                item
            }
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(format!("Soma FM Stations ({} total) - Sorted by Popularity", app.stations.len()))
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        )
        .highlight_symbol(" > ");

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn create_station_rows(app: &UIState, area_width: u16) -> Vec<String> {
    let now = Instant::now();
    // Calculate dynamic column widths based on available space
    // Subtract borders/padding (~4) and highlight column width reserved by List
    let available_width = area_width
        .saturating_sub(4)
        .saturating_sub(HIGHLIGHT_WIDTH as u16) as usize; // Account for borders, padding, and highlight column
    let fixed_width = LISTENERS_WIDTH + SEPARATORS_WIDTH + MIN_GENRE_WIDTH + MIN_DESCRIPTION_WIDTH;
    let remaining_width = available_width.saturating_sub(fixed_width);

    // Distribute remaining width: 30% to station name, 20% to genre, 50% to description
    let station_width = (remaining_width * 3 / 10).max(MIN_STATION_WIDTH);
    let genre_width = MIN_GENRE_WIDTH + (remaining_width * 2 / 10);
    let description_width = MIN_DESCRIPTION_WIDTH + (remaining_width * 5 / 10);

    let rows: Vec<String> = app.stations
        .iter()
        .map(|station| {
            let genre = station.genre.join(", ");
            let genre_display = if genre.is_empty() { "Various" } else { &genre };

            // Enhanced display format with dynamic widths (selection handled via List highlight)
            format!(
                "{:<width1$} │ {:>5} │ {:<width2$} │ {} ",
                truncate_string(&station.title, station_width),
                format!("{}", station.listeners),
                truncate_string(genre_display, genre_width),
                truncate_string(&station.description, description_width),
                width1 = station_width,
                width2 = genre_width
            )
        })
        .collect();
    
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open("debug.log") {
        let _ = writeln!(file, "create_station_rows took: {:.2?}", now.elapsed());
    }
    rows
}

fn render_footer(f: &mut Frame, area: Rect) {
    let controls_text = vec![
        Line::from(vec![
            Span::styled("↑/↓ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("Navigate • ", Style::default().fg(Color::White)),
            Span::styled("ENTER ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("Play • ", Style::default().fg(Color::White)),
            Span::styled("SPACE ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("Pause/Resume • ", Style::default().fg(Color::White)),
            Span::styled("R ", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
            Span::styled("Refresh • ", Style::default().fg(Color::White)),
            Span::styled("Q ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled("Quit", Style::default().fg(Color::White)),
        ]),
    ];

    let controls = Paragraph::new(Text::from(controls_text))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray))
                .title("Controls")
        );

    f.render_widget(controls, area);
}

fn render_status(f: &mut Frame, area: Rect, app: &UIState) {
    // Determine status text priority (owned String)
    let text = if app.is_fetching_stations {
        "Fetching stations…".to_string()
    } else if app.is_fetching_track {
        "Fetching track…".to_string()
    } else if app.audio_player.is_playing() {
        match &app.current_track {
            Some(track) if !track.artist.is_empty() || !track.title.is_empty() => {
                let info = if track.artist.is_empty() {
                    track.title.clone()
                } else if track.title.is_empty() {
                    track.artist.clone()
                } else {
                    format!("{} - {}", track.artist, track.title)
                };
                format!("♪ {}", info)
            }
            _ => String::from("Loading track info…"),
        }
    } else if !app.status_message.is_empty() {
        app.status_message.clone()
    } else {
        String::new()
    };

    let status = Paragraph::new(Text::from(Line::from(vec![
        Span::styled(text, Style::default().fg(Color::White)),
    ])))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue))
            .title("Status"),
    );

    f.render_widget(status, area);
}

fn truncate_string(s: &str, max_len: usize) -> String {
    // Char-aware truncation to avoid breaking UTF-8 boundaries
    let mut result = String::with_capacity(max_len);
    let mut count = 0usize;
    for ch in s.chars() {
        let ch_len = 1; // approximate width; for simplicity treat each char as width 1
        if count + ch_len > max_len {
            break;
        }
        result.push(ch);
        count += ch_len;
    }

    if result.chars().count() < s.chars().count() {
        // Ensure space for ellipsis if truncated
        let ellipsis = "...";
        let mut trimmed = String::new();
        let mut used = 0usize;
        for ch in result.chars() {
            if used + 3 > max_len { break; }
            trimmed.push(ch);
            used += 1;
        }
        format!("{:<width$}", format!("{}{}", trimmed, ellipsis), width = max_len)
    } else {
        format!("{:<width$}", result, width = max_len)
    }
}