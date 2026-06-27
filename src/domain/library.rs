use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LibraryError {
    #[error("library folder path must be absolute: {0:?}")]
    RelativePath(PathBuf),
}

/// Absolute path to a watched music folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LibraryFolder(PathBuf);

impl LibraryFolder {
    /// Returns `Err(RelativePath)` if `path` is not absolute.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, LibraryError> {
        let p = path.into();
        if !p.is_absolute() {
            return Err(LibraryError::RelativePath(p));
        }
        Ok(Self(p))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for LibraryFolder {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("scan failed: {reason}")]
pub struct ScanError {
    pub reason: String,
}

/// Current state of a library scan operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanStatus {
    Idle,
    Scanning {
        scanned: u32,
        total: u32,
    },
    Failed(ScanError),
    Complete {
        added: u32,
        updated: u32,
        removed: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_folder_new_accepts_absolute_path() {
        let folder = LibraryFolder::new("/home/user/Music").unwrap();
        assert_eq!(folder.as_path().to_str().unwrap(), "/home/user/Music");
    }

    #[test]
    fn library_folder_new_rejects_relative_path() {
        assert!(matches!(
            LibraryFolder::new("Music"),
            Err(LibraryError::RelativePath(_))
        ));
    }

    #[test]
    fn scan_status_idle_is_not_scanning() {
        assert!(!matches!(ScanStatus::Idle, ScanStatus::Scanning { .. }));
    }

    #[test]
    fn scan_status_complete_carries_counts() {
        let status = ScanStatus::Complete {
            added: 12,
            updated: 3,
            removed: 1,
        };
        assert!(matches!(status, ScanStatus::Complete { added: 12, .. }));
    }

    #[test]
    fn scan_status_scanning_carries_progress() {
        let status = ScanStatus::Scanning {
            scanned: 5,
            total: 20,
        };
        assert!(matches!(
            status,
            ScanStatus::Scanning {
                scanned: 5,
                total: 20
            }
        ));
    }

    #[test]
    fn scan_status_failed_carries_reason() {
        let err = ScanError {
            reason: "permission denied".into(),
        };
        let status = ScanStatus::Failed(err);
        assert!(matches!(status, ScanStatus::Failed(_)));
    }
}
