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
