use async_channel::Sender;
use notify::Event;
use notify::EventKind;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;
use notify::event::ModifyKind;

use crate::library::db::LibraryFolder;

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("filesystem watch error: {0}")]
    Notify(#[from] notify::Error),
}

/// Owns the live filesystem watcher. Dropping it stops watching.
pub struct FolderWatcher {
    _watcher: RecommendedWatcher,
}

/// Watches each folder recursively and sends `()` on `tx` whenever an entry is
/// created, removed, or renamed beneath it — the changes that alter which tracks
/// exist. The caller debounces these and triggers a rescan.
///
/// # Errors
/// `WatchError::Notify` — the backend could not be created or a folder could not
/// be watched (e.g. it no longer exists).
pub fn watch_folders(
    folders: &[LibraryFolder],
    tx: Sender<()>,
) -> Result<FolderWatcher, WatchError> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        // A send failure means the UI dropped the receiver; nothing to do.
        if let Ok(event) = result
            && is_structural(&event.kind)
        {
            let _ = tx.try_send(());
        }
    })?;

    for folder in folders {
        watcher.watch(folder.as_path(), RecursiveMode::Recursive)?;
    }

    Ok(FolderWatcher { _watcher: watcher })
}

/// True for events that add, remove, or rename entries. Plain content edits
/// (writes to an already-indexed file) don't change the track set, so they're
/// ignored to avoid needless rescans.
fn is_structural(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::*;

    #[test]
    fn watch_folders_errors_on_nonexistent_folder() {
        let (tx, _rx) = async_channel::unbounded();
        let folder = LibraryFolder::new("/nonexistent/watch/target/xyz").unwrap();
        assert!(matches!(
            watch_folders(&[folder], tx),
            Err(WatchError::Notify(_))
        ));
    }

    #[test]
    fn watch_folders_reports_a_newly_created_file() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LibraryFolder::new(dir.path()).unwrap();
        let (tx, rx) = async_channel::unbounded();

        // Bind the watcher — dropping it would stop watching.
        let _watcher = watch_folders(&[folder], tx).unwrap();
        std::fs::write(dir.path().join("track01.flac"), b"").unwrap();

        // Poll for up to 5 seconds in 50 ms increments.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "creating a file should notify the watcher within 5 seconds"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn is_structural_ignores_content_modifications() {
        assert!(!is_structural(&EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content
        ))));
    }

    #[test]
    fn is_structural_accepts_creates_and_removes() {
        assert!(is_structural(&EventKind::Create(
            notify::event::CreateKind::File
        )));
        assert!(is_structural(&EventKind::Remove(
            notify::event::RemoveKind::File
        )));
    }
}
