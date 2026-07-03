use crate::library::track::AlbumArtData;
use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Genre;
use crate::library::track::Year;

/// One album's worth of grid metadata: its title, its (primary) artist, genre,
/// release year, and cover art when any track in the album carries embedded art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumSummary {
    pub album: AlbumTitle,
    pub artist: Artist,
    pub genre: Genre,
    pub year: Year,
    pub art: Option<AlbumArtData>,
}
