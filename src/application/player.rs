use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::application::ports::audio::AudioBackend;
use crate::application::ports::audio::AudioError;
use crate::domain::player::PlaybackState;
use crate::domain::player::PlayerCommand;
use crate::domain::player::SeekPosition;
use crate::domain::track::Track;

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
                        position: SeekPosition::from_millis(
                            backend.position().as_millis() as u64,
                        ),
                    });
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;
    use std::time::Instant;

    use super::PlayerHandle;
    use crate::application::ports::audio::AudioBackend;
    use crate::application::ports::audio::AudioError;
    use crate::domain::player::PlaybackState;
    use crate::domain::player::PlayerCommand;
    use crate::domain::player::Volume;
    use crate::domain::track::AlbumTitle;
    use crate::domain::track::Artist;
    use crate::domain::track::DiscNumber;
    use crate::domain::track::Genre;
    use crate::domain::track::Title;
    use crate::domain::track::Track;
    use crate::domain::track::TrackDuration;
    use crate::domain::track::TrackId;
    use crate::domain::track::TrackNumber;
    use crate::domain::track::TrackPath;
    use crate::domain::track::Year;

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
