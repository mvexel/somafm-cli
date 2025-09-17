use crate::{api::{Station, Track}, audio::SimpleAudioPlayer};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

pub struct App {
    pub stations: Vec<Station>,
    pub current_station_index: usize,
    pub audio_player: SimpleAudioPlayer,
    pub list_state: ListState,
    pub should_quit: bool,
    pub current_track: Option<Track>,
}

impl App {
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
        }
    }

    pub fn current_station(&self) -> Option<&Station> {
        self.stations.get(self.current_station_index)
    }

    pub fn select_station(&mut self, index: usize) {
        if index < self.stations.len() {
            self.current_station_index = index;
            self.list_state.select(Some(index));
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
    }
}

pub fn render_ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(5),  // Header with station info
            Constraint::Min(10),    // Main station browser
            Constraint::Length(3),  // Footer
        ])
        .split(f.area());

    // Header with current station info
    render_header_with_current_station(f, chunks[0], app);

    // Main station browser (full width)
    render_station_list(f, chunks[1], app);

    // Footer
    render_footer(f, chunks[2]);
}

fn render_header_with_current_station(f: &mut Frame, area: Rect, app: &App) {
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

fn render_station_list(f: &mut Frame, area: Rect, app: &App) {
    // Calculate dynamic column widths based on available space
    let available_width = area.width.saturating_sub(4) as usize; // Account for borders and padding
    let prefix_width = 4; // " > " or "   "
    let listeners_width = 6; // " 1339 "
    let separators_width = 6; // " │ " * 2 separators
    let min_genre_width = 8;
    let min_description_width = 20;

    // Calculate remaining width for dynamic columns
    let fixed_width = prefix_width + listeners_width + separators_width + min_genre_width + min_description_width;
    let remaining_width = available_width.saturating_sub(fixed_width);

    // Distribute remaining width: 30% to station name, 20% to genre, 50% to description
    let station_width = (remaining_width * 3 / 10).max(15);
    let genre_width = min_genre_width + (remaining_width * 2 / 10);
    let description_width = min_description_width + (remaining_width * 5 / 10);

    let items: Vec<ListItem> = app
        .stations
        .iter()
        .enumerate()
        .map(|(i, station)| {
            let is_selected = i == app.current_station_index;

            // Create the station display with enhanced formatting
            let style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let genre = station.genre.join(", ");
            let genre_display = if genre.is_empty() { "Various" } else { &genre };

            // Enhanced display format with dynamic widths
            let content = if is_selected {
                format!(
                    " > {:<width1$} │ {:>5} │ {:<width2$} │ {} ",
                    truncate_string(&station.title, station_width),
                    format!("{}", station.listeners),
                    truncate_string(genre_display, genre_width),
                    truncate_string(&station.description, description_width),
                    width1 = station_width,
                    width2 = genre_width
                )
            } else {
                format!(
                    "   {:<width1$} │ {:>5} │ {:<width2$} │ {} ",
                    truncate_string(&station.title, station_width),
                    format!("{}", station.listeners),
                    truncate_string(genre_display, genre_width),
                    truncate_string(&station.description, description_width),
                    width1 = station_width,
                    width2 = genre_width
                )
            };

            ListItem::new(content).style(style)
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
        );

    f.render_stateful_widget(list, area, &mut app.list_state.clone());
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        format!("{:<width$}", s, width = max_len)
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
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