use std::path::Path;
use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::library::album::AlbumSummary;
use crate::library::track::AlbumArtData;
use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Composer;
use crate::library::track::DiscNumber;
use crate::library::track::Genre;
use crate::library::track::Title;
use crate::library::track::Track;
use crate::library::track::TrackDuration;
use crate::library::track::TrackId;
use crate::library::track::TrackNumber;
use crate::library::track::TrackPath;
use crate::library::track::Year;

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

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Rusqlite(#[from] rusqlite::Error),
    #[error("invalid data in database: {0}")]
    InvalidData(String),
}

// Columns are nullable for tag fields — NULL means the tag was absent.
// The adapter maps NULL to the domain defaults when reading rows.
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
        album_artist TEXT,
        album        TEXT,
        genre        TEXT,
        composer     TEXT,
        duration_ms  INTEGER,
        track_number INTEGER,
        disc_number  INTEGER,
        year         INTEGER,
        art          BLOB
    );

    CREATE TABLE IF NOT EXISTS settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
";

fn or_empty(v: Option<String>) -> String {
    v.unwrap_or_default()
}

fn or_zero(v: Option<i64>) -> i64 {
    v.unwrap_or(0)
}

/// Raw column values of a `tracks` row, before domain validation. The trailing
/// `album_artist` and `composer` come last, matching the `SELECT` in
/// `query_tracks`.
type TrackRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<Vec<u8>>,
    Option<String>,
    Option<String>,
);

/// Builds a domain `Track` from a raw row, validating the path.
fn build_track(row: TrackRow) -> Result<Track, DbError> {
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
        album_artist,
        composer,
    ) = row;
    let path = TrackPath::new(path).map_err(|e| DbError::InvalidData(e.to_string()))?;
    Ok(Track {
        id: TrackId::new(id),
        path,
        title: Title::new(or_empty(title)),
        artist: Artist::new(or_empty(artist)),
        album_artist: Artist::new(or_empty(album_artist)),
        album: AlbumTitle::new(or_empty(album)),
        genre: Genre::new(or_empty(genre)),
        composer: Composer::new(or_empty(composer)),
        duration: TrackDuration::from_millis(or_zero(duration_ms) as u64),
        track_number: TrackNumber::new(or_zero(track_num) as u32),
        disc_number: DiscNumber::new(or_zero(disc_num) as u32),
        year: Year::new(or_zero(year) as u16),
        art: art.map(AlbumArtData::new),
    })
}

/// Raw columns of one grouped album row: album, artist, genre, year, art.
type AlbumRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<Vec<u8>>,
);

/// Builds an `AlbumSummary` from a grouped row. All fields are infallible;
/// NULLs map to the domain "unknown" defaults.
fn build_album_summary(row: AlbumRow) -> AlbumSummary {
    let (album, artist, genre, year, art) = row;
    AlbumSummary {
        album: AlbumTitle::new(or_empty(album)),
        artist: Artist::new(or_empty(artist)),
        genre: Genre::new(or_empty(genre)),
        year: Year::new(or_zero(year) as u16),
        art: art.map(AlbumArtData::new),
    }
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
        self.add_missing_track_columns()?;
        Ok(())
    }

    /// Adds columns introduced after the original schema to `tracks` when a
    /// database created by an earlier version lacks them. `CREATE TABLE IF NOT
    /// EXISTS` never alters an existing table, so extra columns need an explicit
    /// `ALTER TABLE` guarded by the current column set.
    fn add_missing_track_columns(&self) -> Result<(), DbError> {
        let existing = self.track_columns()?;
        for (name, decl) in [("album_artist", "TEXT"), ("composer", "TEXT")] {
            if !existing.iter().any(|c| c == name) {
                self.conn
                    .execute(&format!("ALTER TABLE tracks ADD COLUMN {name} {decl}"), [])?;
            }
        }
        Ok(())
    }

    /// The column names of the `tracks` table, in declaration order.
    fn track_columns(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(tracks)")?;
        stmt.query_map([], |row| row.get::<_, String>(1))?
            .map(|r| r.map_err(DbError::from))
            .collect()
    }

    pub fn add_folder(&self, folder: &LibraryFolder) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO folders (path) VALUES (?1)",
            [folder.as_path().to_string_lossy().as_ref()],
        )?;
        Ok(())
    }

    /// Removes a watched folder and every track indexed beneath it. Both deletes
    /// run in one transaction so the folder and its tracks vanish together.
    pub fn remove_folder(&self, folder: &LibraryFolder) -> Result<(), DbError> {
        let folder_str = folder.as_path().to_string_lossy();
        // Nested files share this prefix; the trailing separator stops a sibling
        // like ".../MusicOld" from matching ".../Music".
        let prefix = format!("{folder_str}/");

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM tracks WHERE substr(path, 1, length(?1)) = ?1",
            [&prefix],
        )?;
        tx.execute("DELETE FROM folders WHERE path = ?1", [folder_str.as_ref()])?;
        tx.commit()?;
        Ok(())
    }

    /// Inserts or updates a track keyed on its path. Returns the assigned track_id.
    pub fn upsert_track(&self, track: &Track) -> Result<TrackId, DbError> {
        let path = track.path.as_path().to_string_lossy();
        let title = (!track.title.is_unknown()).then(|| track.title.as_str().to_owned());
        let artist = (!track.artist.is_unknown()).then(|| track.artist.as_str().to_owned());
        let album_artist =
            (!track.album_artist.is_unknown()).then(|| track.album_artist.as_str().to_owned());
        let composer = (!track.composer.is_unknown()).then(|| track.composer.as_str().to_owned());
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
            "INSERT INTO tracks (path, title, artist, album, genre, duration_ms, track_number, disc_number, year, art, album_artist, composer)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(path) DO UPDATE SET
                 title        = excluded.title,
                 artist       = excluded.artist,
                 album        = excluded.album,
                 genre        = excluded.genre,
                 duration_ms  = excluded.duration_ms,
                 track_number = excluded.track_number,
                 disc_number  = excluded.disc_number,
                 year         = excluded.year,
                 art          = excluded.art,
                 album_artist = excluded.album_artist,
                 composer     = excluded.composer
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
                album_artist,
                composer,
            ],
            |row| row.get::<_, i64>(0),
        )?;

        Ok(TrackId::new(id))
    }

    /// Returns all tracks ordered by artist, then album, then track number.
    pub fn list_tracks(&self) -> Result<Vec<Track>, DbError> {
        self.query_tracks("ORDER BY artist, album, track_number", [])
    }

    /// Returns the track with `id`, or `None` when no such row exists. Used to
    /// rebuild a persisted queue from its stored track ids.
    pub fn track_by_id(&self, id: TrackId) -> Result<Option<Track>, DbError> {
        Ok(self
            .query_tracks("WHERE track_id = ?1", [id.value()])?
            .into_iter()
            .next())
    }

    /// Returns tracks whose genre equals `genre`, ordered by artist, then album, then track number.
    pub fn tracks_by_genre(&self, genre: &Genre) -> Result<Vec<Track>, DbError> {
        self.query_tracks(
            "WHERE genre = ?1 ORDER BY artist, album, track_number",
            [genre.as_str()],
        )
    }

    /// Returns tracks by `artist`, ordered by album, then track number.
    pub fn tracks_by_artist(&self, artist: &Artist) -> Result<Vec<Track>, DbError> {
        self.query_tracks(
            "WHERE artist = ?1 ORDER BY album, track_number",
            [artist.as_str()],
        )
    }

    /// Returns tracks on `album`, ordered by disc, then track number.
    pub fn tracks_by_album(&self, album: &AlbumTitle) -> Result<Vec<Track>, DbError> {
        self.query_tracks(
            "WHERE album = ?1 ORDER BY disc_number, track_number",
            [album.as_str()],
        )
    }

    /// Runs a track query with the given `WHERE`/`ORDER BY` clause and parameters.
    fn query_tracks(
        &self,
        filter_and_order: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<Track>, DbError> {
        let sql = format!(
            "SELECT track_id, path, title, artist, album, genre,
                    duration_ms, track_number, disc_number, year, art,
                    album_artist, composer
             FROM tracks {filter_and_order}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params, |row| {
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
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })?;
        rows.map(|r| build_track(r.map_err(DbError::from)?))
            .collect()
    }

    /// Distinct non-empty genres present in the library, alphabetically ordered.
    pub fn distinct_genres(&self) -> Result<Vec<Genre>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT genre FROM tracks WHERE genre IS NOT NULL ORDER BY genre")?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .map(|r| r.map_err(DbError::from).map(Genre::new))
            .collect()
    }

    /// Distinct non-empty artists present in the library, alphabetically ordered.
    pub fn distinct_artists(&self) -> Result<Vec<Artist>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT artist FROM tracks WHERE artist IS NOT NULL ORDER BY artist",
        )?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .map(|r| r.map_err(DbError::from).map(Artist::new))
            .collect()
    }

    /// Distinct non-empty albums present in the library, alphabetically ordered.
    pub fn distinct_albums(&self) -> Result<Vec<AlbumTitle>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT album FROM tracks WHERE album IS NOT NULL ORDER BY album")?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .map(|r| r.map_err(DbError::from).map(AlbumTitle::new))
            .collect()
    }

    /// Returns one summary per (album, album artist) pair for the album grid,
    /// ordered by album artist then album. The album artist is
    /// `COALESCE(album_artist, artist)`, so a compilation credited to one album
    /// artist collapses to a single entry even when its tracks name different
    /// performers, and a track without an album-artist tag falls back to its own
    /// artist. Genre, year, and cover art are aggregated with `MAX`, which skips
    /// NULLs — so a summary carries art from any track in the group that has it,
    /// even when others don't.
    pub fn album_summaries(&self) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("", [])
    }

    /// Album summaries whose genre equals `genre`.
    pub fn album_summaries_by_genre(&self, genre: &Genre) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("WHERE genre = ?1", [genre.as_str()])
    }

    /// Album summaries by `artist`.
    pub fn album_summaries_by_artist(&self, artist: &Artist) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("WHERE artist = ?1", [artist.as_str()])
    }

    /// Album summaries whose album title equals `album`.
    pub fn album_summaries_by_album(
        &self,
        album: &AlbumTitle,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("WHERE album = ?1", [album.as_str()])
    }

    /// Runs the album-summary aggregate with the given `WHERE` clause. Albums are
    /// grouped by their album artist, `COALESCE(album_artist, artist)`, so a
    /// compilation collapses to one entry and a missing album-artist tag falls
    /// back to the track artist. `MAX` on genre/year/art skips NULLs, so a summary
    /// carries art from any track in the group that has it.
    fn album_summaries_query(
        &self,
        where_clause: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        let sql = format!(
            "SELECT album, COALESCE(album_artist, artist) AS album_artist,
                    MAX(genre), MAX(year), MAX(art)
             FROM tracks {where_clause}
             GROUP BY album, album_artist
             ORDER BY album_artist, album"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        stmt.query_map(params, |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<Vec<u8>>>(4)?,
            ))
        })?
        .map(|r| r.map(build_album_summary).map_err(DbError::from))
        .collect()
    }

    /// Stores `value` under `key`, overwriting any existing value.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Returns the value stored under `key`, or `None` when unset.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, DbError> {
        let value = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        Ok(value)
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
    use crate::library::track::AlbumTitle;
    use crate::library::track::Artist;
    use crate::library::track::DiscNumber;
    use crate::library::track::Genre;
    use crate::library::track::Title;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::TrackPath;
    use crate::library::track::Year;

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

    #[test]
    fn remove_folder_deletes_its_tracks_only() {
        let db = Db::open_in_memory().unwrap();
        let music = LibraryFolder::new("/home/user/Music").unwrap();
        let downloads = LibraryFolder::new("/home/user/Downloads").unwrap();
        db.add_folder(&music).unwrap();
        db.add_folder(&downloads).unwrap();
        db.upsert_track(&full_track("/home/user/Music/boc/roygbiv.flac"))
            .unwrap();
        db.upsert_track(&full_track("/home/user/Downloads/aphex/xtal.flac"))
            .unwrap();

        db.remove_folder(&music).unwrap();

        let remaining = db.list_tracks().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].path.as_path().to_str().unwrap(),
            "/home/user/Downloads/aphex/xtal.flac"
        );
    }

    #[test]
    fn remove_folder_spares_sibling_folder_with_shared_prefix() {
        let db = Db::open_in_memory().unwrap();
        let music = LibraryFolder::new("/home/user/Music").unwrap();
        db.add_folder(&music).unwrap();
        db.upsert_track(&full_track("/home/user/Music/boc/roygbiv.flac"))
            .unwrap();
        // Same string prefix, different folder — must survive.
        db.upsert_track(&full_track("/home/user/MusicOld/aphex/xtal.flac"))
            .unwrap();

        db.remove_folder(&music).unwrap();

        let remaining = db.list_tracks().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].path.as_path().to_str().unwrap(),
            "/home/user/MusicOld/aphex/xtal.flac"
        );
    }

    fn minimal_track(path: &str) -> Track {
        Track {
            id: TrackId::new(0),
            path: TrackPath::new(path).unwrap(),
            title: Title::new(""),
            artist: Artist::new(""),
            album_artist: Artist::new(""),
            album: AlbumTitle::new(""),
            genre: Genre::new(""),
            composer: Composer::new(""),
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
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Music Has the Right to Children"),
            genre: Genre::new("Electronic"),
            composer: Composer::new("Boards of Canada"),
            duration: TrackDuration::from_secs(193),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(1998),
            art: None,
        }
    }

    fn track_tagged(path: &str, artist: &str, album: &str, genre: &str) -> Track {
        Track {
            artist: Artist::new(artist),
            // Match the album artist to the track artist so grouping by album
            // artist behaves like grouping by artist for these fixtures.
            album_artist: Artist::new(artist),
            album: AlbumTitle::new(album),
            genre: Genre::new(genre),
            ..full_track(path)
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
    fn track_by_id_returns_the_matching_track() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        let found = db.track_by_id(id).unwrap();
        assert_eq!(found.unwrap().title.as_str(), "Roygbiv");
    }

    #[test]
    fn track_by_id_returns_none_for_unknown_id() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.track_by_id(TrackId::new(4242)).unwrap(), None);
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
    fn upsert_track_round_trips_album_artist_and_composer() {
        let db = Db::open_in_memory().unwrap();
        let track = Track {
            album_artist: Artist::new("Various Artists"),
            composer: Composer::new("Erik Satie"),
            ..full_track("/music/comp/gymnopedie.flac")
        };
        db.upsert_track(&track).unwrap();

        let stored = db.list_tracks().unwrap();
        assert_eq!(stored[0].album_artist.as_str(), "Various Artists");
        assert_eq!(stored[0].composer.as_str(), "Erik Satie");
    }

    #[test]
    fn list_tracks_maps_absent_album_artist_and_composer_to_unknown() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&minimal_track("/music/unknown.mp3"))
            .unwrap();

        let stored = db.list_tracks().unwrap();
        assert!(stored[0].album_artist.is_unknown());
        assert!(stored[0].composer.is_unknown());
    }

    #[test]
    fn schema_has_album_artist_and_composer_columns() {
        let db = Db::open_in_memory().unwrap();
        let columns = db.track_columns().unwrap();
        assert!(columns.iter().any(|c| c == "album_artist"));
        assert!(columns.iter().any(|c| c == "composer"));
    }

    #[test]
    fn migrate_adds_new_columns_to_a_legacy_tracks_table() {
        // A database from before album_artist/composer existed.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tracks (
                 track_id INTEGER PRIMARY KEY,
                 path     TEXT NOT NULL UNIQUE,
                 title    TEXT
             );",
        )
        .unwrap();
        let db = Db { conn };

        db.migrate().unwrap();

        let columns = db.track_columns().unwrap();
        assert!(columns.iter().any(|c| c == "album_artist"));
        assert!(columns.iter().any(|c| c == "composer"));
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

    #[test]
    fn tracks_by_genre_returns_only_matching_genre() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/roygbiv.flac",
            "Boards of Canada",
            "Music Has the Right to Children",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/aphex/xtal.flac",
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "Ambient",
        ))
        .unwrap();

        let electronic = db.tracks_by_genre(&Genre::new("Electronic")).unwrap();
        assert_eq!(electronic.len(), 1);
        assert_eq!(electronic[0].artist.as_str(), "Boards of Canada");
    }

    #[test]
    fn tracks_by_genre_returns_empty_for_unknown_genre() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        assert!(db.tracks_by_genre(&Genre::new("Jazz")).unwrap().is_empty());
    }

    #[test]
    fn tracks_by_artist_returns_only_matching_artist() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/roygbiv.flac",
            "Boards of Canada",
            "Music Has the Right to Children",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/aphex/xtal.flac",
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "Ambient",
        ))
        .unwrap();

        let aphex = db.tracks_by_artist(&Artist::new("Aphex Twin")).unwrap();
        assert_eq!(aphex.len(), 1);
        assert_eq!(aphex[0].album.as_str(), "Selected Ambient Works 85-92");
    }

    #[test]
    fn tracks_by_album_returns_only_matching_album() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/roygbiv.flac",
            "Boards of Canada",
            "Music Has the Right to Children",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/olsonic.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();

        let geogaddi = db.tracks_by_album(&AlbumTitle::new("Geogaddi")).unwrap();
        assert_eq!(geogaddi.len(), 1);
        assert_eq!(geogaddi[0].album.as_str(), "Geogaddi");
    }

    #[test]
    fn distinct_genres_returns_sorted_unique_values() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/roygbiv.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/olsonic.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/aphex/xtal.flac",
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "Ambient",
        ))
        .unwrap();

        let genres = db.distinct_genres().unwrap();
        assert_eq!(
            genres,
            vec![Genre::new("Ambient"), Genre::new("Electronic")]
        );
    }

    #[test]
    fn distinct_artists_excludes_absent_tag() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        db.upsert_track(&minimal_track("/music/unknown.mp3"))
            .unwrap();

        let artists = db.distinct_artists().unwrap();
        assert_eq!(artists, vec![Artist::new("Boards of Canada")]);
    }

    #[test]
    fn distinct_albums_returns_unique_albums() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/roygbiv.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/olsonic.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();

        let albums = db.distinct_albums().unwrap();
        assert_eq!(albums, vec![AlbumTitle::new("Geogaddi")]);
    }

    #[test]
    fn distinct_genres_empty_for_fresh_db() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.distinct_genres().unwrap().is_empty());
    }

    fn track_with_art(path: &str, album: &str, art: &[u8]) -> Track {
        Track {
            album: AlbumTitle::new(album),
            art: Some(AlbumArtData::new(art.to_vec())),
            ..full_track(path)
        }
    }

    #[test]
    fn album_summaries_empty_for_fresh_db() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.album_summaries().unwrap().is_empty());
    }

    #[test]
    fn album_summaries_collapses_tracks_of_one_album_to_a_single_entry() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        db.upsert_track(&full_track("/music/boc/aquarius.flac"))
            .unwrap();

        let summaries = db.album_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].album.as_str(),
            "Music Has the Right to Children"
        );
        assert_eq!(summaries[0].artist.as_str(), "Boards of Canada");
        assert_eq!(summaries[0].year.value(), 1998);
        assert_eq!(summaries[0].genre.as_str(), "Electronic");
    }

    #[test]
    fn album_summaries_separates_same_title_by_different_artists() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/greatest.flac",
            "Boards of Canada",
            "Greatest Hits",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/queen/greatest.flac",
            "Queen",
            "Greatest Hits",
            "Rock",
        ))
        .unwrap();

        assert_eq!(db.album_summaries().unwrap().len(), 2);
    }

    #[test]
    fn album_summaries_orders_by_artist_then_album() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/geogaddi.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/aphex/saw.flac",
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "Ambient",
        ))
        .unwrap();

        let summaries = db.album_summaries().unwrap();
        assert_eq!(summaries[0].artist.as_str(), "Aphex Twin");
        assert_eq!(summaries[1].artist.as_str(), "Boards of Canada");
    }

    /// A track whose credited album artist differs from its performing artist —
    /// the shape of a compilation entry.
    fn compilation_track(path: &str, artist: &str, album_artist: &str) -> Track {
        Track {
            artist: Artist::new(artist),
            album_artist: Artist::new(album_artist),
            album: AlbumTitle::new("Warp10+3 Remixes"),
            ..full_track(path)
        }
    }

    #[test]
    fn album_summaries_group_compilation_under_one_album_artist() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&compilation_track(
            "/music/warp/track01.flac",
            "Aphex Twin",
            "Various Artists",
        ))
        .unwrap();
        db.upsert_track(&compilation_track(
            "/music/warp/track02.flac",
            "Autechre",
            "Various Artists",
        ))
        .unwrap();

        let summaries = db.album_summaries().unwrap();
        assert_eq!(
            summaries.len(),
            1,
            "one album artist collapses to one entry"
        );
        assert_eq!(summaries[0].artist.as_str(), "Various Artists");
    }

    #[test]
    fn album_summaries_fall_back_to_track_artist_when_album_artist_absent() {
        let db = Db::open_in_memory().unwrap();
        // No album-artist tag: is_unknown means it is stored as NULL.
        let track = Track {
            album_artist: Artist::new(""),
            ..full_track("/music/boc/roygbiv.flac")
        };
        db.upsert_track(&track).unwrap();

        let summaries = db.album_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].artist.as_str(), "Boards of Canada");
    }

    #[test]
    fn album_summaries_by_genre_returns_only_matching_albums() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/geogaddi.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/miles/kind.flac",
            "Miles Davis",
            "Kind of Blue",
            "Jazz",
        ))
        .unwrap();

        let jazz = db.album_summaries_by_genre(&Genre::new("Jazz")).unwrap();
        assert_eq!(jazz.len(), 1);
        assert_eq!(jazz[0].album.as_str(), "Kind of Blue");
    }

    #[test]
    fn album_summaries_by_artist_returns_only_that_artists_albums() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/geogaddi.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/mhtrtc.flac",
            "Boards of Canada",
            "Music Has the Right to Children",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/aphex/saw.flac",
            "Aphex Twin",
            "Selected Ambient Works 85-92",
            "Ambient",
        ))
        .unwrap();

        let boc = db
            .album_summaries_by_artist(&Artist::new("Boards of Canada"))
            .unwrap();
        assert_eq!(boc.len(), 2);
    }

    #[test]
    fn album_summaries_by_album_returns_only_that_album() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/geogaddi.flac",
            "Boards of Canada",
            "Geogaddi",
            "Electronic",
        ))
        .unwrap();
        db.upsert_track(&track_tagged(
            "/music/boc/mhtrtc.flac",
            "Boards of Canada",
            "Music Has the Right to Children",
            "Electronic",
        ))
        .unwrap();

        let geogaddi = db
            .album_summaries_by_album(&AlbumTitle::new("Geogaddi"))
            .unwrap();
        assert_eq!(geogaddi.len(), 1);
        assert_eq!(geogaddi[0].album.as_str(), "Geogaddi");
    }

    #[test]
    fn get_setting_returns_none_for_missing_key() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.get_setting("view_mode").unwrap(), None);
    }

    #[test]
    fn set_setting_persists_value() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("view_mode", "grid").unwrap();
        assert_eq!(
            db.get_setting("view_mode").unwrap(),
            Some("grid".to_owned())
        );
    }

    #[test]
    fn set_setting_overwrites_existing_value() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("view_mode", "grid").unwrap();
        db.set_setting("view_mode", "list").unwrap();
        assert_eq!(
            db.get_setting("view_mode").unwrap(),
            Some("list".to_owned())
        );
    }

    #[test]
    fn album_summaries_carries_cover_art_from_any_track_in_the_album() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        db.upsert_track(&track_with_art(
            "/music/boc/aquarius.flac",
            "Music Has the Right to Children",
            &[0xFF, 0xD8, 0xFF],
        ))
        .unwrap();

        let summaries = db.album_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].art.as_ref().map(AlbumArtData::as_bytes),
            Some(&[0xFF, 0xD8, 0xFF][..])
        );
    }
}
