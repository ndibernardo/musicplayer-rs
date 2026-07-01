use std::time::Duration;

use crate::domain::player::Volume;
use crate::domain::track::TrackPath;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("failed to open audio output: {0}")]
    Device(String),
    #[error("failed to decode {0}: {1}")]
    Decode(String, String),
}

/// Drives the underlying audio hardware. Implemented by `RodioAudioBackend`.
///
/// Intentionally not `Send` — implementations may hold OS audio handles tied
/// to the thread that created them. The player thread creates its own instance.
pub trait AudioBackend {
    fn play(&mut self, path: &TrackPath) -> Result<(), AudioError>;
    fn pause(&mut self);
    fn resume(&mut self);
    fn stop(&mut self);
    fn set_volume(&mut self, volume: Volume);
    fn is_playing(&self) -> bool;
    fn is_paused(&self) -> bool;
    fn position(&self) -> Duration;
}
