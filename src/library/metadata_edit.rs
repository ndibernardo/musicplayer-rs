use std::path::Path;
use std::path::PathBuf;

use crate::library::db::Db;
use crate::library::db::DbError;
use crate::library::metadata;
use crate::library::metadata::MetadataError;
use crate::library::track::AlbumArtData;
use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Composer;
use crate::library::track::DiscNumber;
use crate::library::track::Genre;
use crate::library::track::Title;
use crate::library::track::Track;
use crate::library::track::TrackNumber;
use crate::library::track::Year;

/// Whether a field's value is the same across every track in a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shared<T> {
    Common(T),
    Mixed,
}

impl<T: Clone + PartialEq> Shared<T> {
    /// `Mixed` if `tracks` is empty or `extract` disagrees across them.
    pub fn of(tracks: &[Track], extract: impl Fn(&Track) -> T) -> Self {
        let mut values = tracks.iter().map(&extract);
        let Some(first) = values.next() else {
            return Shared::Mixed;
        };
        if values.all(|v| v == first) {
            Shared::Common(first)
        } else {
            Shared::Mixed
        }
    }
}

/// A user-authored change to a track's metadata. `None` means "leave
/// unchanged" — distinct from a domain "unknown" value (empty title, year 0).
#[derive(Debug, Clone, Default)]
pub struct TrackEdit {
    pub title: Option<Title>,
    pub artist: Option<Artist>,
    pub album_artist: Option<Artist>,
    pub album: Option<AlbumTitle>,
    pub genre: Option<Genre>,
    pub composer: Option<Composer>,
    pub track_number: Option<TrackNumber>,
    pub disc_number: Option<DiscNumber>,
    pub year: Option<Year>,
}

impl TrackEdit {
    /// Applies every `Some` field onto `track`; `None` fields pass through.
    pub fn apply(&self, track: Track) -> Track {
        Track {
            title: self.title.clone().unwrap_or(track.title),
            artist: self.artist.clone().unwrap_or(track.artist),
            album_artist: self.album_artist.clone().unwrap_or(track.album_artist),
            album: self.album.clone().unwrap_or(track.album),
            genre: self.genre.clone().unwrap_or(track.genre),
            composer: self.composer.clone().unwrap_or(track.composer),
            track_number: self.track_number.unwrap_or(track.track_number),
            disc_number: self.disc_number.unwrap_or(track.disc_number),
            year: self.year.unwrap_or(track.year),
            ..track
        }
    }

    /// True when the edit can change which cover an album shows: it touches a
    /// field `ArtKey` is derived from. Drives the texture-cache invalidation
    /// gate in the UI — a title/genre/number edit must not force every cover
    /// in the library to re-decode.
    pub fn affects_art_grouping(&self) -> bool {
        self.album.is_some() || self.album_artist.is_some() || self.artist.is_some()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("failed to write tags to {path}: {source}")]
    Metadata {
        path: PathBuf,
        source: MetadataError,
    },
    #[error("database error: {0}")]
    Database(#[from] DbError),
}

/// The outcome of a (possibly partial) save: every track whose file *and* DB
/// row were updated, plus the error that stopped the run, if any. File writes
/// aren't transactional across files, so a mid-album failure leaves the
/// already-written tracks saved — `saved` reports exactly which.
#[derive(Debug)]
pub struct EditOutcome {
    pub saved: Vec<Track>,
    pub failed: Option<EditError>,
}

/// Writes `edit` (and `art`, if changed) to every file in `tracks`, then
/// upserts the resulting tracks (and their art) to `db` in one transaction.
/// Stops writing files at the first failure — file writes aren't
/// transactional, so the caller sees exactly which file failed —  but still
/// commits whatever succeeded before that point.
pub fn save_edits(
    db: &Db,
    tracks: &[Track],
    edit: &TrackEdit,
    art: Option<&AlbumArtData>,
) -> EditOutcome {
    let mut stamped: Vec<(Track, u64, u64)> = Vec::with_capacity(tracks.len());
    let mut failed = None;

    for track in tracks {
        let edited = edit.apply(track.clone());
        match metadata::write(&edited, art) {
            Ok(()) => {
                let (mtime, size) = file_stamp(edited.path.as_path());
                stamped.push((edited, mtime, size));
            }
            Err(source) => {
                failed = Some(EditError::Metadata {
                    path: edited.path.as_path().to_path_buf(),
                    source,
                });
                break;
            }
        }
    }

    match commit(db, &stamped, art) {
        Ok(saved) => EditOutcome { saved, failed },
        Err(e) => EditOutcome {
            saved: Vec::new(),
            failed: Some(failed.unwrap_or(e)),
        },
    }
}

/// One transaction covering every stamped track (and, if `art` changed, its
/// art) — one WAL commit for a whole album instead of one per track. The
/// fresh `(mtime, size)` stamps mean the next scan's `known_file_stats`
/// comparison sees these files as already up to date, instead of re-parsing
/// (lofty read, art extraction included) every file this edit touched.
fn commit(
    db: &Db,
    stamped: &[(Track, u64, u64)],
    art: Option<&AlbumArtData>,
) -> Result<Vec<Track>, EditError> {
    if stamped.is_empty() {
        return Ok(Vec::new());
    }
    let tx = db.conn.unchecked_transaction().map_err(DbError::from)?;
    let mut saved = Vec::with_capacity(stamped.len());
    for (track, mtime, size) in stamped {
        let id = Db::upsert_one(&tx, track, *mtime, *size)?;
        if let Some(art) = art {
            Db::upsert_art_for_track(&tx, id, art.as_bytes())?;
        }
        saved.push(track.clone());
    }
    tx.commit().map_err(DbError::from)?;
    Ok(saved)
}

/// The track's fresh `(mtime_secs, size)` after `metadata::write` touched it.
/// Returns `(0, 0)` on a stat failure, matching `scan.rs`'s own fallback —
/// worst case the file is simply re-indexed on the next scan.
fn file_stamp(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (0, 0);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (mtime, meta.len())
}

/// Runs [`save_edits`] on a background thread with its own `Db` connection —
/// never share `db`'s main-thread connection across threads (see
/// `library::db`'s WAL/threading rules) — mirroring `scan::spawn_scan`'s
/// shape. Yields the single `EditOutcome`.
pub fn spawn_save_edits(
    db_path: PathBuf,
    tracks: Vec<Track>,
    edit: TrackEdit,
    art: Option<AlbumArtData>,
) -> async_channel::Receiver<EditOutcome> {
    let (tx, rx) = async_channel::unbounded::<EditOutcome>();

    std::thread::spawn(move || {
        let outcome = match Db::open(&db_path) {
            Ok(db) => save_edits(&db, &tracks, &edit, art.as_ref()),
            Err(e) => EditOutcome {
                saved: Vec::new(),
                failed: Some(EditError::from(e)),
            },
        };
        let _ = tx.try_send(outcome);
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackPath;

    fn roygbiv() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/music/boc/geogaddi/roygbiv.flac").unwrap(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Electronic"),
            composer: Composer::new(""),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
            duration: TrackDuration::from_secs(193),
        }
    }

    fn alpha_star() -> Track {
        Track {
            id: TrackId::new(2),
            path: TrackPath::new("/music/boc/geogaddi/alpha_and_omega.flac").unwrap(),
            title: Title::new("Alpha and Omega"),
            track_number: TrackNumber::new(9),
            ..roygbiv()
        }
    }

    #[test]
    fn shared_of_returns_common_when_every_track_agrees() {
        let tracks = vec![roygbiv(), alpha_star()];
        let shared = Shared::of(&tracks, |t| t.album.clone());
        assert_eq!(shared, Shared::Common(AlbumTitle::new("Geogaddi")));
    }

    #[test]
    fn shared_of_returns_mixed_when_a_track_differs() {
        let tracks = vec![roygbiv(), alpha_star()];
        let shared = Shared::of(&tracks, |t| t.title.clone());
        assert_eq!(shared, Shared::Mixed);
    }

    #[test]
    fn shared_of_returns_mixed_for_an_empty_slice() {
        let shared: Shared<AlbumTitle> = Shared::of(&[], |t| t.album.clone());
        assert_eq!(shared, Shared::Mixed);
    }

    #[test]
    fn track_edit_apply_overwrites_only_the_set_fields() {
        let edit = TrackEdit {
            album: Some(AlbumTitle::new("Geogaddi (Remaster)")),
            ..TrackEdit::default()
        };
        let edited = edit.apply(roygbiv());

        assert_eq!(edited.album.as_str(), "Geogaddi (Remaster)");
        assert_eq!(
            edited.title,
            roygbiv().title,
            "untouched field must survive"
        );
        assert_eq!(edited.track_number, roygbiv().track_number);
    }

    #[test]
    fn track_edit_default_leaves_every_field_unchanged() {
        let edited = TrackEdit::default().apply(roygbiv());
        assert_eq!(edited, roygbiv());
    }

    #[test]
    fn affects_art_grouping_is_true_only_for_key_fields() {
        assert!(
            TrackEdit {
                album: Some(AlbumTitle::new("Geogaddi (Remaster)")),
                ..TrackEdit::default()
            }
            .affects_art_grouping()
        );
        assert!(
            !TrackEdit {
                genre: Some(Genre::new("Ambient")),
                ..TrackEdit::default()
            }
            .affects_art_grouping()
        );
    }
}
