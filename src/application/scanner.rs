use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use crate::application::ports::RepositoryError;
use crate::application::ports::library::Library;
use crate::domain::library::LibraryFolder;
use crate::domain::track::Track;
use crate::domain::track::TrackPath;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot read directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// Walks `folder` recursively, reads each audio file with `read_track`, and upserts to `library`.
///
/// `read_track` returns `None` to skip a file (e.g. format error). Returns the count of
/// successfully indexed tracks.
pub fn scan_folder(
    folder: &LibraryFolder,
    library: &dyn Library,
    read_track: impl Fn(&TrackPath) -> Option<Track>,
) -> Result<u32, ScanError> {
    let files = collect_audio_files(folder.as_path())?;
    let mut count = 0u32;

    for path in files {
        let Ok(track_path) = TrackPath::new(&path) else {
            continue;
        };
        if let Some(track) = read_track(&track_path) {
            library.upsert_track(&track)?;
            count += 1;
        }
    }

    Ok(count)
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
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::path::Path;

    use super::*;
    use crate::application::ports::RepositoryError;
    use crate::application::ports::library::Library;
    use crate::domain::library::LibraryFolder;
    use crate::domain::track::AlbumTitle;
    use crate::domain::track::Artist;
    use crate::domain::track::DiscNumber;
    use crate::domain::track::Genre;
    use crate::domain::track::Title;
    use crate::domain::track::TrackDuration;
    use crate::domain::track::TrackId;
    use crate::domain::track::TrackNumber;
    use crate::domain::track::Year;

    struct InMemoryLibrary {
        tracks: RefCell<HashMap<TrackPath, Track>>,
    }

    impl InMemoryLibrary {
        fn new() -> Self {
            Self {
                tracks: RefCell::new(HashMap::new()),
            }
        }

        fn count(&self) -> usize {
            self.tracks.borrow().len()
        }
    }

    impl Library for InMemoryLibrary {
        fn add_folder(&self, _folder: &LibraryFolder) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn remove_folder(&self, _folder: &LibraryFolder) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn list_folders(&self) -> Result<Vec<LibraryFolder>, RepositoryError> {
            Ok(vec![])
        }

        fn upsert_track(&self, track: &Track) -> Result<TrackId, RepositoryError> {
            self.tracks
                .borrow_mut()
                .insert(track.path.clone(), track.clone());
            Ok(TrackId::new(0))
        }

        fn all_tracks(&self) -> Result<Vec<Track>, RepositoryError> {
            Ok(self.tracks.borrow().values().cloned().collect())
        }
    }

    fn fake_track(path: &TrackPath) -> Track {
        Track {
            id: TrackId::new(0),
            path: path.clone(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Music Has the Right to Children"),
            genre: Genre::new("Electronic"),
            duration: TrackDuration::from_secs(193),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(1998),
            art: None,
        }
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn scan_folder_indexes_audio_files() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track01.flac"));
        touch(&dir.path().join("track02.mp3"));

        let lib = InMemoryLibrary::new();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let count = scan_folder(&folder, &lib, |p| Some(fake_track(p))).unwrap();

        assert_eq!(count, 2);
        assert_eq!(lib.count(), 2);
    }

    #[test]
    fn scan_folder_skips_non_audio_files() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track01.flac"));
        touch(&dir.path().join("cover.jpg"));
        touch(&dir.path().join("info.txt"));

        let lib = InMemoryLibrary::new();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let count = scan_folder(&folder, &lib, |p| Some(fake_track(p))).unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn scan_folder_recurses_into_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("album");
        std::fs::create_dir(&sub).unwrap();
        touch(&dir.path().join("root.flac"));
        touch(&sub.join("sub.flac"));

        let lib = InMemoryLibrary::new();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let count = scan_folder(&folder, &lib, |p| Some(fake_track(p))).unwrap();

        assert_eq!(count, 2);
    }

    #[test]
    fn scan_folder_skips_files_where_read_track_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("corrupt.flac"));
        touch(&dir.path().join("valid.flac"));

        let lib = InMemoryLibrary::new();
        let folder = LibraryFolder::new(dir.path()).unwrap();

        let count = scan_folder(&folder, &lib, |p| {
            if p.as_path().file_name() == Some(OsStr::new("corrupt.flac")) {
                None
            } else {
                Some(fake_track(p))
            }
        })
        .unwrap();

        assert_eq!(count, 1);
        assert_eq!(lib.count(), 1);
    }

    #[test]
    fn scan_folder_skips_symlinked_directories() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track.flac"));
        // A symlink back to the folder itself recurses forever if followed.
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();

        let lib = InMemoryLibrary::new();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let count = scan_folder(&folder, &lib, |p| Some(fake_track(p))).unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn scan_folder_returns_error_for_nonexistent_directory() {
        let lib = InMemoryLibrary::new();
        let folder = LibraryFolder::new("/nonexistent/path/that/does/not/exist").unwrap();
        let result = scan_folder(&folder, &lib, |p| Some(fake_track(p)));
        assert!(matches!(result, Err(ScanError::ReadDir { .. })));
    }

    #[test]
    fn scan_folder_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("track.flac"));

        let lib = InMemoryLibrary::new();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        scan_folder(&folder, &lib, |p| Some(fake_track(p))).unwrap();
        scan_folder(&folder, &lib, |p| Some(fake_track(p))).unwrap();

        assert_eq!(lib.count(), 1, "re-scan must not duplicate rows");
    }
}
