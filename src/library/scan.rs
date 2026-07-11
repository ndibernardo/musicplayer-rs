use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use async_channel::Receiver;
use async_channel::Sender;

use crate::library::db::Db;
use crate::library::db::DbError;
use crate::library::db::FileStamp;
use crate::library::db::LibraryFolder;
use crate::library::track::AlbumArtData;
use crate::library::track::Track;
use crate::library::track::TrackPath;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot read directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("database error: {0}")]
    Database(#[from] DbError),
}

/// One successfully read audio file: its tags, and its embedded cover if any.
/// `art` stays an `Option` because absence is a legitimate, meaningful state.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub track: Track,
    pub art: Option<AlbumArtData>,
}

/// Counts reported by a finished scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanSummary {
    pub indexed: u32,
    pub unchanged: u32,
    pub removed: u64,
}

/// A message sent from the background scan to the UI over the scan channel.
#[derive(Debug)]
pub enum ScanEvent {
    /// Running count of files indexed so far, across all folders.
    Progress(u32),
    /// The scan finished: its summary, or the first error encountered.
    Finished(Result<ScanSummary, ScanError>),
}

/// Walks `folder` recursively, reads each audio file with `read_track`, and upserts to `db`.
///
/// `read_track` returns `None` to skip a file (e.g. format error).
pub fn scan_folder(
    folder: &LibraryFolder,
    db: &Db,
    read_track: impl Fn(&TrackPath) -> Option<ScannedFile>,
) -> Result<ScanSummary, ScanError> {
    scan_folder_with_progress(folder, db, read_track, |_| {})
}

/// Like [`scan_folder`], but invokes `on_progress` with the running indexed count
/// after each file, so a caller can report scan progress live.
///
/// Upserts are batched into transactions of up to `BATCH_SIZE` files. One WAL
/// commit per batch is orders of magnitude faster than one per file for large
/// libraries while still calling `on_progress` after every individual file.
pub fn scan_folder_with_progress(
    folder: &LibraryFolder,
    db: &Db,
    read_track: impl Fn(&TrackPath) -> Option<ScannedFile>,
    mut on_progress: impl FnMut(u32),
) -> Result<ScanSummary, ScanError> {
    const BATCH_SIZE: usize = 200;
    let files = collect_audio_files(folder.as_path())?;
    let known = db.known_file_stats(folder)?;
    let mut seen: HashSet<PathBuf> = HashSet::with_capacity(files.len());
    let mut indexed = 0u32;
    let mut unchanged = 0u32;

    for chunk in files.chunks(BATCH_SIZE) {
        let tx = db.conn.unchecked_transaction().map_err(DbError::from)?;
        for path in chunk {
            seen.insert(path.clone());
            let Ok(track_path) = TrackPath::new(path) else {
                continue;
            };
            let stamp = std::fs::metadata(path).ok().map(|m| FileStamp::of(&m));
            if known.get(path) == stamp.as_ref() {
                unchanged += 1;
                continue; // file unchanged — skip re-indexing
            }
            if let Some(scanned) = read_track(&track_path) {
                let track_id = Db::upsert_one(&tx, &scanned.track, stamp)?;
                if let Some(art) = scanned.art {
                    Db::upsert_art_for_track(&tx, track_id, art.as_bytes())?;
                }
                indexed += 1;
                on_progress(indexed);
            }
        }
        tx.commit().map_err(DbError::from)?;
    }

    let removed = db.remove_stale_tracks(folder, &seen)?;
    db.prune_orphaned_art()?;
    Ok(ScanSummary {
        indexed,
        unchanged,
        removed,
    })
}

/// Scans `folders` on a background thread with its own DB connection (WAL mode
/// lets it write while the UI reads). Streams `ScanEvent::Progress` as files are
/// indexed, then a final `ScanEvent::Finished` with the total or the first error.
pub fn spawn_scan(db_path: PathBuf, folders: Vec<LibraryFolder>) -> Receiver<ScanEvent> {
    let (tx, rx) = async_channel::unbounded::<ScanEvent>();

    std::thread::spawn(move || {
        let db = match Db::open(&db_path) {
            Ok(db) => db,
            Err(e) => {
                let _ = tx.try_send(ScanEvent::Finished(Err(ScanError::from(e))));
                return;
            }
        };

        let mut total = ScanSummary::default();
        for folder in &folders {
            let base = total.indexed;
            let progress_tx: Sender<ScanEvent> = tx.clone();
            let result = scan_folder_with_progress(
                folder,
                &db,
                |p| {
                    crate::library::metadata::read(p)
                        .ok()
                        .map(|(track, art)| ScannedFile { track, art })
                },
                |n| {
                    let _ = progress_tx.try_send(ScanEvent::Progress(base + n));
                },
            );
            match result {
                Ok(summary) => {
                    total.indexed += summary.indexed;
                    total.unchanged += summary.unchanged;
                    total.removed += summary.removed;
                }
                Err(e) => {
                    let _ = tx.try_send(ScanEvent::Finished(Err(e)));
                    return;
                }
            }
        }

        let _ = tx.try_send(ScanEvent::Finished(Ok(total)));
    });

    rx
}

fn collect_audio_files(dir: &Path) -> Result<Vec<PathBuf>, ScanError> {
    let entries = std::fs::read_dir(dir).map_err(|source| ScanError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut files = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        // Never recurse through symlinks: a link pointing back up the tree
        // would otherwise make the walk cycle forever.
        let is_symlink = entry.file_type().is_ok_and(|ft| ft.is_symlink());
        if path.is_dir() {
            if !is_symlink {
                files.extend(collect_audio_files(&path)?);
            }
        } else if is_audio_file(&path) {
            files.push(path);
        }
    }
    Ok(files)
}

fn is_audio_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("mp3" | "flac" | "ogg" | "m4a" | "wav" | "opus" | "aac")
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::*;
    use crate::library::db::LibraryFolder;
    use crate::library::track::AlbumTitle;
    use crate::library::track::Artist;
    use crate::library::track::Composer;
    use crate::library::track::DiscNumber;
    use crate::library::track::Genre;
    use crate::library::track::Title;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::Year;

    fn fake_track(path: &TrackPath) -> Track {
        Track {
            id: TrackId::new(0),
            path: path.clone(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Music Has the Right to Children"),
            genre: Genre::new("Electronic"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(193),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(1998),
        }
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"").unwrap();
    }

    fn scanned(path: &TrackPath) -> ScannedFile {
        ScannedFile {
            track: fake_track(path),
            art: None,
        }
    }

    #[test]
    fn scan_folder_indexes_audio_files() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track01.flac"));
        touch(&dir.path().join("track02.mp3"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let summary = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();

        assert_eq!(summary.indexed, 2);
        assert_eq!(db.track_count().unwrap(), 2);
    }

    #[test]
    fn scan_folder_skips_non_audio_files() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track01.flac"));
        touch(&dir.path().join("cover.jpg"));
        touch(&dir.path().join("info.txt"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let summary = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();

        assert_eq!(summary.indexed, 1);
    }

    #[test]
    fn scan_folder_recurses_into_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("album");
        std::fs::create_dir(&sub).unwrap();
        touch(&dir.path().join("root.flac"));
        touch(&sub.join("sub.flac"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let summary = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();

        assert_eq!(summary.indexed, 2);
    }

    #[test]
    fn scan_folder_skips_files_where_read_track_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("corrupt.flac"));
        touch(&dir.path().join("valid.flac"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();

        let summary = scan_folder(&folder, &db, |p| {
            if p.as_path().file_name() == Some(OsStr::new("corrupt.flac")) {
                None
            } else {
                Some(scanned(p))
            }
        })
        .unwrap();

        assert_eq!(summary.indexed, 1);
        assert_eq!(db.track_count().unwrap(), 1);
    }

    #[test]
    fn scan_folder_skips_symlinked_directories() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track.flac"));
        // A symlink back to the folder itself recurses forever if followed.
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let summary = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();

        assert_eq!(summary.indexed, 1);
    }

    #[test]
    fn scan_folder_returns_error_for_nonexistent_directory() {
        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new("/nonexistent/path/that/does/not/exist").unwrap();
        let result = scan_folder(&folder, &db, |p| Some(scanned(p)));
        assert!(matches!(result, Err(ScanError::ReadDir { .. })));
    }

    #[test]
    fn scan_folder_with_progress_reports_running_count() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track01.flac"));
        touch(&dir.path().join("track02.flac"));
        touch(&dir.path().join("track03.flac"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();

        let mut seen = Vec::new();
        let summary =
            scan_folder_with_progress(&folder, &db, |p| Some(scanned(p)), |n| seen.push(n))
                .unwrap();

        assert_eq!(summary.indexed, 3);
        assert_eq!(seen, vec![1, 2, 3], "progress is the running indexed count");
    }

    #[test]
    fn scan_folder_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track.flac"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();
        scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();

        assert_eq!(
            db.track_count().unwrap(),
            1,
            "re-scan must not duplicate rows"
        );
    }

    #[test]
    fn scan_folder_skips_unchanged_files_on_second_scan() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track.flac"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();

        let first = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();
        assert_eq!(first.indexed, 1);

        // File unchanged on disk: mtime and size are the same.
        let second = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();
        assert_eq!(second.indexed, 0, "unchanged file must be skipped");
        assert_eq!(
            second.unchanged, 1,
            "unchanged file is counted, not silently dropped"
        );
        assert_eq!(db.track_count().unwrap(), 1, "track must still exist");
    }

    #[test]
    fn scan_folder_reindexes_file_whose_size_changed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("track.flac");
        std::fs::write(&file, b"original").unwrap();

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();

        scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();

        // Write more bytes so the file size changes, triggering re-indexing.
        std::fs::write(&file, b"updated with different byte length").unwrap();
        let summary = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();
        assert_eq!(
            summary.indexed, 1,
            "file with changed size must be re-indexed"
        );
    }

    #[test]
    fn scan_folder_removes_deleted_files_from_db() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("track.flac");
        touch(&file);

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();

        scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();
        assert_eq!(db.track_count().unwrap(), 1);

        std::fs::remove_file(&file).unwrap();

        let summary = scan_folder(&folder, &db, |p| Some(scanned(p))).unwrap();
        assert_eq!(
            db.track_count().unwrap(),
            0,
            "deleted file must be removed from DB"
        );
        assert_eq!(summary.removed, 1, "removed count reflects the pruned row");
    }

    fn track_art_row_count(db: &Db) -> u32 {
        db.conn
            .query_row("SELECT COUNT(*) FROM track_art", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap() as u32
    }

    fn art_blob_count(db: &Db) -> u32 {
        db.conn
            .query_row("SELECT COUNT(*) FROM art_blobs", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap() as u32
    }

    #[test]
    fn scan_folder_writes_one_track_art_row_per_track_but_shares_one_blob() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track01.flac"));
        touch(&dir.path().join("track02.flac"));
        touch(&dir.path().join("track03.flac"));

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let art = AlbumArtData::new(vec![0xFF, 0xD8, 0xFF]);
        scan_folder(&folder, &db, |p| {
            Some(ScannedFile {
                track: fake_track(p),
                art: Some(art.clone()),
            })
        })
        .unwrap();

        assert_eq!(
            track_art_row_count(&db),
            3,
            "every track that has art gets its own track_art row"
        );
        assert_eq!(
            art_blob_count(&db),
            1,
            "three tracks sharing identical embedded bytes must share one blob"
        );
    }

    #[test]
    fn scan_folder_prunes_art_for_an_album_whose_last_track_was_removed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("track.flac");
        touch(&file);

        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let art = AlbumArtData::new(vec![0xFF, 0xD8, 0xFF]);
        scan_folder(&folder, &db, |p| {
            Some(ScannedFile {
                track: fake_track(p),
                art: Some(art.clone()),
            })
        })
        .unwrap();
        assert_eq!(track_art_row_count(&db), 1);

        std::fs::remove_file(&file).unwrap();
        scan_folder(&folder, &db, |p| {
            Some(ScannedFile {
                track: fake_track(p),
                art: Some(art.clone()),
            })
        })
        .unwrap();

        assert_eq!(
            track_art_row_count(&db),
            0,
            "art for a fully-removed track must be pruned"
        );
        assert_eq!(art_blob_count(&db), 0);
    }
}
