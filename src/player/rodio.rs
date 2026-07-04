use std::fs::File;
use std::io::BufReader;
use std::time::Duration;

use rodio::Decoder;
use rodio::DeviceSinkBuilder;
use rodio::MixerDeviceSink;
use rodio::Player;

use crate::library::track::TrackPath;
use crate::player::AudioBackend;
use crate::player::AudioError;
use crate::player::Volume;

pub struct RodioAudioBackend {
    // Keeps the OS audio stream alive. Must outlive `player`.
    _device_sink: MixerDeviceSink,
    player: Player,
    // The current track's path, needed to reload it for a backward seek.
    current_path: Option<TrackPath>,
    // Remembered so a fresh player (new track or backward seek) keeps the volume.
    volume: f32,
}

impl RodioAudioBackend {
    pub fn new() -> Result<Self, AudioError> {
        let device_sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let player = Player::connect_new(device_sink.mixer());
        Ok(Self {
            _device_sink: device_sink,
            player,
            current_path: None,
            volume: 1.0,
        })
    }
}

impl RodioAudioBackend {
    /// Opens and decodes `path`, mapping failures to `AudioError::Decode`.
    fn open_decoder(path: &TrackPath) -> Result<Decoder<BufReader<File>>, AudioError> {
        let path_str = path.as_path().to_string_lossy().into_owned();
        let file = File::open(path.as_path())
            .map_err(|e| AudioError::Decode(path_str.clone(), e.to_string()))?;
        Decoder::new(BufReader::new(file)).map_err(|e| AudioError::Decode(path_str, e.to_string()))
    }

    /// Replaces the player with a fresh one, stopping the previous track cleanly.
    fn fresh_player(&mut self) {
        self.player = Player::connect_new(self._device_sink.mixer());
    }
}

impl AudioBackend for RodioAudioBackend {
    fn play(&mut self, path: &TrackPath) -> Result<(), AudioError> {
        let decoder = Self::open_decoder(path)?;
        self.fresh_player();
        self.player.append(decoder);
        self.player.set_volume(self.volume);
        self.current_path = Some(path.clone());
        Ok(())
    }

    fn play_paused(&mut self, path: &TrackPath, position: Duration) -> Result<(), AudioError> {
        let decoder = Self::open_decoder(path)?;
        self.fresh_player();
        // Pause before appending so the restored track makes no sound until the
        // user resumes; then move the play head to where the session ended.
        self.player.pause();
        self.player.append(decoder);
        self.player.set_volume(self.volume);
        let _ = self.player.try_seek(position);
        self.current_path = Some(path.clone());
        Ok(())
    }

    fn seek(&mut self, position: Duration) {
        // The live decoder only seeks forward; a backward seek returns an error
        // and leaves the position unchanged. So for a backward target, reload the
        // track and seek forward from the start, which is supported.
        if position < self.player.get_pos() {
            if let Some(path) = self.current_path.clone() {
                let was_paused = self.player.is_paused();
                if let Ok(decoder) = Self::open_decoder(&path) {
                    self.fresh_player();
                    if was_paused {
                        self.player.pause();
                    }
                    self.player.append(decoder);
                    self.player.set_volume(self.volume);
                    let _ = self.player.try_seek(position);
                }
            }
        } else {
            let _ = self.player.try_seek(position);
        }
    }

    fn pause(&mut self) {
        self.player.pause();
    }

    fn resume(&mut self) {
        self.player.play();
    }

    fn stop(&mut self) {
        self.player.stop();
    }

    fn set_volume(&mut self, volume: Volume) {
        self.volume = volume.value();
        self.player.set_volume(self.volume);
    }

    fn is_playing(&self) -> bool {
        !self.player.is_paused() && !self.player.empty()
    }

    fn is_paused(&self) -> bool {
        self.player.is_paused()
    }

    fn position(&self) -> Duration {
        self.player.get_pos()
    }
}
