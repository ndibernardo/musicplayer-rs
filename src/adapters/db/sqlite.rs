use std::path::Path;

use rusqlite::Connection;

use crate::domain::library::LibraryFolder;
use crate::domain::track::AlbumArtData;
use crate::domain::track::AlbumTitle;
use crate::domain::track::Artist;
use crate::domain::track::DiscNumber;
use crate::domain::track::Genre;
use crate::domain::track::Title;
use crate::domain::track::Track;
use crate::domain::track::TrackDuration;
use crate::domain::track::TrackId;
use crate::domain::track::TrackNumber;
use crate::domain::track::TrackPath;
use crate::domain::track::Year;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Rusqlite(#[from] rusqlite::Error),
    #[error("invalid data in database: {0}")]
    InvalidData(String),
}

// Columns are nullable for tag fields — NULL means the tag was absent.
// The adapter maps NULL → domain defaults when reading rows.
const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS folders (
        folder_id INTEGER PRIMARY KEY,
        path      TEXT NOT NULL UNIQUE
    );

    CREATE TABLE IF NOT EXISTS tracks (
        track_id     INTEGER PRIMARY KEY,
        path         TEXT    NOT NULL UNIQUE,
        title        TEXT,
        artist       TEXT,
        album        TEXT,
        genre        TEXT,
        duration_ms  INTEGER,
        track_number INTEGER,
        disc_number  INTEGER,
        year         INTEGER,
        art          BLOB
    );
";

fn or_empty(v: Option<String>) -> String {
    v.unwrap_or_default()
}

fn or_zero(v: Option<i64>) -> i64 {
    v.unwrap_or(0)
}

pub struct Db {
    pub(crate) conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        // WAL allows a background writer and the UI reader to proceed without blocking each other.
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), DbError> {
        self.conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    pub fn add_folder(&self, folder: &LibraryFolder) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO folders (path) VALUES (?1)",
            [folder.as_path().to_string_lossy().as_ref()],
        )?;
        Ok(())
    }

    pub fn remove_folder(&self, folder: &LibraryFolder) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM folders WHERE path = ?1",
            [folder.as_path().to_string_lossy().as_ref()],
        )?;
        Ok(())
    }

    /// Inserts or updates a track keyed on its path. Returns the assigned track_id.
    pub fn upsert_track(&self, track: &Track) -> Result<TrackId, DbError> {
        let path = track.path.as_path().to_string_lossy();
        let title = (!track.title.is_unknown()).then(|| track.title.as_str().to_owned());
        let artist = (!track.artist.is_unknown()).then(|| track.artist.as_str().to_owned());
        let album = (!track.album.as_str().is_empty()).then(|| track.album.as_str().to_owned());
        let genre = (!track.genre.as_str().is_empty()).then(|| track.genre.as_str().to_owned());
        let duration_ms = track.duration.as_duration().as_millis() as i64;
        let track_number =
            (!track.track_number.is_unknown()).then(|| track.track_number.value() as i64);
        let disc_number =
            (!track.disc_number.is_unknown()).then(|| track.disc_number.value() as i64);
        let year = (!track.year.is_unknown()).then(|| track.year.value() as i64);
        let art = track.art.as_ref().map(|a| a.as_bytes().to_vec());

        // RETURNING yields the row's id on both the insert and the update path;
        // last_insert_rowid() would be stale after ON CONFLICT DO UPDATE.
        let id = self.conn.query_row(
            "INSERT INTO tracks (path, title, artist, album, genre, duration_ms, track_number, disc_number, year, art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(path) DO UPDATE SET
                 title        = excluded.title,
                 artist       = excluded.artist,
                 album        = excluded.album,
                 genre        = excluded.genre,
                 duration_ms  = excluded.duration_ms,
                 track_number = excluded.track_number,
                 disc_number  = excluded.disc_number,
                 year         = excluded.year,
                 art          = excluded.art
             RETURNING track_id",
            rusqlite::params![
                path.as_ref(),
                title,
                artist,
                album,
                genre,
                duration_ms,
                track_number,
                disc_number,
                year,
                art,
            ],
            |row| row.get::<_, i64>(0),
        )?;

        Ok(TrackId::new(id))
    }

    /// Returns all tracks ordered by artist → album → track number.
    pub fn list_tracks(&self) -> Result<Vec<Track>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT track_id, path, title, artist, album, genre,
                    duration_ms, track_number, disc_number, year, art
             FROM tracks
             ORDER BY artist, album, track_number",
        )?;

        stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<Vec<u8>>>(10)?,
            ))
        })?
        .map(|r| r.map_err(DbError::from))
        .map(|r| {
            let (
                id,
                path,
                title,
                artist,
                album,
                genre,
                duration_ms,
                track_num,
                disc_num,
                year,
                art,
            ) = r?;
            let path = TrackPath::new(path).map_err(|e| DbError::InvalidData(e.to_string()))?;
            Ok(Track {
                id: TrackId::new(id),
                path,
                title: Title::new(or_empty(title)),
                artist: Artist::new(or_empty(artist)),
                album: AlbumTitle::new(or_empty(album)),
                genre: Genre::new(or_empty(genre)),
                duration: TrackDuration::from_millis(or_zero(duration_ms) as u64),
                track_number: TrackNumber::new(or_zero(track_num) as u32),
                disc_number: DiscNumber::new(or_zero(disc_num) as u32),
                year: Year::new(or_zero(year) as u16),
                art: art.map(AlbumArtData::new),
            })
        })
        .collect()
    }

    pub fn track_count(&self) -> Result<u64, DbError> {
        let n = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| {
                row.get::<_, i64>(0)
            })?;
        Ok(n as u64)
    }

    pub fn list_folders(&self) -> Result<Vec<LibraryFolder>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM folders ORDER BY path")?;

        stmt.query_map([], |row| row.get::<_, String>(0))?
            .map(|res| res.map_err(DbError::from))
            .map(|res| {
                res.and_then(|path| {
                    LibraryFolder::new(path).map_err(|e| DbError::InvalidData(e.to_string()))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::track::AlbumTitle;
    use crate::domain::track::Artist;
    use crate::domain::track::DiscNumber;
    use crate::domain::track::Genre;
    use crate::domain::track::Title;
    use crate::domain::track::TrackDuration;
    use crate::domain::track::TrackId;
    use crate::domain::track::TrackNumber;
    use crate::domain::track::TrackPath;
    use crate::domain::track::Year;

    fn table_exists(db: &Db, name: &str) -> bool {
        db.conn
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1",
                [name],
                |_| Ok(()),
            )
            .is_ok()
    }

    #[test]
    fn open_in_memory_succeeds() {
        assert!(Db::open_in_memory().is_ok());
    }

    #[test]
    fn schema_creates_folders_table() {
        let db = Db::open_in_memory().unwrap();
        assert!(table_exists(&db, "folders"));
    }

    #[test]
    fn schema_creates_tracks_table() {
        let db = Db::open_in_memory().unwrap();
        assert!(table_exists(&db, "tracks"));
    }

    #[test]
    fn migrate_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.migrate().is_ok(), "second migration must not fail");
    }

    #[test]
    fn add_folder_persists_to_db() {
        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new("/home/user/Music").unwrap();
        db.add_folder(&folder).unwrap();
        assert_eq!(db.list_folders().unwrap(), vec![folder]);
    }

    #[test]
    fn add_folder_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new("/home/user/Music").unwrap();
        db.add_folder(&folder).unwrap();
        db.add_folder(&folder).unwrap();
        assert_eq!(db.list_folders().unwrap().len(), 1);
    }

    #[test]
    fn remove_folder_deletes_from_db() {
        let db = Db::open_in_memory().unwrap();
        let folder = LibraryFolder::new("/home/user/Music").unwrap();
        db.add_folder(&folder).unwrap();
        db.remove_folder(&folder).unwrap();
        assert!(db.list_folders().unwrap().is_empty());
    }

    fn minimal_track(path: &str) -> Track {
        Track {
            id: TrackId::new(0),
            path: TrackPath::new(path).unwrap(),
            title: Title::new(""),
            artist: Artist::new(""),
            album: AlbumTitle::new(""),
            genre: Genre::new(""),
            duration: TrackDuration::from_secs(0),
            track_number: TrackNumber::new(0),
            disc_number: DiscNumber::new(0),
            year: Year::new(0),
            art: None,
        }
    }

    fn full_track(path: &str) -> Track {
        Track {
            id: TrackId::new(0),
            path: TrackPath::new(path).unwrap(),
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

    #[test]
    fn upsert_track_inserts_new_track() {
        let db = Db::open_in_memory().unwrap();
        let track = full_track("/music/boc/roygbiv.flac");
        db.upsert_track(&track).unwrap();
        assert_eq!(db.track_count().unwrap(), 1);
    }

    #[test]
    fn upsert_track_returns_assigned_id() {
        let db = Db::open_in_memory().unwrap();
        let track = full_track("/music/boc/roygbiv.flac");
        let id = db.upsert_track(&track).unwrap();
        assert!(id.value() > 0);
    }

    #[test]
    fn upsert_track_updates_existing_by_path() {
        let db = Db::open_in_memory().unwrap();
        let path = "/music/boc/roygbiv.flac";

        db.upsert_track(&minimal_track(path)).unwrap();

        let updated = Track {
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            ..minimal_track(path)
        };
        db.upsert_track(&updated).unwrap();

        assert_eq!(
            db.track_count().unwrap(),
            1,
            "path is unique key — no duplicate row"
        );
    }

    #[test]
    fn upsert_track_preserves_id_on_update() {
        let db = Db::open_in_memory().unwrap();
        let path = "/music/boc/roygbiv.flac";
        let first_id = db.upsert_track(&minimal_track(path)).unwrap();
        let second_id = db.upsert_track(&full_track(path)).unwrap();
        assert_eq!(first_id, second_id, "track_id must not change on update");
    }

    #[test]
    fn upsert_track_returns_own_id_after_inserting_other_tracks() {
        let db = Db::open_in_memory().unwrap();
        let roygbiv_id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        db.upsert_track(&full_track("/music/boc/aquarius.flac"))
            .unwrap();

        // Update path: last_insert_rowid() still points at aquarius here —
        // the id must come from the updated row, not the last insert.
        let updated_id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();

        assert_eq!(updated_id, roygbiv_id);
    }

    #[test]
    fn upsert_track_stores_null_for_absent_tags() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&minimal_track("/music/unknown.mp3"))
            .unwrap();
        let title: Option<String> = db
            .conn
            .query_row("SELECT title FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert!(title.is_none(), "empty title must be stored as NULL");
    }

    #[test]
    fn track_count_returns_zero_for_fresh_db() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.track_count().unwrap(), 0);
    }

    #[test]
    fn list_tracks_returns_empty_for_fresh_db() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.list_tracks().unwrap().is_empty());
    }

    #[test]
    fn list_tracks_returns_inserted_track_with_correct_fields() {
        let db = Db::open_in_memory().unwrap();
        let track = full_track("/music/boc/roygbiv.flac");
        db.upsert_track(&track).unwrap();

        let tracks = db.list_tracks().unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_str(), "Roygbiv");
        assert_eq!(tracks[0].artist.as_str(), "Boards of Canada");
        assert_eq!(tracks[0].album.as_str(), "Music Has the Right to Children");
        assert_eq!(tracks[0].track_number.value(), 7);
        assert_eq!(tracks[0].year.value(), 1998);
    }

    #[test]
    fn list_tracks_maps_null_fields_to_domain_defaults() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&minimal_track("/music/unknown.mp3"))
            .unwrap();

        let tracks = db.list_tracks().unwrap();
        assert!(tracks[0].title.is_unknown());
        assert!(tracks[0].artist.is_unknown());
        assert!(tracks[0].track_number.is_unknown());
        assert!(tracks[0].year.is_unknown());
    }

    #[test]
    fn list_tracks_orders_by_artist_album_track_number() {
        let db = Db::open_in_memory().unwrap();
        let mut t3 = full_track("/music/boc/track03.flac");
        t3.track_number = TrackNumber::new(3);
        let mut t1 = full_track("/music/boc/track01.flac");
        t1.track_number = TrackNumber::new(1);
        let mut t2 = full_track("/music/boc/track02.flac");
        t2.track_number = TrackNumber::new(2);

        db.upsert_track(&t3).unwrap();
        db.upsert_track(&t1).unwrap();
        db.upsert_track(&t2).unwrap();

        let tracks = db.list_tracks().unwrap();
        assert_eq!(tracks[0].track_number.value(), 1);
        assert_eq!(tracks[1].track_number.value(), 2);
        assert_eq!(tracks[2].track_number.value(), 3);
    }

    #[test]
    fn list_folders_returns_all_configured_paths() {
        let db = Db::open_in_memory().unwrap();
        db.add_folder(&LibraryFolder::new("/home/user/Music").unwrap())
            .unwrap();
        db.add_folder(&LibraryFolder::new("/home/user/Downloads").unwrap())
            .unwrap();
        assert_eq!(db.list_folders().unwrap().len(), 2);
    }
}
