use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Genre;

/// The active track-list and album-grid filter, driven by the sidebar selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryFilter {
    All,
    ByGenre(Genre),
    ByArtist(Artist),
    ByAlbum(AlbumTitle),
}
