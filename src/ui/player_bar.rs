use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Adjustment;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::GestureClick;
use gtk4::Image;
use gtk4::Label;
use gtk4::Orientation;
use gtk4::Scale;
use gtk4::glib::markup_escape_text;
use gtk4::prelude::*;

use crate::library::track::Track;
use crate::player::PlaybackState;
use crate::player::PlayerCommand;
use crate::player::PlayerHandle;
use crate::player::SeekPosition;
use crate::player::Volume;
use crate::ui::format::display_title;
use crate::ui::format::format_duration_secs;

/// Bar containing transport controls, track info, and volume.
#[derive(Clone)]
pub struct PlayerBar {
    pub widget: GtkBox,
    track_label: Label,
    time_label: Label,
    total_label: Label,
    progress: Scale,
    play_pause_btn: Button,
    volume_scale: Scale,
    // The last state reported by the player, so the play/pause button can
    // decide its action from the actual domain state instead of a bool that
    // collapses Stopped and Failed into the same "not playing" bucket.
    last_state: Rc<RefCell<PlaybackState>>,
    // The last track shown, kept so a click on a Failed track can retry it.
    last_track: Rc<RefCell<Option<Track>>>,
    // Current track length in milliseconds, used to set the progress scale range.
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
        // The playing markup uses a `large` span, which is taller than the
        // plain empty label shown at Stop; fix the height so the bar's
        // overall size never depends on which one is set.
        track_label.set_height_request(28);

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

        let duration_ms = Rc::new(Cell::new(0u64));

        // A scale for the fill; the knob is hidden by CSS. A click seeks to the
        // exact clicked fraction of the track, so it jumps both forward and back
        // (a plain trough click only page-steps, which playback overtakes).
        let progress = Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 1000.0);
        progress.add_css_class("seek");
        progress.set_hexpand(true);
        progress.set_draw_value(false);
        progress.set_valign(gtk4::Align::Center);
        {
            let player = player.clone();
            let duration_ms = Rc::clone(&duration_ms);
            let scale = progress.clone();
            let click = GestureClick::new();
            // Capture phase + claim, so this runs before the scale's own drag
            // gesture, which would otherwise swallow the click (leaving seeking
            // dead) or only page-step it.
            click.set_propagation_phase(gtk4::PropagationPhase::Capture);
            click.connect_pressed(move |gesture, _n_press, x, _y| {
                let width = scale.width().max(1) as f64;
                let fraction = (x / width).clamp(0.0, 1.0);
                let position = (fraction * duration_ms.get() as f64) as u64;
                player.send(PlayerCommand::Seek(SeekPosition::from_millis(position)));
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            });
            progress.add_controller(click);
        }

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

        let last_state = Rc::new(RefCell::new(PlaybackState::Stopped));
        let last_track: Rc<RefCell<Option<Track>>> = Rc::new(RefCell::new(None));

        {
            let player = player.clone();
            let last_state = Rc::clone(&last_state);
            let last_track = Rc::clone(&last_track);
            play_pause_btn.connect_clicked(move |_| match &*last_state.borrow() {
                PlaybackState::Playing { .. } => player.send(PlayerCommand::Pause),
                PlaybackState::Paused { .. } => player.send(PlayerCommand::Resume),
                // The queue was cleared on stop; there is nothing to resume.
                PlaybackState::Stopped => {}
                // Retry: the queue still holds this track, but the backend has
                // no loaded source, so a bare Resume would report a phantom
                // Playing state instead of actually retrying it.
                PlaybackState::Failed { .. } => {
                    if let Some(track) = last_track.borrow().clone() {
                        player.send(PlayerCommand::Play(Box::new(track)));
                    }
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
            last_state,
            last_track,
            duration_ms,
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
        *self.last_track.borrow_mut() = Some(track.clone());
        self.track_label.set_markup(&track_markup(track));

        let duration_ms = track.duration.as_duration().as_millis() as u64;
        self.duration_ms.set(duration_ms);
        self.total_label
            .set_text(&format_duration_secs(track.duration.as_secs()));
        self.time_label.set_text("0:00");
        // At least 1 so the scale has a valid, non-empty range even for a
        // zero-length or untagged track.
        self.progress.set_range(0.0, duration_ms.max(1) as f64);
        self.progress.set_value(0.0);
    }

    /// Called on every state change (play/pause/stop/fail) and on position ticks.
    pub fn update_state(&self, state: &PlaybackState) {
        *self.last_state.borrow_mut() = state.clone();
        match state {
            PlaybackState::Stopped => {
                self.play_pause_btn
                    .set_icon_name("media-playback-start-symbolic");
                self.track_label.set_text("");
                self.time_label.set_text("0:00");
                self.total_label.set_text("0:00");
                self.progress.set_value(0.0);
            }
            PlaybackState::Playing { position, .. } => {
                self.play_pause_btn
                    .set_icon_name("media-playback-pause-symbolic");
                self.show_position(*position);
            }
            PlaybackState::Paused { position, .. } => {
                self.play_pause_btn
                    .set_icon_name("media-playback-start-symbolic");
                self.show_position(*position);
            }
            PlaybackState::Failed { .. } => {
                self.play_pause_btn
                    .set_icon_name("media-playback-start-symbolic");
            }
        }
    }

    /// Updates the elapsed time label and the progress position. Uses set_value
    /// (not change-value), so this does not trigger a seek.
    fn show_position(&self, position: SeekPosition) {
        self.time_label
            .set_text(&format_duration_secs(position.as_secs()));
        self.progress
            .set_value(position.as_duration().as_millis() as f64);
    }
}

/// Pango markup showing the title large and bold with the artist dimmed beside
/// it, so the now-playing track stands out. Falls back to the filename when the
/// title tag is absent.
fn track_markup(track: &Track) -> String {
    let raw = display_title(track);
    let title = markup_escape_text(&raw);
    if track.artist.is_unknown() {
        format!("<span size='large' weight='bold'>{title}</span>")
    } else {
        let artist = markup_escape_text(track.artist.as_str());
        format!(
            "<span size='large' weight='bold'>{title}</span>  <span size='large' alpha='70%'>{artist}</span>"
        )
    }
}
