use crate::domain::track::TrackId;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlayerError {
    #[error("volume must be between 0.0 and 1.0, got {0}")]
    VolumeOutOfRange(f32),
}

/// Playback volume. Always in [0.0, 1.0].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Volume(f32);

impl Volume {
    /// Returns `Err(VolumeOutOfRange)` if `v` is outside [0.0, 1.0].
    pub fn new(v: f32) -> Result<Self, PlayerError> {
        if !(0.0..=1.0).contains(&v) {
            return Err(PlayerError::VolumeOutOfRange(v));
        }
        Ok(Self(v))
    }

    pub fn value(self) -> f32 {
        self.0
    }

    pub fn silent() -> Self {
        Self(0.0)
    }

    pub fn full() -> Self {
        Self(1.0)
    }
}

/// Playback position within a track. Always non-negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct SeekPosition(std::time::Duration);

impl SeekPosition {
    pub fn from_secs(secs: u64) -> Self {
        Self(std::time::Duration::from_secs(secs))
    }

    pub fn from_millis(millis: u64) -> Self {
        Self(std::time::Duration::from_millis(millis))
    }

    pub fn as_duration(self) -> std::time::Duration {
        self.0
    }

    pub fn as_secs(self) -> u64 {
        self.0.as_secs()
    }

    pub fn zero() -> Self {
        Self(std::time::Duration::ZERO)
    }
}

/// Current state of the audio engine.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackState {
    Stopped,
    Playing { track: TrackId, position: SeekPosition },
    Paused  { track: TrackId, position: SeekPosition },
}

impl PlaybackState {
    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped)
    }

    pub fn is_playing(&self) -> bool {
        matches!(self, Self::Playing { .. })
    }

    pub fn current_track(&self) -> Option<TrackId> {
        match self {
            Self::Stopped                  => None,
            Self::Playing { track, .. }    => Some(*track),
            Self::Paused  { track, .. }    => Some(*track),
        }
    }
}

/// Commands sent to the audio engine thread.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerCommand {
    Play(TrackId),
    Pause,
    Resume,
    Stop,
    Seek(SeekPosition),
    SetVolume(Volume),
    Next,
    Previous,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::track::TrackId;

    #[test]
    fn volume_new_accepts_zero() {
        assert_eq!(Volume::new(0.0).unwrap().value(), 0.0);
    }

    #[test]
    fn volume_new_accepts_one() {
        assert_eq!(Volume::new(1.0).unwrap().value(), 1.0);
    }

    #[test]
    fn volume_new_accepts_midpoint() {
        assert_eq!(Volume::new(0.5).unwrap().value(), 0.5);
    }

    #[test]
    fn volume_new_rejects_value_above_one() {
        assert!(matches!(Volume::new(1.1), Err(PlayerError::VolumeOutOfRange(_))));
    }

    #[test]
    fn volume_new_rejects_negative_value() {
        assert!(matches!(Volume::new(-0.1), Err(PlayerError::VolumeOutOfRange(_))));
    }

    #[test]
    fn volume_silent_is_zero() {
        assert_eq!(Volume::silent().value(), 0.0);
    }

    #[test]
    fn volume_full_is_one() {
        assert_eq!(Volume::full().value(), 1.0);
    }

    #[test]
    fn seek_position_from_secs_round_trips() {
        assert_eq!(SeekPosition::from_secs(90).as_secs(), 90);
    }

    #[test]
    fn seek_position_ordering_reflects_time() {
        assert!(SeekPosition::from_secs(10) < SeekPosition::from_secs(60));
    }

    #[test]
    fn playback_state_stopped_is_stopped() {
        assert!(PlaybackState::Stopped.is_stopped());
    }

    #[test]
    fn playback_state_playing_is_not_stopped() {
        let state = PlaybackState::Playing {
            track:    TrackId::new(1),
            position: SeekPosition::zero(),
        };
        assert!(!state.is_stopped());
        assert!(state.is_playing());
    }

    #[test]
    fn playback_state_stopped_has_no_current_track() {
        assert_eq!(PlaybackState::Stopped.current_track(), None);
    }

    #[test]
    fn playback_state_playing_exposes_current_track() {
        let id = TrackId::new(7);
        let state = PlaybackState::Playing { track: id, position: SeekPosition::zero() };
        assert_eq!(state.current_track(), Some(id));
    }

    #[test]
    fn playback_state_paused_exposes_current_track() {
        let id = TrackId::new(3);
        let state = PlaybackState::Paused { track: id, position: SeekPosition::from_secs(42) };
        assert_eq!(state.current_track(), Some(id));
    }
}
