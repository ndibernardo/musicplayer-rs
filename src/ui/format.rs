use gtk4::glib::markup_escape_text;

use crate::library::album::AlbumSummary;
use crate::library::track::Track;
use crate::library::track::TrackDuration;

/// Formats a `TrackDuration` as `m:ss`.
pub fn format_duration(d: TrackDuration) -> String {
    format_duration_secs(d.as_secs())
}

/// Formats a raw second count as `m:ss`.
pub fn format_duration_secs(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Returns the track's title, falling back to the path's filename stem when the
/// title tag is absent, and to an empty string if the path has no filename.
pub fn display_title(track: &Track) -> String {
    if track.title.is_unknown() {
        track
            .path
            .as_path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned()
    } else {
        track.title.as_str().to_owned()
    }
}

/// The track's position label. With a disc number it reads `disc.track` with the
/// track zero-padded to two digits (so an album sorts and reads as 1.01, 1.10,
/// 2.01); without one it is just the track number, and it is empty when neither
/// tag is present.
pub fn track_number(track: &Track) -> String {
    let track_num = track.track_number;
    if track.disc_number.is_unknown() {
        if track_num.is_unknown() {
            String::new()
        } else {
            track_num.value().to_string()
        }
    } else if track_num.is_unknown() {
        format!("{}.", track.disc_number.value())
    } else {
        format!("{}.{:02}", track.disc_number.value(), track_num.value())
    }
}

/// The album drawer's heading: "Album — Artist (Year)", the year parenthetical
/// omitted when unknown.
pub fn drawer_heading(summary: &AlbumSummary) -> String {
    let title = format!("{} — {}", summary.album.as_str(), summary.artist.as_str());
    if summary.year.is_unknown() {
        title
    } else {
        format!("{title} ({})", summary.year.value())
    }
}

/// Pango markup showing the title large and bold with the artist dimmed beside
/// it, so the now-playing track stands out. Falls back to the filename when the
/// title tag is absent.
pub fn track_markup(track: &Track) -> String {
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
