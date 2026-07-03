use std::cell::Cell;
use std::rc::Rc;

use gtk4::Adjustment;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::Image;
use gtk4::Label;
use gtk4::Orientation;
use gtk4::Scale;
use gtk4::prelude::*;

use crate::library::track::Track;
use crate::player::PlaybackState;
use crate::player::PlayerCommand;
use crate::player::PlayerHandle;
use crate::player::Volume;

/// Bar containing transport controls, track info, and volume.
#[derive(Clone)]
pub struct PlayerBar {
    pub widget: GtkBox,
    track_label: Label,
    time_label: Label,
    play_pause_btn: Button,
    volume_scale: Scale,
    // Tracks whether the engine is currently playing so the button can toggle correctly.
    is_playing: Rc<Cell<bool>>,
}

impl PlayerBar {
    /// `initial_volume` is a 0–100 percentage restored from settings.
    pub fn new(player: PlayerHandle, initial_volume: f64) -> Self {
        let play_pause_btn = Button::from_icon_name("media-playback-start-symbolic");
        let stop_btn = Button::from_icon_name("media-playback-stop-symbolic");

        let track_label = Label::new(None);
        track_label.set_hexpand(true);
        track_label.set_halign(gtk4::Align::Center);
        track_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        track_label.set_max_width_chars(50);

        let time_label = Label::new(Some("0:00"));
        time_label.add_css_class("numeric");
        time_label.set_width_chars(6);
        time_label.set_xalign(1.0);

        let vol_adj = Adjustment::new(initial_volume, 0.0, 100.0, 1.0, 10.0, 0.0);
        let volume_scale = Scale::new(Orientation::Horizontal, Some(&vol_adj));
        volume_scale.set_width_request(120);
        volume_scale.set_draw_value(false);

        let controls = GtkBox::new(Orientation::Horizontal, 4);
        controls.set_margin_start(8);
        controls.set_margin_end(8);
        controls.set_valign(gtk4::Align::Center);
        controls.append(&play_pause_btn);
        controls.append(&stop_btn);

        let info = GtkBox::new(Orientation::Horizontal, 8);
        info.set_hexpand(true);
        info.set_halign(gtk4::Align::Center);
        info.set_valign(gtk4::Align::Center);
        info.append(&track_label);
        info.append(&time_label);

        let vol_box = GtkBox::new(Orientation::Horizontal, 4);
        vol_box.set_margin_end(12);
        vol_box.set_valign(gtk4::Align::Center);
        let vol_icon = Image::from_icon_name("audio-volume-medium-symbolic");
        vol_box.append(&vol_icon);
        vol_box.append(&volume_scale);

        let widget = GtkBox::new(Orientation::Horizontal, 0);
        widget.set_height_request(56);
        widget.add_css_class("toolbar");
        widget.append(&controls);
        widget.append(&info);
        widget.append(&vol_box);

        let is_playing = Rc::new(Cell::new(false));

        {
            let player = player.clone();
            let is_playing = Rc::clone(&is_playing);
            play_pause_btn.connect_clicked(move |_| {
                if is_playing.get() {
                    player.send(PlayerCommand::Pause);
                } else {
                    player.send(PlayerCommand::Resume);
                }
            });
        }
        {
            let player = player.clone();
            stop_btn.connect_clicked(move |_| player.send(PlayerCommand::Stop));
        }
        {
            let player = player.clone();
            volume_scale.connect_value_changed(move |scale| {
                let raw = scale.value() as f32 / 100.0;
                if let Ok(v) = Volume::new(raw) {
                    player.send(PlayerCommand::SetVolume(v));
                }
            });
        }

        // Sync the engine to the restored volume; the adjustment starts there, so
        // no value-changed fires to do it for us.
        if let Ok(v) = Volume::new(initial_volume as f32 / 100.0) {
            player.send(PlayerCommand::SetVolume(v));
        }

        Self {
            widget,
            track_label,
            time_label,
            play_pause_btn,
            volume_scale,
            is_playing,
        }
    }

    /// Registers a callback invoked with the new 0–100 volume whenever the user
    /// moves the volume slider.
    pub fn connect_volume_changed<F: Fn(f64) + 'static>(&self, f: F) {
        self.volume_scale
            .connect_value_changed(move |scale| f(scale.value()));
    }

    /// Called when a new track starts (from double-click or auto-advance).
    pub fn set_track(&self, track: &Track) {
        let text = if track.artist.is_unknown() {
            if track.title.is_unknown() {
                track
                    .path
                    .as_path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Unknown")
                    .to_owned()
            } else {
                track.title.as_str().to_owned()
            }
        } else {
            format!("{} — {}", track.artist.as_str(), track.title.as_str())
        };
        self.track_label.set_text(&text);
    }

    /// Called on every state change (play/pause/stop) and on position ticks.
    pub fn update_state(&self, state: &PlaybackState) {
        match state {
            PlaybackState::Stopped => {
                self.is_playing.set(false);
                self.play_pause_btn
                    .set_icon_name("media-playback-start-symbolic");
                self.track_label.set_text("");
                self.time_label.set_text("0:00");
            }
            PlaybackState::Playing { position, .. } => {
                self.is_playing.set(true);
                self.play_pause_btn
                    .set_icon_name("media-playback-pause-symbolic");
                self.time_label.set_text(&format_secs(position.as_secs()));
            }
            PlaybackState::Paused { position, .. } => {
                self.is_playing.set(false);
                self.play_pause_btn
                    .set_icon_name("media-playback-start-symbolic");
                self.time_label.set_text(&format_secs(position.as_secs()));
            }
        }
    }
}

fn format_secs(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}
