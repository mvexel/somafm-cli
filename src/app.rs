use anyhow::Result;
use crossterm::event::KeyCode;
use log::debug;
use tokio::sync::mpsc;

use crate::{
    api::SomaFMClient,
    audio::SimpleAudioPlayer,
    ui::UIState as UIApp,
};
use crate::actions::{Request, Response};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct AppController {
    pub ui_app: UIApp,
    pub client: SomaFMClient,
    req_tx: mpsc::Sender<Request>,
    last_track_req: HashMap<String, Instant>,
}

impl AppController {
    pub fn new(audio_player: SimpleAudioPlayer, req_tx: mpsc::Sender<Request>) -> Self {
        Self { ui_app: UIApp::new(audio_player), client: SomaFMClient::new(), req_tx, last_track_req: HashMap::new() }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // Request stations in background
        self.ui_app.is_fetching_stations = true;
        let _ = self.req_tx.try_send(Request::LoadStations);
        Ok(())
    }

    pub async fn load_stations(&mut self) -> Result<()> {
        // Kept for refresh via 'r' to align with behavior; send request instead
        self.ui_app.is_fetching_stations = true;
        let _ = self.req_tx.try_send(Request::LoadStations);
        Ok(())
    }


    pub async fn handle_key_event(&mut self, key_code: KeyCode) -> Result<bool> {
        match key_code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('Q') => {
                self.ui_app.quit();
                return Ok(true);
            }
            KeyCode::Up => {
                self.ui_app.previous_station();
                self.maybe_request_track_for_selected();
            }
            KeyCode::Down => {
                self.ui_app.next_station();
                self.maybe_request_track_for_selected();
            }
            KeyCode::Enter => {
                self.play_current_station().await?;
            }
            KeyCode::Char(' ') => {
                self.toggle_playback().await?;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                let _ = self.load_stations().await;
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if self.select_station_by_number(c)? {
                    self.maybe_request_track_for_selected();
                }
            }
            _ => {}
        }
        Ok(false)
    }

    async fn play_current_station(&mut self) -> Result<()> {
        if let Some(station) = self.ui_app.current_station() {
            // Clone needed data to avoid holding borrow across awaits/mut operations
            let station_id = station.id.clone();
            let stream_url = self.client.get_stream_url(station);

            // If already playing this station, do nothing
            if self.ui_app.audio_player.is_playing() {
                if let Some(current) = &self.ui_app.currently_playing_station_id {
                    if current == &station_id {
                        return Ok(());
                    }
                }
            }

            // Request track info asynchronously (set flag before borrow ends)
            self.ui_app.is_fetching_track = true;
            let _ = self.req_tx.try_send(Request::LoadTrackForStation { station_id: station_id.clone() });

            if let Some(stream_url) = stream_url {
                match self.ui_app.audio_player.play(stream_url) {
                    Ok(_) => {
                        // Mark which station is now playing
                        self.ui_app.currently_playing_station_id = Some(station_id);
                    }
                    Err(_) => {}
                }
            }
        }
        Ok(())
    }

    async fn toggle_playback(&mut self) -> Result<()> {
        if self.ui_app.audio_player.is_playing() {
            let _ = self.ui_app.audio_player.pause();
        } else {
            let _ = self.ui_app.audio_player.resume();
        }
        Ok(())
    }

    fn select_station_by_number(&mut self, digit: char) -> Result<bool> {
        let index = digit.to_digit(10).unwrap() as usize;
        if index > 0 && index <= self.ui_app.stations.len() {
            let new_index = index - 1;
            if new_index != self.ui_app.current_station_index {
                self.ui_app.select_station(new_index);
                return Ok(true); // Station changed, caller should update track history
            }
        }
        Ok(false) // No station change
    }

    fn maybe_request_track_for_selected(&mut self) {
        const DEBOUNCE_MS: u64 = 2000; // 2s per-station debounce
        if let Some(station_id) = self.ui_app.current_station().map(|s| s.id.clone()) {
            // Only fetch on selection if either nothing is playing (and we want to show selection's track),
            // or if the selection equals the currently playing station. Otherwise skip.
            if !self.ui_app.audio_player.is_playing() {
                return;
            } else if let Some(current) = &self.ui_app.currently_playing_station_id {
                if current != &station_id {
                    return; // keep showing current track; don't fetch for hovered station
                }
            }
            let now = Instant::now();
            let should_send = match self.last_track_req.get(&station_id) {
                Some(last) => now.duration_since(*last) >= Duration::from_millis(DEBOUNCE_MS),
                None => true,
            };
            if should_send {
                self.ui_app.is_fetching_track = true;
                if self.req_tx.try_send(Request::LoadTrackForStation { station_id: station_id.clone() }).is_ok() {
                    self.last_track_req.insert(station_id, now);
                } else {
                    // sending failed; don't leave the UI in fetching state
                    self.ui_app.is_fetching_track = false;
                }
            }
        }
    }


    pub fn should_quit(&self) -> bool {
        self.ui_app.should_quit
    }

    pub async fn process_response(&mut self, resp: Response) -> Result<()> {
        match resp {
            Response::StationsLoaded(res) => match res {
                Ok(stations) => {
                    self.ui_app.stations = stations;
                    self.ui_app.invalidate_station_cache();
                    if !self.ui_app.stations.is_empty() {
                        self.ui_app.select_station(0);
                    }
                    self.ui_app.is_fetching_stations = false;
                }
                Err(_e) => {
                    self.ui_app.is_fetching_stations = false;
                    // TODO: surface error in UI
                }
            },
            Response::TrackLoaded { station_id, result } => match result {
                Ok(track) => {
                    // Only update UI if this track belongs to the currently playing station,
                    // or if nothing is playing and the currently selected station matches.
                    let apply = if let Some(current_playing) = &self.ui_app.currently_playing_station_id {
                        &station_id == current_playing
                    } else if let Some(selected) = self.ui_app.current_station().map(|s| s.id.clone()) {
                        station_id == selected
                    } else {
                        false
                    };
                    if apply {
                        debug!("Updating current_track in ui_app: {:?}", track);
                        self.ui_app.current_track = track;
                    }
                    self.ui_app.is_fetching_track = false;
                }
                Err(_e) => {
                    self.ui_app.is_fetching_track = false;
                    // keep previous track on error
                }
            },
        }
        Ok(())
    }
}