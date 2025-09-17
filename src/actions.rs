//! Actions and messages for async app operations
use crate::api::{Station, Track};
use anyhow::Error;

// Requests from UI/controller to the worker
#[derive(Debug, Clone)]
pub enum Request {
    LoadStations,
    LoadTrackForStation { station_id: String },
}

// Responses from worker back to UI/controller
#[derive(Debug)]
pub enum Response {
    StationsLoaded(Result<Vec<Station>, Error>),
    TrackLoaded { station_id: String, result: Result<Option<Track>, Error> },
}
