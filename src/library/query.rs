use crate::library::album::AlbumSort;
use crate::library::album::AlbumSummary;
use crate::library::db::Db;
use crate::library::db::DbError;
use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Genre;
use crate::library::track::Track;

/// The active track-list filter, driven by the sidebar selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryFilter {
    All,
    ByGenre(Genre),
    ByArtist(Artist),
    ByAlbum(AlbumTitle),
}

/// Returns the tracks matching `filter`.
pub fn tracks_for(filter: &LibraryFilter, db: &Db) -> Result<Vec<Track>, DbError> {
    match filter {
        LibraryFilter::All => db.list_tracks(),
        LibraryFilter::ByGenre(genre) => db.tracks_by_genre(genre),
        LibraryFilter::ByArtist(artist) => db.tracks_by_artist(artist),
        LibraryFilter::ByAlbum(album) => db.tracks_by_album(album),
    }
}

/// Returns the album summaries matching `filter`, in `sort` order — the album
/// grid shows only the albums under the active sidebar filter, or every album
/// when unfiltered.
pub fn album_summaries_for(
    filter: &LibraryFilter,
    sort: &AlbumSort,
    db: &Db,
) -> Result<Vec<AlbumSummary>, DbError> {
    match filter {
        LibraryFilter::All => db.album_summaries_sorted(sort),
        LibraryFilter::ByGenre(genre) => db.album_summaries_by_genre_sorted(genre, sort),
        LibraryFilter::ByArtist(artist) => db.album_summaries_by_artist_sorted(artist, sort),
        LibraryFilter::ByAlbum(album) => db.album_summaries_by_album_sorted(album, sort),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::track::AlbumTitle;
    use crate::library::track::Artist;
    use crate::library::track::Composer;
    use crate::library::track::DiscNumber;
    use crate::library::track::Genre;
    use crate::library::track::Title;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::TrackPath;
    use crate::library::track::Year;

    fn track(path: &str, artist: &str, album: &str, genre: &str) -> Track {
        Track {
            id: TrackId::new(0),
            path: TrackPath::new(path).unwrap(),
            title: Title::new("Roygbiv"),
            artist: Artist::new(artist),
            album_artist: Artist::new(artist),
            album: AlbumTitle::new(album),
            genre: Genre::new(genre),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(193),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(1998),
        }
    }

    fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track(
            "/music/boc/roygbiv.flac",
            "Boards of Canada",
            "Music Has the Right to Children",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track(
            "/music/aphex/xtal.flac",
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "Ambient",
        ))
        .unwrap();
        db
    }

    #[test]
    fn tracks_for_all_returns_every_track() {
        let db = library();
        let tracks = tracks_for(&LibraryFilter::All, &db).unwrap();
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn tracks_for_by_genre_returns_only_that_genre() {
        let db = library();
        let tracks = tracks_for(&LibraryFilter::ByGenre(Genre::new("Ambient")), &db).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].artist.as_str(), "Aphex Twin");
    }

    #[test]
    fn tracks_for_by_artist_returns_only_that_artist() {
        let db = library();
        let tracks = tracks_for(
            &LibraryFilter::ByArtist(Artist::new("Boards of Canada")),
            &db,
        )
        .unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].album.as_str(), "Music Has the Right to Children");
    }

    #[test]
    fn tracks_for_by_album_returns_only_that_album() {
        let db = library();
        let tracks = tracks_for(
            &LibraryFilter::ByAlbum(AlbumTitle::new("Selected Ambient Works 85-92")),
            &db,
        )
        .unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].genre.as_str(), "Ambient");
    }

    #[test]
    fn album_summaries_for_all_returns_every_album() {
        let db = library();
        let albums = album_summaries_for(&LibraryFilter::All, &AlbumSort::default(), &db).unwrap();
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn album_summaries_for_by_genre_returns_only_that_genre() {
        let db = library();
        let albums = album_summaries_for(
            &LibraryFilter::ByGenre(Genre::new("Ambient")),
            &AlbumSort::default(),
            &db,
        )
        .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist.as_str(), "Aphex Twin");
    }

    #[test]
    fn album_summaries_for_by_artist_returns_only_that_artist() {
        let db = library();
        let albums = album_summaries_for(
            &LibraryFilter::ByArtist(Artist::new("Boards of Canada")),
            &AlbumSort::default(),
            &db,
        )
        .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].album.as_str(), "Music Has the Right to Children");
    }

    #[test]
    fn album_summaries_for_by_album_returns_only_that_album() {
        let db = library();
        let albums = album_summaries_for(
            &LibraryFilter::ByAlbum(AlbumTitle::new("Selected Ambient Works 85-92")),
            &AlbumSort::default(),
            &db,
        )
        .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].genre.as_str(), "Ambient");
    }
}
