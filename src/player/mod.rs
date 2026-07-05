use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::library::track::Track;
use crate::library::track::TrackId;
use crate::library::track::TrackPath;
use crate::player::queue::Queue;

pub mod queue;

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
    /// The backend failed to open or decode the track (e.g. missing file, corrupt data).
    Failed {
        track: TrackId,
        error: String,
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
            Self::Failed { track, .. } => Some(*track),
        }
    }

    /// The playback position, or `None` when stopped or failed.
    pub fn position(&self) -> Option<SeekPosition> {
        match self {
            Self::Stopped => None,
            Self::Playing { position, .. } => Some(*position),
            Self::Paused { position, .. } => Some(*position),
            Self::Failed { .. } => None,
        }
    }
}

/// Commands sent to the audio engine thread.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerCommand {
    // Boxed: `Track` is large, and an unboxed variant would bloat every command.
    Play(Box<Track>),
    /// Replaces the queue with `tracks` positioned at `start` and plays it. This
    /// is what `Next`/`Previous` and auto-advance then navigate through.
    PlayQueue {
        tracks: Vec<Track>,
        start: usize,
    },
    /// Appends `tracks` to the end of the current queue without disturbing
    /// what's already playing. If the queue was empty, playback starts from
    /// the first appended track.
    Enqueue(Vec<Track>),
    /// Restores `tracks` at `start`, loaded paused at `position` — used on
    /// startup to reopen where the previous session left off, without resuming.
    RestorePaused {
        tracks: Vec<Track>,
        start: usize,
        position: SeekPosition,
    },
    /// Replaces the queue's track list without disturbing playback: if the
    /// current track is present in `tracks`, its position resumes unchanged;
    /// otherwise playback stops. Used to reconcile the queue after a library
    /// change removes some of its tracks — the player is the sole owner of
    /// the queue, so the UI cannot just drop entries from its own copy.
    SetQueue(Vec<Track>),
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

/// What the audio hardware is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendState {
    Playing,
    Paused,
    /// No source loaded, or the loaded source finished on its own. Also the
    /// state of a backend on which `resume()` was called with nothing to
    /// resume — checked before reporting `Playing`, so a resume after a
    /// decode failure never claims audio that isn't there.
    Idle,
}

/// Drives the underlying audio hardware. Implemented by `RodioAudioBackend`.
///
/// Intentionally not `Send` — implementations may hold OS audio handles tied
/// to the thread that created them. The player thread creates its own instance.
pub trait AudioBackend {
    fn play(&mut self, path: &TrackPath) -> Result<(), AudioError>;
    /// Loads `path` and holds it paused at `position`, with no audible playback,
    /// so a restored session reopens where it left off without resuming.
    fn play_paused(&mut self, path: &TrackPath, position: Duration) -> Result<(), AudioError>;
    /// Moves the play head of the current track to `position`.
    fn seek(&mut self, position: Duration);
    fn pause(&mut self);
    fn resume(&mut self);
    fn stop(&mut self);
    fn set_volume(&mut self, volume: Volume);
    fn state(&self) -> BackendState;
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
    /// and runs the player loop. `on_state` is called from that thread on
    /// every state transition and on each 250 ms position tick while playing.
    /// `on_queue_changed` is called whenever the queue's track list itself
    /// changes (not on cursor-only moves like `Next`/`Previous`) — the player
    /// owns the queue, so this is the only way a caller learns its contents.
    pub fn launch<B, F, G>(
        make_backend: impl FnOnce() -> Result<B, AudioError> + Send + 'static,
        on_state: F,
        on_queue_changed: G,
    ) -> Self
    where
        B: AudioBackend + 'static,
        F: Fn(PlaybackState) + Send + 'static,
        G: Fn(Vec<Track>) + Send + 'static,
    {
        let (command_tx, command_rx) = mpsc::channel();
        std::thread::spawn(move || match make_backend() {
            Ok(mut backend) => player_loop(&mut backend, command_rx, on_state, on_queue_changed),
            Err(e) => tracing::error!("audio backend init failed: {e}"),
        });
        Self { command_tx }
    }

    pub fn send(&self, cmd: PlayerCommand) {
        let _ = self.command_tx.send(cmd);
    }
}

/// Starts `track` on the backend. Emits `Playing` on success or `Failed` on error.
fn play_track<B: AudioBackend, F: Fn(PlaybackState)>(backend: &mut B, track: &Track, on_state: &F) {
    match backend.play(&track.path) {
        Ok(()) => on_state(PlaybackState::Playing {
            track: track.id,
            position: SeekPosition::zero(),
        }),
        Err(e) => on_state(PlaybackState::Failed {
            track: track.id,
            error: e.to_string(),
        }),
    }
}

/// Plays the queue's current track, if any.
fn play_current<B: AudioBackend, F: Fn(PlaybackState)>(
    backend: &mut B,
    queue: &Queue,
    on_state: &F,
) {
    if let Some(track) = queue.current() {
        play_track(backend, track, on_state);
    }
}

fn player_loop<B: AudioBackend, F: Fn(PlaybackState), G: Fn(Vec<Track>)>(
    backend: &mut B,
    command_rx: Receiver<PlayerCommand>,
    on_state: F,
    on_queue_changed: G,
) {
    let mut queue = Queue::empty();
    // True while a track is playing — drives the 250 ms position tick. Set by
    // play commands; cleared by stop, pause, failure, or end-of-queue. Tracked
    // here rather than re-querying `backend.state()` at the loop top to avoid
    // a race where the backend's state changes between the check and the
    // timeout decision.
    let mut ticking = false;

    loop {
        let cmd_opt = if ticking {
            match command_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(cmd) => Some(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match command_rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => break,
            }
        };

        match cmd_opt {
            None => {
                // 250 ms tick: update position or detect natural end-of-track.
                let Some(current_id) = queue.current().map(|t| t.id) else {
                    ticking = false;
                    continue;
                };
                match backend.state() {
                    BackendState::Playing => on_state(PlaybackState::Playing {
                        track: current_id,
                        position: SeekPosition::from_millis(backend.position().as_millis() as u64),
                    }),
                    BackendState::Paused => {
                        // Still paused since the last tick — nothing to report.
                    }
                    BackendState::Idle => {
                        // Track ended on its own: advance, or stop at end of queue.
                        if queue.advance().is_some() {
                            play_current(backend, &queue, &on_state);
                            ticking = matches!(backend.state(), BackendState::Playing);
                        } else {
                            backend.stop();
                            queue = Queue::empty();
                            ticking = false;
                            on_queue_changed(Vec::new());
                            on_state(PlaybackState::Stopped);
                        }
                    }
                }
            }
            Some(PlayerCommand::Play(track)) => {
                queue = Queue::single(*track);
                on_queue_changed(queue.tracks().to_vec());
                play_current(backend, &queue, &on_state);
                ticking = matches!(backend.state(), BackendState::Playing);
            }
            Some(PlayerCommand::PlayQueue { tracks, start }) => {
                queue = Queue::new(tracks, start);
                on_queue_changed(queue.tracks().to_vec());
                play_current(backend, &queue, &on_state);
                ticking = matches!(backend.state(), BackendState::Playing);
            }
            Some(PlayerCommand::Enqueue(tracks)) => {
                let was_empty = queue.is_empty();
                queue.append(tracks);
                on_queue_changed(queue.tracks().to_vec());
                if was_empty {
                    play_current(backend, &queue, &on_state);
                    ticking = matches!(backend.state(), BackendState::Playing);
                }
            }
            Some(PlayerCommand::RestorePaused {
                tracks,
                start,
                position,
            }) => {
                queue = Queue::new(tracks, start);
                on_queue_changed(queue.tracks().to_vec());
                if let Some(track) = queue.current() {
                    match backend.play_paused(&track.path, position.as_duration()) {
                        Ok(()) => on_state(PlaybackState::Paused {
                            track: track.id,
                            position,
                        }),
                        Err(e) => on_state(PlaybackState::Failed {
                            track: track.id,
                            error: e.to_string(),
                        }),
                    }
                }
                ticking = false;
            }
            Some(PlayerCommand::SetQueue(tracks)) => {
                let current_id = queue.current().map(|t| t.id);
                let survives = current_id.is_some_and(|id| tracks.iter().any(|t| t.id == id));
                if current_id.is_some() && !survives {
                    // The playing/paused track was pruned out from under us —
                    // the safe, unsurprising behaviour is to stop rather than
                    // silently continue on (or jump to) a track the user
                    // didn't choose.
                    backend.stop();
                    queue = Queue::empty();
                    ticking = false;
                    on_queue_changed(Vec::new());
                    on_state(PlaybackState::Stopped);
                } else {
                    let start = current_id
                        .and_then(|id| tracks.iter().position(|t| t.id == id))
                        .unwrap_or(0);
                    queue = Queue::new(tracks, start);
                    on_queue_changed(queue.tracks().to_vec());
                }
            }
            Some(PlayerCommand::Next) => {
                if queue.advance().is_some() {
                    play_current(backend, &queue, &on_state);
                    ticking = matches!(backend.state(), BackendState::Playing);
                }
            }
            Some(PlayerCommand::Previous) => {
                if queue.rewind().is_some() {
                    play_current(backend, &queue, &on_state);
                    ticking = matches!(backend.state(), BackendState::Playing);
                }
            }
            Some(PlayerCommand::Pause) => {
                backend.pause();
                ticking = false;
                if let Some(t) = queue.current() {
                    on_state(PlaybackState::Paused {
                        track: t.id,
                        position: SeekPosition::from_millis(backend.position().as_millis() as u64),
                    });
                }
            }
            Some(PlayerCommand::Resume) => {
                backend.resume();
                // Only report Playing when the backend actually has something to
                // play — resuming with no loaded source (e.g. right after a
                // decode failure) must not claim audio that isn't there.
                let now_playing = matches!(backend.state(), BackendState::Playing);
                ticking = now_playing;
                if now_playing && let Some(t) = queue.current() {
                    on_state(PlaybackState::Playing {
                        track: t.id,
                        position: SeekPosition::from_millis(backend.position().as_millis() as u64),
                    });
                }
            }
            Some(PlayerCommand::Stop) => {
                backend.stop();
                queue = Queue::empty();
                ticking = false;
                on_queue_changed(Vec::new());
                on_state(PlaybackState::Stopped);
            }
            Some(PlayerCommand::SetVolume(v)) => {
                backend.set_volume(v);
            }
            Some(PlayerCommand::Seek(position)) => {
                backend.seek(position.as_duration());
                if let Some(track) = queue.current() {
                    // Report the new position immediately, keeping the play/pause
                    // state, rather than waiting for the next tick.
                    let state = if matches!(backend.state(), BackendState::Paused) {
                        PlaybackState::Paused {
                            track: track.id,
                            position,
                        }
                    } else {
                        PlaybackState::Playing {
                            track: track.id,
                            position,
                        }
                    };
                    on_state(state);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    use super::*;
    use crate::library::track::AlbumTitle;
    use crate::library::track::Artist;
    use crate::library::track::Composer;
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
        state: BackendState,
        volume: f32,
        position: Duration,
    }

    impl MockAudioBackend {
        fn new() -> Self {
            Self {
                state: BackendState::Idle,
                volume: 1.0,
                position: Duration::ZERO,
            }
        }
    }

    impl AudioBackend for MockAudioBackend {
        fn play(&mut self, _path: &TrackPath) -> Result<(), AudioError> {
            self.state = BackendState::Playing;
            self.position = Duration::ZERO;
            Ok(())
        }
        fn play_paused(
            &mut self,
            _path: &TrackPath,
            _position: Duration,
        ) -> Result<(), AudioError> {
            self.state = BackendState::Paused;
            Ok(())
        }
        fn pause(&mut self) {
            self.state = BackendState::Paused;
        }
        fn resume(&mut self) {
            self.state = BackendState::Playing;
        }
        fn stop(&mut self) {
            self.state = BackendState::Idle;
        }
        fn set_volume(&mut self, v: Volume) {
            self.volume = v.value();
        }
        fn state(&self) -> BackendState {
            self.state
        }
        fn seek(&mut self, position: Duration) {
            self.position = position;
        }
        fn position(&self) -> Duration {
            self.position
        }
    }

    fn julie_and_candy() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/music/geogaddi/julie_and_candy.flac").unwrap(),
            title: Title::new("Julie and Candy"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Electronic"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(232),
            track_number: TrackNumber::new(2),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
        }
    }

    fn launch_with_channel() -> (PlayerHandle, mpsc::Receiver<PlaybackState>) {
        let (handle, rx, _queue_rx) = launch_with_channels();
        (handle, rx)
    }

    /// Like `launch_with_channel`, but also returns the queue-snapshot receiver
    /// for tests that care about `on_queue_changed`.
    fn launch_with_channels() -> (
        PlayerHandle,
        mpsc::Receiver<PlaybackState>,
        mpsc::Receiver<Vec<Track>>,
    ) {
        let (tx, rx) = mpsc::channel();
        let (queue_tx, queue_rx) = mpsc::channel();
        let handle = PlayerHandle::launch(
            || Ok(MockAudioBackend::new()),
            move |s| {
                let _ = tx.send(s);
            },
            move |tracks| {
                let _ = queue_tx.send(tracks);
            },
        );
        (handle, rx, queue_rx)
    }

    /// Drains `rx` until a message matching `pred` arrives (or 3 s elapses).
    /// Intermediate gaps are tolerated: the player emits only every 250 ms while
    /// playing, and auto-advance lands a full tick after a track ends.
    fn recv_matching(
        rx: &mpsc::Receiver<PlaybackState>,
        pred: impl Fn(&PlaybackState) -> bool,
    ) -> PlaybackState {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(100))
                && pred(&s)
            {
                return s;
            }
            if Instant::now() > deadline {
                panic!("deadline exceeded waiting for expected playback state");
            }
        }
    }

    #[test]
    fn player_play_command_transitions_to_playing_state() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        assert!(matches!(s, PlaybackState::Playing { .. }));
    }

    #[test]
    fn player_play_command_reports_correct_track_id() {
        let track = julie_and_candy();
        let expected = track.id;
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(track)));
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        assert_eq!(s.current_track(), Some(expected));
    }

    #[test]
    fn player_pause_after_play_transitions_to_paused_state() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Pause);
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Paused { .. }));
        assert!(matches!(s, PlaybackState::Paused { .. }));
    }

    #[test]
    fn player_resume_after_pause_transitions_to_playing_state() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
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
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Stop);
        let s = recv_matching(&rx, |s| s == &PlaybackState::Stopped);
        assert_eq!(s, PlaybackState::Stopped);
    }

    #[test]
    fn player_stop_clears_current_track() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Stop);
        let s = recv_matching(&rx, |s| s == &PlaybackState::Stopped);
        assert_eq!(s.current_track(), None);
    }

    fn geogaddi(id: i64, title: &str) -> Track {
        Track {
            id: TrackId::new(id),
            path: TrackPath::new(format!("/music/geogaddi/{id:02}.flac")).unwrap(),
            title: Title::new(title),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Electronic"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(200),
            track_number: TrackNumber::new(id as u32),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
        }
    }

    fn geogaddi_pair() -> Vec<Track> {
        vec![geogaddi(10, "Dawn Chorus"), geogaddi(20, "1969")]
    }

    #[test]
    fn player_play_queue_plays_the_starting_track() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::PlayQueue {
            tracks: geogaddi_pair(),
            start: 1,
        });
        let s = recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(20)));
        assert_eq!(s.current_track(), Some(TrackId::new(20)));
    }

    #[test]
    fn player_next_plays_the_following_track() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::PlayQueue {
            tracks: geogaddi_pair(),
            start: 0,
        });
        recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(10)));
        handle.send(PlayerCommand::Next);
        let s = recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(20)));
        assert_eq!(s.current_track(), Some(TrackId::new(20)));
    }

    #[test]
    fn player_previous_plays_the_prior_track() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::PlayQueue {
            tracks: geogaddi_pair(),
            start: 1,
        });
        recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(20)));
        handle.send(PlayerCommand::Previous);
        let s = recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(10)));
        assert_eq!(s.current_track(), Some(TrackId::new(10)));
    }

    #[test]
    fn player_enqueue_on_empty_queue_starts_playing_the_first_track() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Enqueue(geogaddi_pair()));
        let s = recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(10)));
        assert_eq!(s.current_track(), Some(TrackId::new(10)));
    }

    #[test]
    fn player_enqueue_while_playing_does_not_disturb_the_current_track() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Enqueue(geogaddi_pair()));
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        assert_eq!(s.current_track(), Some(julie_and_candy().id));
    }

    #[test]
    fn player_next_reaches_a_track_appended_via_enqueue() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        handle.send(PlayerCommand::Enqueue(geogaddi_pair()));
        handle.send(PlayerCommand::Next);
        let s = recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(10)));
        assert_eq!(s.current_track(), Some(TrackId::new(10)));
    }

    /// A backend whose playing state a test can flip to simulate a track ending.
    struct FlaggedBackend {
        playing: Arc<AtomicBool>,
    }

    impl AudioBackend for FlaggedBackend {
        fn play(&mut self, _path: &TrackPath) -> Result<(), AudioError> {
            self.playing.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn play_paused(
            &mut self,
            _path: &TrackPath,
            _position: Duration,
        ) -> Result<(), AudioError> {
            self.playing.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn pause(&mut self) {
            self.playing.store(false, Ordering::SeqCst);
        }
        fn resume(&mut self) {
            self.playing.store(true, Ordering::SeqCst);
        }
        fn stop(&mut self) {
            self.playing.store(false, Ordering::SeqCst);
        }
        fn set_volume(&mut self, _v: Volume) {}
        fn seek(&mut self, _position: Duration) {}
        fn state(&self) -> BackendState {
            if self.playing.load(Ordering::SeqCst) {
                BackendState::Playing
            } else {
                BackendState::Idle
            }
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
    }

    /// Launches a player over a `FlaggedBackend`; the returned flag lets the test
    /// simulate the current track finishing by storing `false`.
    fn launch_flagged() -> (PlayerHandle, mpsc::Receiver<PlaybackState>, Arc<AtomicBool>) {
        let flag = Arc::new(AtomicBool::new(false));
        let backend_flag = Arc::clone(&flag);
        let (tx, rx) = mpsc::channel();
        let handle = PlayerHandle::launch(
            move || {
                Ok(FlaggedBackend {
                    playing: backend_flag,
                })
            },
            move |s| {
                let _ = tx.send(s);
            },
            |_tracks| {},
        );
        (handle, rx, flag)
    }

    #[test]
    fn player_auto_advances_when_the_track_ends() {
        let (handle, rx, flag) = launch_flagged();
        handle.send(PlayerCommand::PlayQueue {
            tracks: geogaddi_pair(),
            start: 0,
        });
        recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(10)));

        // The first track finishes on its own.
        flag.store(false, Ordering::SeqCst);

        let s = recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(20)));
        assert_eq!(s.current_track(), Some(TrackId::new(20)));
    }

    #[test]
    fn player_stops_after_the_last_track_ends() {
        let (handle, rx, flag) = launch_flagged();
        handle.send(PlayerCommand::PlayQueue {
            tracks: vec![geogaddi(10, "Dawn Chorus")],
            start: 0,
        });
        recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(10)));

        // The only track finishes: no next, so playback stops.
        flag.store(false, Ordering::SeqCst);

        let s = recv_matching(&rx, |s| s == &PlaybackState::Stopped);
        assert_eq!(s, PlaybackState::Stopped);
    }

    #[test]
    fn player_restore_paused_reports_paused_at_position() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::RestorePaused {
            tracks: geogaddi_pair(),
            start: 1,
            position: SeekPosition::from_secs(87),
        });
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Paused { .. }));
        assert_eq!(s.current_track(), Some(TrackId::new(20)));
        assert_eq!(s.position(), Some(SeekPosition::from_secs(87)));
    }

    #[test]
    fn player_seek_reports_the_new_position_while_playing() {
        let (handle, rx) = launch_with_channel();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));

        handle.send(PlayerCommand::Seek(SeekPosition::from_secs(90)));

        let s = recv_matching(&rx, |s| s.position() == Some(SeekPosition::from_secs(90)));
        assert!(matches!(s, PlaybackState::Playing { .. }));
    }

    #[test]
    fn playback_state_stopped_has_no_position() {
        assert_eq!(PlaybackState::Stopped.position(), None);
    }

    #[test]
    fn playback_state_playing_exposes_position() {
        let state = PlaybackState::Playing {
            track: TrackId::new(1),
            position: SeekPosition::from_secs(12),
        };
        assert_eq!(state.position(), Some(SeekPosition::from_secs(12)));
    }

    #[test]
    fn playback_state_paused_exposes_position() {
        let state = PlaybackState::Paused {
            track: TrackId::new(3),
            position: SeekPosition::from_secs(30),
        };
        assert_eq!(state.position(), Some(SeekPosition::from_secs(30)));
    }

    #[test]
    fn playback_state_failed_is_not_stopped() {
        let state = PlaybackState::Failed {
            track: TrackId::new(1),
            error: "corrupt file".into(),
        };
        assert!(!state.is_stopped());
    }

    #[test]
    fn playback_state_failed_is_not_playing() {
        let state = PlaybackState::Failed {
            track: TrackId::new(1),
            error: "corrupt file".into(),
        };
        assert!(!state.is_playing());
    }

    #[test]
    fn playback_state_failed_exposes_current_track() {
        let id = TrackId::new(5);
        let state = PlaybackState::Failed {
            track: id,
            error: "missing file".into(),
        };
        assert_eq!(state.current_track(), Some(id));
    }

    #[test]
    fn playback_state_failed_has_no_position() {
        let state = PlaybackState::Failed {
            track: TrackId::new(1),
            error: "missing file".into(),
        };
        assert_eq!(state.position(), None);
    }

    /// A backend whose `play` always fails, so the player emits `Failed`.
    struct FailingBackend;

    impl AudioBackend for FailingBackend {
        fn play(&mut self, _path: &TrackPath) -> Result<(), AudioError> {
            Err(AudioError::Decode(
                "track.flac".into(),
                "corrupt file".into(),
            ))
        }
        fn play_paused(
            &mut self,
            _path: &TrackPath,
            _position: Duration,
        ) -> Result<(), AudioError> {
            Err(AudioError::Decode(
                "track.flac".into(),
                "corrupt file".into(),
            ))
        }
        fn pause(&mut self) {}
        // Never has a loaded source, so resume() must not make it Playing —
        // the case the phantom-playing bug hinged on.
        fn resume(&mut self) {}
        fn stop(&mut self) {}
        fn seek(&mut self, _position: Duration) {}
        fn set_volume(&mut self, _v: Volume) {}
        fn state(&self) -> BackendState {
            BackendState::Idle
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
    }

    #[test]
    fn player_transitions_to_failed_state_on_decode_error() {
        let (tx, rx) = mpsc::channel();
        let handle = PlayerHandle::launch(
            || Ok(FailingBackend),
            move |s| {
                let _ = tx.send(s);
            },
            |_tracks| {},
        );
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Failed { .. }));
        assert!(matches!(s, PlaybackState::Failed { .. }));
    }

    #[test]
    fn player_failed_state_includes_the_track_id() {
        let track = julie_and_candy();
        let expected = track.id;
        let (tx, rx) = mpsc::channel();
        let handle = PlayerHandle::launch(
            || Ok(FailingBackend),
            move |s| {
                let _ = tx.send(s);
            },
            |_tracks| {},
        );
        handle.send(PlayerCommand::Play(Box::new(track)));
        let s = recv_matching(&rx, |s| matches!(s, PlaybackState::Failed { .. }));
        assert_eq!(s.current_track(), Some(expected));
    }

    #[test]
    fn player_resume_after_failure_does_not_report_playing() {
        let (tx, rx) = mpsc::channel();
        let handle = PlayerHandle::launch(
            || Ok(FailingBackend),
            move |s| {
                let _ = tx.send(s);
            },
            |_tracks| {},
        );
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Failed { .. }));

        handle.send(PlayerCommand::Resume);

        // Absence check: drain every state emitted within a bounded window and
        // assert none of them is Playing. FailingBackend never has a loaded
        // source, so resume() must never be reported as Playing.
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(50)) {
                assert!(
                    !matches!(s, PlaybackState::Playing { .. }),
                    "resume after a failed track must not report Playing, got {s:?}"
                );
            }
        }
    }

    /// Drains `rx` until a track list matching `pred` arrives (or 3 s elapses),
    /// mirroring `recv_matching` for the queue-snapshot channel.
    fn recv_queue_matching(
        rx: &mpsc::Receiver<Vec<Track>>,
        pred: impl Fn(&[Track]) -> bool,
    ) -> Vec<Track> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(tracks) = rx.recv_timeout(Duration::from_millis(100))
                && pred(&tracks)
            {
                return tracks;
            }
            if Instant::now() > deadline {
                panic!("deadline exceeded waiting for expected queue snapshot");
            }
        }
    }

    #[test]
    fn queue_changed_reports_full_track_list_on_play_queue() {
        let (handle, _rx, queue_rx) = launch_with_channels();
        handle.send(PlayerCommand::PlayQueue {
            tracks: geogaddi_pair(),
            start: 0,
        });
        let tracks = recv_queue_matching(&queue_rx, |t| t.len() == 2);
        assert_eq!(tracks[0].id, TrackId::new(10));
        assert_eq!(tracks[1].id, TrackId::new(20));
    }

    #[test]
    fn queue_changed_reports_appended_tracks_on_enqueue() {
        let (handle, _rx, queue_rx) = launch_with_channels();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_queue_matching(&queue_rx, |t| t.len() == 1);

        handle.send(PlayerCommand::Enqueue(geogaddi_pair()));

        let tracks = recv_queue_matching(&queue_rx, |t| t.len() == 3);
        assert_eq!(tracks[0].id, julie_and_candy().id);
        assert_eq!(tracks[1].id, TrackId::new(10));
        assert_eq!(tracks[2].id, TrackId::new(20));
    }

    #[test]
    fn queue_changed_reports_empty_list_on_stop() {
        let (handle, rx, queue_rx) = launch_with_channels();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        recv_queue_matching(&queue_rx, |t| t.len() == 1);

        handle.send(PlayerCommand::Stop);

        let tracks = recv_queue_matching(&queue_rx, |t| t.is_empty());
        assert!(tracks.is_empty());
    }

    #[test]
    fn queue_changed_not_emitted_on_next_or_previous() {
        let (handle, rx, queue_rx) = launch_with_channels();
        handle.send(PlayerCommand::PlayQueue {
            tracks: geogaddi_pair(),
            start: 0,
        });
        recv_queue_matching(&queue_rx, |t| t.len() == 2);
        recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(10)));

        handle.send(PlayerCommand::Next);
        recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(20)));

        // Absence check: the track list itself didn't change, only the
        // cursor, so no further queue snapshot should follow.
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(tracks) = queue_rx.recv_timeout(Duration::from_millis(50)) {
                panic!("Next must not emit a queue snapshot, got {tracks:?}");
            }
        }
    }

    #[test]
    fn set_queue_preserves_current_track_position_when_it_survives() {
        let (handle, rx, queue_rx) = launch_with_channels();
        handle.send(PlayerCommand::PlayQueue {
            tracks: geogaddi_pair(),
            start: 1,
        });
        recv_matching(&rx, |s| s.current_track() == Some(TrackId::new(20)));
        recv_queue_matching(&queue_rx, |t| t.len() == 2);

        // Prune the first track; the playing track (id 20) survives.
        handle.send(PlayerCommand::SetQueue(vec![geogaddi(20, "1969")]));

        let tracks = recv_queue_matching(&queue_rx, |t| t.len() == 1);
        assert_eq!(tracks[0].id, TrackId::new(20));

        // Playback must not have been disturbed: no Stopped state follows.
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(50)) {
                assert_ne!(
                    s,
                    PlaybackState::Stopped,
                    "the surviving current track must keep playing"
                );
            }
        }
    }

    #[test]
    fn set_queue_stops_playback_when_current_track_is_pruned() {
        let (handle, rx, queue_rx) = launch_with_channels();
        handle.send(PlayerCommand::Play(Box::new(julie_and_candy())));
        recv_matching(&rx, |s| matches!(s, PlaybackState::Playing { .. }));
        recv_queue_matching(&queue_rx, |t| t.len() == 1);

        // The playing track (julie_and_candy) is not in the new list.
        handle.send(PlayerCommand::SetQueue(geogaddi_pair()));

        let s = recv_matching(&rx, |s| s == &PlaybackState::Stopped);
        assert_eq!(s, PlaybackState::Stopped);
        let tracks = recv_queue_matching(&queue_rx, |t| t.is_empty());
        assert!(tracks.is_empty());
    }
}
