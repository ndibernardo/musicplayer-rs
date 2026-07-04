use std::cell::Cell;
use std::rc::Rc;

use gtk4::Adjustment;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::Image;
use gtk4::Label;
use gtk4::Orientation;
use gtk4::ProgressBar;
use gtk4::Scale;
use gtk4::glib::markup_escape_text;
use gtk4::prelude::*;

use crate::library::track::Track;
use crate::player::PlaybackState;
use crate::player::PlayerCommand;
use crate::player::PlayerHandle;
use crate::player::SeekPosition;
use crate::player::Volume;

/// Bar containing transport controls, track info, and volume.
#[derive(Clone)]
pub struct PlayerBar {
    pub widget: GtkBox,
    track_label: Label,
    time_label: Label,
    total_label: Label,
    progress: ProgressBar,
    play_pause_btn: Button,
    volume_scale: Scale,
    // Tracks whether the engine is currently playing so the button can toggle correctly.
    is_playing: Rc<Cell<bool>>,
    // Current track length in milliseconds, for computing the progress fraction.
    duration_ms: Rc<Cell<u64>>,
}

impl PlayerBar {
    /// `initial_volume` is a 0–100 percentage restored from settings.
    pub fn new(player: PlayerHandle, initial_volume: f64) -> Self {
        let prev_btn = Button::from_icon_name("media-skip-backward-symbolic");
        prev_btn.set_tooltip_text(Some("Previous track"));
        let play_pause_btn = Button::from_icon_name("media-playback-start-symbolic");
        let stop_btn = Button::from_icon_name("media-playback-stop-symbolic");
        let next_btn = Button::from_icon_name("media-skip-forward-symbolic");
        next_btn.set_tooltip_text(Some("Next track"));

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
        controls.set_valign(gtk4::Align::Center);
        controls.append(&prev_btn);
        controls.append(&play_pause_btn);
        controls.append(&stop_btn);
        controls.append(&next_btn);

        let total_label = Label::new(Some("0:00"));
        total_label.add_css_class("numeric");
        total_label.set_width_chars(6);
        total_label.set_xalign(0.0);

        let progress = ProgressBar::new();
        progress.set_hexpand(true);
        progress.set_valign(gtk4::Align::Center);

        let vol_icon = Image::from_icon_name("audio-volume-medium-symbolic");
        let vol_box = GtkBox::new(Orientation::Horizontal, 4);
        vol_box.set_valign(gtk4::Align::Center);
        vol_box.append(&vol_icon);
        vol_box.append(&volume_scale);

        // Transport, times, progress, and volume share one row so the control
        // icons line up with the progress bar.
        let bottom = GtkBox::new(Orientation::Horizontal, 8);
        bottom.set_valign(gtk4::Align::Center);
        bottom.set_margin_start(8);
        bottom.set_margin_end(12);
        bottom.set_margin_bottom(12);
        bottom.append(&controls);
        bottom.append(&time_label);
        bottom.append(&progress);
        bottom.append(&total_label);
        bottom.append(&vol_box);

        track_label.set_margin_top(8);

        let widget = GtkBox::new(Orientation::Vertical, 4);
        widget.set_height_request(112);
        widget.add_css_class("toolbar");
        widget.add_css_class("player-bar");
        widget.append(&track_label);
        widget.append(&bottom);

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
            prev_btn.connect_clicked(move |_| player.send(PlayerCommand::Previous));
        }
        {
            let player = player.clone();
            next_btn.connect_clicked(move |_| player.send(PlayerCommand::Next));
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
            total_label,
            progress,
            play_pause_btn,
            volume_scale,
            is_playing,
            duration_ms: Rc::new(Cell::new(0)),
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
        self.track_label.set_markup(&track_markup(track));

        self.duration_ms
            .set(track.duration.as_duration().as_millis() as u64);
        self.total_label
            .set_text(&format_secs(track.duration.as_secs()));
        self.time_label.set_text("0:00");
        self.progress.set_fraction(0.0);
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
                self.total_label.set_text("0:00");
                self.progress.set_fraction(0.0);
            }
            PlaybackState::Playing { position, .. } => {
                self.is_playing.set(true);
                self.play_pause_btn
                    .set_icon_name("media-playback-pause-symbolic");
                self.show_position(*position);
            }
            PlaybackState::Paused { position, .. } => {
                self.is_playing.set(false);
                self.play_pause_btn
                    .set_icon_name("media-playback-start-symbolic");
                self.show_position(*position);
            }
        }
    }

    /// Updates the elapsed time label and the progress fraction for `position`.
    fn show_position(&self, position: SeekPosition) {
        self.time_label.set_text(&format_secs(position.as_secs()));
        let elapsed_ms = position.as_duration().as_millis() as u64;
        self.progress
            .set_fraction(fraction(elapsed_ms, self.duration_ms.get()));
    }
}

fn format_secs(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Pango markup showing the title large and bold with the artist dimmed beside
/// it, so the now-playing track stands out. Falls back to the file name when the
/// title tag is absent.
fn track_markup(track: &Track) -> String {
    let title = if track.title.is_unknown() {
        track
            .path
            .as_path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
    } else {
        track.title.as_str()
    };
    let title = markup_escape_text(title);
    if track.artist.is_unknown() {
        format!("<span size='large' weight='bold'>{title}</span>")
    } else {
        let artist = markup_escape_text(track.artist.as_str());
        format!(
            "<span size='large' weight='bold'>{title}</span>  <span size='large' alpha='70%'>{artist}</span>"
        )
    }
}

/// The played fraction in [0.0, 1.0], or 0.0 when the duration is unknown.
fn fraction(elapsed_ms: u64, duration_ms: u64) -> f64 {
    if duration_ms == 0 {
        0.0
    } else {
        (elapsed_ms as f64 / duration_ms as f64).min(1.0)
    }
}
