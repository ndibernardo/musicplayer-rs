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
}

impl RodioAudioBackend {
    pub fn new() -> Result<Self, AudioError> {
        let device_sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let player = Player::connect_new(device_sink.mixer());
        Ok(Self {
            _device_sink: device_sink,
            player,
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
        Ok(())
    }

    fn play_paused(&mut self, path: &TrackPath, position: Duration) -> Result<(), AudioError> {
        let decoder = Self::open_decoder(path)?;
        self.fresh_player();
        // Pause before appending so the restored track makes no sound until the
        // user resumes; then move the play head to where the session ended.
        self.player.pause();
        self.player.append(decoder);
        let _ = self.player.try_seek(position);
        Ok(())
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
        self.player.set_volume(volume.value());
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
