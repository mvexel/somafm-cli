use anyhow::Result;
use log::{debug};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Playlist {
    pub url: String,
    pub format: String,
    pub quality: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Track {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(rename = "albumArt", default)]
    pub album_art: String,
    #[serde(deserialize_with = "deserialize_date")]
    pub date: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TracksResponse {
    pub id: String,
    pub songs: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Station {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(deserialize_with = "deserialize_listeners")]
    pub listeners: u32,
    #[serde(default)]
    pub image: String,
    #[serde(rename = "lastPlaying", default)]
    pub last_playing: String,
    #[serde(deserialize_with = "deserialize_genre")]
    pub genre: Vec<String>,
    #[serde(default)]
    pub dj: String,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
}


#[derive(Debug, Deserialize)]
struct ChannelsResponse {
    channels: Vec<Station>,
}

fn deserialize_genre<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }

    match StringOrVec::deserialize(deserializer) {
        Ok(StringOrVec::String(s)) => Ok(vec![s]),
        Ok(StringOrVec::Vec(v)) => Ok(v),
        Err(_) => Ok(vec![]), // Default to empty vec on any error
    }
}

fn deserialize_listeners<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(u32),
    }

    match StringOrNumber::deserialize(deserializer) {
        Ok(StringOrNumber::String(s)) => s.parse().map_err(serde::de::Error::custom),
        Ok(StringOrNumber::Number(n)) => Ok(n),
        Err(_) => Ok(0), // Default to 0 on any error
    }
}

fn deserialize_date<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(u64),
    }

    match StringOrNumber::deserialize(deserializer) {
        Ok(StringOrNumber::String(s)) => s.parse().map_err(serde::de::Error::custom),
        Ok(StringOrNumber::Number(n)) => Ok(n),
        Err(_) => Ok(0), // Default to 0 on any error
    }
}

pub struct SomaFMClient {
    client: reqwest::Client,
}

impl SomaFMClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    pub async fn get_stations(&self) -> Result<Vec<Station>> {
        let response = self
            .client
            .get("https://somafm.com/channels.json")
            .send()
            .await?;

        let mut channels_response: ChannelsResponse = response.json().await?;

        // Sort by listener count (popularity)
        channels_response
            .channels
            .sort_by(|a, b| b.listeners.cmp(&a.listeners));

        Ok(channels_response.channels)
    }

    pub fn get_stream_url(&self, station: &Station) -> Option<String> {
        // Find the highest quality MP3 stream
        let mut best_playlist = None;

        for playlist in &station.playlists {
            if playlist.format == "mp3" {
                match playlist.quality.as_str() {
                    "highest" => return Some(playlist.url.clone()),
                    "high" if best_playlist.is_none() => best_playlist = Some(&playlist.url),
                    _ if best_playlist.is_none() => best_playlist = Some(&playlist.url),
                    _ => {}
                }
            }
        }

        best_playlist.cloned()
    }

    pub async fn get_current_tracks(&self, station_id: &str) -> Result<Vec<Track>> {
        let url = format!("https://somafm.com/songs/{}.json", station_id);

        let response = self
            .client
            .get(&url)
            .send()
            .await?;

        let tracks_response: TracksResponse = response.json().await?;
        Ok(tracks_response.songs)
    }

    pub async fn get_current_track(&self, station_id: &str) -> Result<Option<Track>> {
        let tracks = self.get_current_tracks(station_id).await?;

        debug!("Fetched {} tracks for station {}", tracks.len(), station_id);

        if let Some(first_track) = tracks.first() {
            debug!("First track: {} - {}", first_track.artist, first_track.title);
        }

        // The first track in the list is typically the most recently played
        // which should be the currently playing track
        Ok(tracks.into_iter().next())
    }
}