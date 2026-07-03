use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::library::track::Track;
use crate::library::track::TrackId;
use crate::library::track::TrackPath;

#[cfg(feature = "ui")]
pub mod rodio;

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
    Playing {
        track: TrackId,
        position: SeekPosition,
    },
    Paused {
        track: TrackId,
        position: SeekPosition,
    },
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
            Self::Stopped => None,
            Self::Playing { track, .. } => Some(*track),
            Self::Paused { track, .. } => Some(*track),
        }
    }
}

/// Commands sent to the audio engine thread.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerCommand {
    Play(Track),
    Pause,
    Resume,
    Stop,
    Seek(SeekPosition),
    SetVolume(Volume),
    Next,
    Previous,
}

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

/// Cloneable handle to the background player thread.
///
/// Each clone shares the same command channel — any clone can send commands.
#[derive(Clone)]
pub struct PlayerHandle {
    command_tx: Sender<PlayerCommand>,
}

impl PlayerHandle {
    /// Spawns a background thread, creates the backend with `make_backend`,
    /// and runs the player loop.  `on_state` is called from that thread on
    /// every state transition and on each 250 ms position tick while playing.
    pub fn launch<B, F>(
        make_backend: impl FnOnce() -> Result<B, AudioError> + Send + 'static,
        on_state: F,
    ) -> Self
    where
        B: AudioBackend + 'static,
        F: Fn(PlaybackState) + Send + 'static,
    {
        let (command_tx, command_rx) = mpsc::channel();
        std::thread::spawn(move || match make_backend() {
            Ok(mut backend) => player_loop(&mut backend, command_rx, on_state),
            Err(e) => eprintln!("audio backend init failed: {e}"),
        });
        Self { command_tx }
    }

    pub fn send(&self, cmd: PlayerCommand) {
        let _ = self.command_tx.send(cmd);
    }
}

fn player_loop<B: AudioBackend, F: Fn(PlaybackState)>(
    backend: &mut B,
    command_rx: Receiver<PlayerCommand>,
    on_state: F,
) {
    let mut current: Option<Track> = None;

    loop {
        match command_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(PlayerCommand::Play(track)) => {
                if let Err(e) = backend.play(&track.path) {
                    eprintln!("playback error: {e}");
                } else {
                    current = Some(track.clone());
                    on_state(PlaybackState::Playing {
                        track: track.id,
                        position: SeekPosition::zero(),
                    });
                }
            }
            Ok(PlayerCommand::Pause) => {
                backend.pause();
                if let Some(ref t) = current {
                    on_state(PlaybackState::Paused {
                        track: t.id,
                        position: SeekPosition::from_millis(backend.position().as_millis() as u64),
                    });
                }
            }
            Ok(PlayerCommand::Resume) => {
                backend.resume();
                if let Some(ref t) = current {
                    on_state(PlaybackState::Playing {
                        track: t.id,
                        position: SeekPosition::from_millis(backend.position().as_millis() as u64),
                    });
                }
            }
            Ok(PlayerCommand::Stop) => {
                backend.stop();
                current = None;
                on_state(PlaybackState::Stopped);
            }
            Ok(PlayerCommand::SetVolume(v)) => {
                backend.set_volume(v);
            }
            Ok(PlayerCommand::Seek(_) | PlayerCommand::Next | PlayerCommand::Previous) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(ref t) = current
                    && backend.is_playing()
                {
                    on_state(PlaybackState::Playing {
                        track: t.id,
                        position: SeekPosition::from_millis(backend.position().as_millis() as u64),
                    });
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::library::track::AlbumTitle;
    use crate::library::track::Artist;
    use crate::library::track::DiscNumber;
    use crate::library::track::Genre;
    use crate::library::track::Title;
    use crate::library::track::Track;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::TrackPath;
    use crate::library::track::Year;

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
        assert!(matches!(
            Volume::new(1.1),
            Err(PlayerError::VolumeOutOfRange(_))
        ));
    }

    #[test]
    fn volume_new_rejects_negative_value() {
        assert!(matches!(
            Volume::new(-0.1),
            Err(PlayerError::VolumeOutOfRange(_))
        ));
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
            track: TrackId::new(1),
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
        let state = PlaybackState::Playing {
            track: id,
            position: SeekPosition::zero(),
        };
        assert_eq!(state.current_track(), Some(id));
    }

    #[test]
    fn playback_state_paused_exposes_current_track() {
        let id = TrackId::new(3);
        let state = PlaybackState::Paused {
            track: id,
            position: SeekPosition::from_secs(42),
        };
        assert_eq!(state.current_track(), Some(id));
    }

    struct MockAudioBackend {
        playing: bool,
        paused: bool,
        volume: f32,
    }

    impl MockAudioBackend {
        fn new() -> Self {
            Self {
                playing: false,
                paused: false,
                volume: 1.0,
            }
        }
    }

    impl AudioBackend for MockAudioBackend {
        fn play(&mut self, _path: &TrackPath) -> Result<(), AudioError> {
            self.playing = true;
            self.paused = false;
            Ok(())
        }
        fn pause(&mut self) {
            self.playing = false;
            self.paused = true;
        }
        fn resume(&mut self) {
            self.playing = true;
            self.paused = false;
        }
        fn stop(&mut self) {
            self.playing = false;
            self.paused = false;
        }
        fn set_volume(&mut self, v: Volume) {
            self.volume = v.value();
        }
        fn is_playing(&self) -> bool {
            self.playing
        }
        fn is_paused(&self) -> bool {
            self.paused
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
    }

    fn julie_and_candy() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/music/geogaddi/julie_and_candy.flac").unwrap(),
            title: Title::new("Julie and Candy"),
            artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Electronic"),
            duration: TrackDuration::from_secs(232),
            track_number: TrackNumber::new(2),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
            art: None,
        }
    }

    fn launch_with_channel() -> (PlayerHandle, mpsc::Receiver<PlaybackState>) {
        let (tx, rx) = mpsc::channel();
        let handle = PlayerHandle::launch(
            || Ok(MockAudioBackend::new()),
            move |s| {
                let _ = tx.send(s);
            },
        );
        (handle, rx)
    }

    /// Drains `rx` until a message matching `pred` arrives (or 2 s elapses).
    fn recv_matching(
        rx: &mpsc::Receiver<PlaybackState>,
        pred: impl Fn(&PlaybackState) -> bool,
    ) -> PlaybackState {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(s) if pred(&s) => return s,
                Ok(_) => {}
                Err(_) => panic!("timed out waiting for expected playback state"),
            }
            if Instant::now() > deadline {
                panic!("deadline exceeded waiting for expected playback state");
            }
        }
    }

    #[test]
    fn player_play_command_transitions_to_playing_state() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(julie_and_candy()));
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        assert!(matches!(s, PlaybackState::Playing { .. }));
    }

    #[test]
    fn player_play_command_reports_correct_track_id() {
        let track = julie_and_candy();
        let expected = track.id;
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(track));
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        assert_eq!(s.current_track(), Some(expected));
    }

    #[test]
    fn player_pause_after_play_transitions_to_paused_state() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(julie_and_candy()));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Pause);
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Paused { .. }));
        assert!(matches!(s, PlaybackState::Paused { .. }));
    }

    #[test]
    fn player_resume_after_pause_transitions_to_playing_state() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(julie_and_candy()));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Pause);
        recv_matching(&rx, |s| matches!(s, PlaybackState::Paused { .. }));
        handle.send(PlayerCommand::Resume);
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        assert!(matches!(s, PlaybackState::Playing { .. }));
    }

    #[test]
    fn player_stop_after_play_transitions_to_stopped_state() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(julie_and_candy()));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Stop);
        let s = recv_matching(&rx, |s| s == &PlaybackState::Stopped);
        assert_eq!(s, PlaybackState::Stopped);
    }

    #[test]
    fn player_stop_clears_current_track() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(julie_and_candy()));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Stop);
        let s = recv_matching(&rx, |s| s == &PlaybackState::Stopped);
        assert_eq!(s.current_track(), None);
    }
}
