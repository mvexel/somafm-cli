use anyhow::Result;
use crossterm::event::KeyCode;
use log::debug;

use crate::{
    api::SomaFMClient,
    audio::SimpleAudioPlayer,
    ui::App as UIApp,
};

pub struct AppController {
    pub ui_app: UIApp,
    pub client: SomaFMClient,
}

impl AppController {
    pub fn new(audio_player: SimpleAudioPlayer) -> Self {
        Self {
            ui_app: UIApp::new(audio_player),
            client: SomaFMClient::new(),
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        self.load_stations().await?;
        // Load initial track info for the first station
        if let Some(station_id) = self.ui_app.current_station().map(|s| s.id.clone()) {
            let _ = self.update_current_track(&station_id).await;
        }
        Ok(())
    }

    pub async fn load_stations(&mut self) -> Result<()> {
        self.ui_app.stations = self.client.get_stations().await?;

        if !self.ui_app.stations.is_empty() {
            self.ui_app.select_station(0);
        }

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
                let _ = self.update_current_track_for_selected_station().await;
            }
            KeyCode::Down => {
                self.ui_app.next_station();
                let _ = self.update_current_track_for_selected_station().await;
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
                    let _ = self.update_current_track_for_selected_station().await;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    async fn play_current_station(&mut self) -> Result<()> {
        if let Some(station) = self.ui_app.current_station() {
            if let Some(stream_url) = self.client.get_stream_url(station) {
                match self.ui_app.audio_player.play(stream_url) {
                    Ok(_) => {
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

    pub async fn update_current_track(&mut self, station_id: &str) -> Result<()> {
        // println!("DEBUG: Updating track for station: {}", station_id);
        match self.client.get_current_track(station_id).await {
            Ok(track) => {
                debug!("Updating current_track in ui_app: {:?}", track);
                self.ui_app.current_track = track;
            }
            Err(_e) => {
                // println!("DEBUG: Error fetching track: {}", e);
                // Keep existing track on error, don't reset to None
            }
        }
        Ok(())
    }

    pub async fn update_current_track_for_selected_station(&mut self) -> Result<()> {
        if let Some(station_id) = self.ui_app.current_station().map(|s| s.id.clone()) {
            self.update_current_track(&station_id).await?;
        }
        Ok(())
    }


    pub fn should_quit(&self) -> bool {
        self.ui_app.should_quit
    }
}