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
}
