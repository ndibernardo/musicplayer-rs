use std::path::PathBuf;

use async_channel::Receiver;

use crate::library::album::AlbumSort;
use crate::library::album::AlbumSummary;
use crate::library::db::Db;
use crate::library::db::DbError;
use crate::library::db::LibraryFolder;
use crate::library::filter::FilterField;
use crate::library::filter::LibraryFilter;
use crate::library::track::Track;

/// Errors that can occur while loading the library on startup.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("database error: {0}")]
    Database(#[from] DbError),
}

/// Everything the main window needs to populate itself right after opening —
/// loaded once, off the main thread, so the window can present before any of
/// it exists.
#[derive(Debug, Clone)]
pub struct LibraryBootstrap {
    pub folders: Vec<LibraryFolder>,
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumSummary>,
    pub sidebar_values: Vec<(FilterField, Vec<String>)>,
}

/// Loads a [`LibraryBootstrap`] on a background thread, through its own `Db`
/// connection, and sends the result once ready. Mirrors `scan::spawn_scan`'s
/// shape: a fresh connection per thread, result delivered over a channel.
pub fn spawn_bootstrap(
    db_path: PathBuf,
    filter: LibraryFilter,
    sort: AlbumSort,
    sidebar_fields: Vec<FilterField>,
) -> Receiver<Result<LibraryBootstrap, BootstrapError>> {
    let (tx, rx) = async_channel::unbounded();

    std::thread::spawn(move || {
        let result = Db::open(&db_path)
            .map_err(BootstrapError::from)
            .and_then(|db| load(&db, &filter, sort, &sidebar_fields));
        let _ = tx.try_send(result);
    });

    rx
}

/// The testable half of [`spawn_bootstrap`]: every query against an already
/// open `Db`, with no thread or connection setup of its own — mirrors
/// `scan::scan_folder` taking `&Db` while `scan::spawn_scan` owns the thread.
fn load(
    db: &Db,
    filter: &LibraryFilter,
    sort: AlbumSort,
    sidebar_fields: &[FilterField],
) -> Result<LibraryBootstrap, BootstrapError> {
    let folders = db.list_folders()?;
    let tracks = db.tracks_for(filter)?;
    let albums = db.album_summaries_for(filter, &sort)?;
    let sidebar_values = sidebar_fields
        .iter()
        .map(|&field| Ok((field, db.distinct_values_for(field)?)))
        .collect::<Result<Vec<_>, DbError>>()?;
    Ok(LibraryBootstrap {
        folders,
        tracks,
        albums,
        sidebar_values,
    })
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

    fn roygbiv() -> Track {
        Track {
            id: TrackId::new(0),
            path: TrackPath::new("/home/user/Music/boc/roygbiv.flac").unwrap(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Music Has the Right to Children"),
            genre: Genre::new("Electronic"),
            composer: Composer::new("Boards of Canada"),
            duration: TrackDuration::from_secs(193),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(1998),
        }
    }

    #[test]
    fn load_returns_empty_snapshot_for_a_fresh_db() {
        let db = Db::open_in_memory().unwrap();

        let bootstrap = load(
            &db,
            &LibraryFilter::All,
            AlbumSort::default(),
            &FilterField::all(),
        )
        .unwrap();

        assert!(bootstrap.folders.is_empty());
        assert!(bootstrap.tracks.is_empty());
        assert!(bootstrap.albums.is_empty());
        assert_eq!(bootstrap.sidebar_values.len(), FilterField::COUNT);
        assert!(bootstrap.sidebar_values.iter().all(|(_, v)| v.is_empty()));
    }

    #[test]
    fn load_returns_the_library_s_folders_tracks_albums_and_sidebar_values() {
        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new("/home/user/Music/boc").unwrap();
        db.add_folder(&folder).unwrap();
        db.upsert_track(&roygbiv()).unwrap();

        let bootstrap = load(
            &db,
            &LibraryFilter::All,
            AlbumSort::default(),
            &[FilterField::Genre],
        )
        .unwrap();

        assert_eq!(bootstrap.folders, vec![folder]);
        assert_eq!(bootstrap.tracks.len(), 1);
        assert_eq!(bootstrap.tracks[0].title.as_str(), "Roygbiv");
        assert_eq!(bootstrap.albums.len(), 1);
        assert_eq!(
            bootstrap.sidebar_values,
            vec![(FilterField::Genre, vec!["Electronic".to_owned()])]
        );
    }

    #[test]
    fn load_scopes_tracks_and_albums_to_the_given_filter() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&roygbiv()).unwrap();
        db.upsert_track(&Track {
            path: TrackPath::new("/home/user/Music/aphex/xtal.flac").unwrap(),
            title: Title::new("Xtal"),
            artist: Artist::new("Aphex Twin"),
            album_artist: Artist::new("Aphex Twin"),
            album: AlbumTitle::new("Selected Ambient Works 85-92"),
            genre: Genre::new("Ambient"),
            ..roygbiv()
        })
        .unwrap();

        let bootstrap = load(
            &db,
            &LibraryFilter::ByGenre(Genre::new("Ambient")),
            AlbumSort::default(),
            &[],
        )
        .unwrap();

        assert_eq!(bootstrap.tracks.len(), 1);
        assert_eq!(bootstrap.tracks[0].artist.as_str(), "Aphex Twin");
        assert_eq!(bootstrap.albums.len(), 1);
        assert_eq!(bootstrap.albums[0].artist.as_str(), "Aphex Twin");
        assert!(bootstrap.sidebar_values.is_empty());
    }

    #[test]
    fn spawn_bootstrap_sends_a_database_error_when_the_db_path_cannot_be_opened() {
        let db_path = std::path::PathBuf::from("/nonexistent/directory/library.db");

        let rx = spawn_bootstrap(
            db_path,
            LibraryFilter::All,
            AlbumSort::default(),
            Vec::new(),
        );

        let result = rx.recv_blocking().unwrap();
        assert!(matches!(result, Err(BootstrapError::Database(_))));
    }

    #[test]
    fn spawn_bootstrap_sends_the_loaded_snapshot_for_a_valid_db_path() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("library.db");
        // Written through the real path so `spawn_bootstrap`'s own `Db::open`
        // sees the same data — an in-memory `Db` isn't reachable from the
        // background thread it spawns.
        let db = Db::open(&db_path).unwrap();
        db.upsert_track(&roygbiv()).unwrap();
        drop(db);

        let rx = spawn_bootstrap(
            db_path,
            LibraryFilter::All,
            AlbumSort::default(),
            vec![FilterField::Genre],
        );

        let bootstrap = rx.recv_blocking().unwrap().unwrap();
        assert_eq!(bootstrap.tracks.len(), 1);
        assert_eq!(bootstrap.tracks[0].title.as_str(), "Roygbiv");
    }
}
