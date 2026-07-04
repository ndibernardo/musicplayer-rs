use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TrackError {
    #[error("path must be absolute: {0:?}")]
    RelativePath(PathBuf),
}

/// SQLite INTEGER PRIMARY KEY for a track. Matches the i64 rowid type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackId(i64);

impl TrackId {
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    pub fn value(self) -> i64 {
        self.0
    }
}

/// Track title. Empty means the tag was absent or unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Title(String);

impl Title {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().trim().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_unknown(&self) -> bool {
        self.0.is_empty()
    }
}

/// Artist name. Empty means the tag was absent or unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Artist(String);

impl Artist {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().trim().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_unknown(&self) -> bool {
        self.0.is_empty()
    }
}

/// Album title. Empty means the tag was absent or unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct AlbumTitle(String);

impl AlbumTitle {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().trim().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Musical genre. Empty means the tag was absent or unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Genre(String);

impl Genre {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().trim().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Composer name. Empty means the tag was absent or unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Composer(String);

impl Composer {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().trim().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_unknown(&self) -> bool {
        self.0.is_empty()
    }
}

/// Audio track duration. Always non-negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct TrackDuration(std::time::Duration);

impl TrackDuration {
    pub fn from_secs(secs: u64) -> Self {
        Self(std::time::Duration::from_secs(secs))
    }

    pub fn from_millis(millis: u64) -> Self {
        Self(std::time::Duration::from_millis(millis))
    }

    pub fn as_duration(self) -> std::time::Duration {
        self.0
    }

    pub fn as_secs(self) -> u64 {
        self.0.as_secs()
    }
}

/// 1-based track position within a disc. 0 means the tag was absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrackNumber(u32);

impl TrackNumber {
    pub fn new(n: u32) -> Self {
        Self(n)
    }

    pub fn value(self) -> u32 {
        self.0
    }

    pub fn is_unknown(self) -> bool {
        self.0 == 0
    }
}

/// 1-based disc number within an album. 0 means the tag was absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiscNumber(u32);

impl DiscNumber {
    pub fn new(n: u32) -> Self {
        Self(n)
    }

    pub fn value(self) -> u32 {
        self.0
    }

    pub fn is_unknown(self) -> bool {
        self.0 == 0
    }
}

/// Release year. 0 means the tag was absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Year(u16);

impl Year {
    pub fn new(y: u16) -> Self {
        Self(y)
    }

    pub fn value(self) -> u16 {
        self.0
    }

    pub fn is_unknown(self) -> bool {
        self.0 == 0
    }
}

/// Absolute filesystem path to an audio file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackPath(PathBuf);

impl TrackPath {
    /// Returns `Err(RelativePath)` if `path` is not absolute.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, TrackError> {
        let p = path.into();
        if !p.is_absolute() {
            return Err(TrackError::RelativePath(p));
        }
        Ok(Self(p))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TrackPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// Raw image bytes for an embedded album cover. JPEG or PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumArtData(Vec<u8>);

impl AlbumArtData {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A single audio track with all its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub id: TrackId,
    pub path: TrackPath,
    pub title: Title,
    pub artist: Artist,
    /// The album's credited artist. Empty falls back to `artist` for grouping.
    pub album_artist: Artist,
    pub album: AlbumTitle,
    pub genre: Genre,
    pub composer: Composer,
    pub duration: TrackDuration,
    pub track_number: TrackNumber,
    pub disc_number: DiscNumber,
    pub year: Year,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_new_trims_surrounding_whitespace() {
        assert_eq!(Title::new("  Geogaddi  ").as_str(), "Geogaddi");
    }

    #[test]
    fn title_new_empty_string_is_unknown() {
        assert!(Title::new("").is_unknown());
    }

    #[test]
    fn title_new_whitespace_only_is_unknown() {
        assert!(Title::new("   ").is_unknown());
    }

    #[test]
    fn artist_new_trims_surrounding_whitespace() {
        assert_eq!(
            Artist::new("  Boards of Canada  ").as_str(),
            "Boards of Canada"
        );
    }

    #[test]
    fn artist_new_empty_string_is_unknown() {
        assert!(Artist::new("").is_unknown());
    }

    #[test]
    fn artist_is_unknown_returns_false_for_known_artist() {
        assert!(!Artist::new("Boards of Canada").is_unknown());
    }

    #[test]
    fn composer_new_trims_surrounding_whitespace() {
        assert_eq!(Composer::new("  Wendy Carlos  ").as_str(), "Wendy Carlos");
    }

    #[test]
    fn composer_new_empty_string_is_unknown() {
        assert!(Composer::new("").is_unknown());
    }

    #[test]
    fn composer_is_unknown_returns_false_for_known_composer() {
        assert!(!Composer::new("Wendy Carlos").is_unknown());
    }

    #[test]
    fn track_path_new_accepts_absolute_path() {
        let path = TrackPath::new("/music/geogaddi/track01.flac").unwrap();
        assert_eq!(
            path.as_path().to_str().unwrap(),
            "/music/geogaddi/track01.flac"
        );
    }

    #[test]
    fn track_path_new_rejects_relative_path() {
        assert!(matches!(
            TrackPath::new("music/geogaddi/track01.flac"),
            Err(TrackError::RelativePath(_))
        ));
    }

    #[test]
    fn track_duration_from_secs_round_trips() {
        assert_eq!(TrackDuration::from_secs(243).as_secs(), 243);
    }

    #[test]
    fn track_duration_ordering_reflects_length() {
        assert!(TrackDuration::from_secs(120) < TrackDuration::from_secs(360));
    }

    #[test]
    fn track_number_zero_is_unknown() {
        assert!(TrackNumber::new(0).is_unknown());
    }

    #[test]
    fn track_number_nonzero_is_known() {
        assert!(!TrackNumber::new(7).is_unknown());
    }

    #[test]
    fn year_zero_is_unknown() {
        assert!(Year::new(0).is_unknown());
    }

    #[test]
    fn year_nonzero_is_known() {
        assert!(!Year::new(2002).is_unknown());
    }

    #[test]
    fn track_id_value_round_trips() {
        assert_eq!(TrackId::new(42).value(), 42);
    }
}
