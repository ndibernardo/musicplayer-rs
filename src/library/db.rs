use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::library::album::AlbumSort;
use crate::library::album::AlbumSummary;
use crate::library::album::ArtKey;
use crate::library::album::CoverArt;
use crate::library::filter::LibraryFilter;
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
        year         INTEGER
    );

    CREATE TABLE IF NOT EXISTS settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS art_blobs (
        hash INTEGER PRIMARY KEY,
        data BLOB NOT NULL
    );

    CREATE TABLE IF NOT EXISTS track_art (
        track_id INTEGER PRIMARY KEY,
        hash     INTEGER NOT NULL
    );
";

/// 64-bit FNV-1a content hash of `data`, stored as SQLite's signed `INTEGER`
/// via a bit-preserving cast. Must stay FNV-1a forever: unlike
/// `DefaultHasher`, whose keys aren't guaranteed stable across Rust releases,
/// this keeps existing `art_blobs` rows resolving after a toolchain upgrade.
/// A collision only mis-displays a cover, never loses data, and is negligible
/// over the few thousand distinct images a real library has.
pub(crate) fn art_hash(data: &[u8]) -> i64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as i64
}

fn or_empty(v: Option<String>) -> String {
    v.unwrap_or_default()
}

fn or_zero(v: Option<i64>) -> i64 {
    v.unwrap_or(0)
}

/// Raw column values of a `tracks` row, before domain validation.
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
    })
}

/// Raw columns of one grouped album row: album, artist, genre, year, has_art.
type AlbumRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    bool,
);

/// Builds an `AlbumSummary` from a grouped row. All fields are infallible;
/// NULLs map to the domain "unknown" defaults. `has_art` (a plain SQL `EXISTS`)
/// is converted to `CoverArt` here — the bool never travels further than this
/// function.
fn build_album_summary(row: AlbumRow) -> AlbumSummary {
    let (album, artist, genre, year, has_art) = row;
    let album = AlbumTitle::new(or_empty(album));
    let artist = Artist::new(or_empty(artist));
    let art = if has_art {
        ArtKey::new(album.clone(), artist.clone())
            .map(CoverArt::Available)
            .unwrap_or(CoverArt::Absent)
    } else {
        CoverArt::Absent
    };
    AlbumSummary {
        album,
        artist,
        genre: Genre::new(or_empty(genre)),
        year: Year::new(or_zero(year) as u16),
        art,
    }
}

pub struct Db {
    pub(crate) conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        // WAL: concurrent UI reader + background scan writer without blocking.
        // busy_timeout: wait up to 5 s instead of returning SQLITE_BUSY immediately
        // when a second writer (e.g. settings save during a scan) contends.
        // synchronous=NORMAL: safe with WAL and eliminates per-commit fsyncs.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA synchronous=NORMAL;",
        )?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        // busy_timeout matches open() so tests exercise the same configuration.
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), DbError> {
        self.conn.execute_batch(SCHEMA)?;
        // Column migrations must run before index creation: indexes referencing
        // columns that don't exist yet would fail on a legacy database.
        self.add_missing_track_columns()?;
        self.ensure_indexes()?;
        Ok(())
    }

    /// Creates the three query indexes idempotently. Runs after all column
    /// migrations so every referenced column is guaranteed to exist.
    fn ensure_indexes(&self) -> Result<(), DbError> {
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist, album, track_number);
             CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album, disc_number, track_number);
             CREATE INDEX IF NOT EXISTS idx_tracks_genre  ON tracks(genre);",
        )?;
        Ok(())
    }

    /// Ensures every optional column exists in `tracks`. `CREATE TABLE IF NOT
    /// EXISTS` never alters an existing table, so columns added after the initial
    /// creation need an explicit `ALTER TABLE` guarded by the current column set.
    /// Checking all optional columns (not just the most recently added ones) keeps
    /// migration correct for databases created by very old versions.
    fn add_missing_track_columns(&self) -> Result<(), DbError> {
        let existing = self.track_columns()?;
        for (name, decl) in [
            ("title", "TEXT"),
            ("artist", "TEXT"),
            ("album", "TEXT"),
            ("genre", "TEXT"),
            ("duration_ms", "INTEGER"),
            ("track_number", "INTEGER"),
            ("disc_number", "INTEGER"),
            ("year", "INTEGER"),
            ("album_artist", "TEXT"),
            ("composer", "TEXT"),
            ("mtime", "INTEGER"),
            ("size", "INTEGER"),
        ] {
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
        self.prune_orphaned_art()?;
        Ok(())
    }

    /// Upserts `track` via `conn` (which may be a `Transaction` deref'd to
    /// `Connection`) and returns the assigned `track_id`. Uses `prepare_cached`
    /// so the statement is compiled once and reused on subsequent calls with the
    /// same connection.
    pub(crate) fn upsert_one(
        conn: &Connection,
        track: &Track,
        mtime: u64,
        size: u64,
    ) -> Result<TrackId, DbError> {
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

        // RETURNING yields the row's id on both the insert and the update path;
        // last_insert_rowid() would be stale after ON CONFLICT DO UPDATE.
        let mtime = mtime as i64;
        let size = size as i64;
        let mut stmt = conn.prepare_cached(
            "INSERT INTO tracks (path, title, artist, album, genre, duration_ms, track_number, disc_number, year, album_artist, composer, mtime, size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(path) DO UPDATE SET
                 title        = excluded.title,
                 artist       = excluded.artist,
                 album        = excluded.album,
                 genre        = excluded.genre,
                 duration_ms  = excluded.duration_ms,
                 track_number = excluded.track_number,
                 disc_number  = excluded.disc_number,
                 year         = excluded.year,
                 album_artist = excluded.album_artist,
                 composer     = excluded.composer,
                 mtime        = excluded.mtime,
                 size         = excluded.size
             RETURNING track_id",
        )?;
        let id = stmt.query_row(
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
                album_artist,
                composer,
                mtime,
                size,
            ],
            |row| row.get::<_, i64>(0),
        )?;

        Ok(TrackId::new(id))
    }

    /// Upserts cover art for one track, keyed on its content hash. Storing by
    /// hash means an album whose tracks share one embedded image (the common
    /// case) still costs one blob no matter how many tracks point at it.
    pub(crate) fn upsert_art_for_track(
        conn: &Connection,
        track_id: TrackId,
        data: &[u8],
    ) -> Result<(), DbError> {
        let hash = art_hash(data);
        conn.prepare_cached("INSERT OR IGNORE INTO art_blobs (hash, data) VALUES (?1, ?2)")?
            .execute(rusqlite::params![hash, data])?;
        conn.prepare_cached(
            "INSERT INTO track_art (track_id, hash) VALUES (?1, ?2)
             ON CONFLICT(track_id) DO UPDATE SET hash = excluded.hash",
        )?
        .execute(rusqlite::params![track_id.value(), hash])?;
        Ok(())
    }

    /// Returns the cover art embedded in `id`'s own file, if any.
    pub fn art_for_track(&self, id: TrackId) -> Result<Option<AlbumArtData>, DbError> {
        let data = self
            .conn
            .query_row(
                "SELECT ab.data FROM track_art ta
                 JOIN art_blobs ab ON ab.hash = ta.hash
                 WHERE ta.track_id = ?1",
                [id.value()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        Ok(data.map(AlbumArtData::new))
    }

    /// Returns the cover art of the lowest `(disc_number, track_number)` track
    /// in `key`'s album/artist group that has one — the album grid's
    /// representative cover.
    pub fn art_for(&self, key: &ArtKey) -> Result<Option<AlbumArtData>, DbError> {
        let data = self
            .conn
            .query_row(
                "SELECT ab.data FROM tracks t
                 JOIN track_art ta ON ta.track_id = t.track_id
                 JOIN art_blobs ab ON ab.hash = ta.hash
                 WHERE t.album = ?1 AND COALESCE(t.album_artist, t.artist) = ?2
                 ORDER BY t.disc_number, t.track_number
                 LIMIT 1",
                rusqlite::params![key.album().as_str(), key.album_artist().as_str()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        Ok(data.map(AlbumArtData::new))
    }

    /// Inserts or updates a track keyed on its path. Returns the assigned track_id.
    pub fn upsert_track(&self, track: &Track) -> Result<TrackId, DbError> {
        Self::upsert_one(&self.conn, track, 0, 0)
    }

    /// Upserts all `tracks` in a single transaction. Prefer this over calling
    /// `upsert_track` in a loop when indexing a batch of files — one commit per
    /// batch is orders of magnitude faster than one implicit commit per file.
    pub fn upsert_tracks(&self, tracks: &[Track]) -> Result<(), DbError> {
        let tx = self.conn.unchecked_transaction()?;
        for track in tracks {
            Self::upsert_one(&tx, track, 0, 0)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Returns the `(mtime, size)` pair for every track path under `folder`.
    /// Used by the scanner to decide whether a file needs re-indexing.
    pub(crate) fn known_file_stats(
        &self,
        folder: &LibraryFolder,
    ) -> Result<HashMap<PathBuf, (u64, u64)>, DbError> {
        let prefix = format!("{}/", folder.as_path().to_string_lossy());
        let mut stmt = self.conn.prepare_cached(
            "SELECT path, mtime, size FROM tracks WHERE substr(path, 1, length(?1)) = ?1",
        )?;
        let map = stmt
            .query_map([&prefix], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .map(|(path, mtime, size)| {
                (
                    PathBuf::from(path),
                    (mtime.unwrap_or(0) as u64, size.unwrap_or(0) as u64),
                )
            })
            .collect();
        Ok(map)
    }

    /// Deletes tracks whose paths lie under `folder` but are absent from `seen`.
    /// Returns the count of removed rows.
    pub fn remove_stale_tracks(
        &self,
        folder: &LibraryFolder,
        seen: &HashSet<PathBuf>,
    ) -> Result<u64, DbError> {
        let prefix = format!("{}/", folder.as_path().to_string_lossy());
        let existing: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare_cached("SELECT path FROM tracks WHERE substr(path, 1, length(?1)) = ?1")?;
            stmt.query_map([&prefix], |row| row.get::<_, String>(0))?
                .map(|r| r.map_err(DbError::from))
                .collect::<Result<_, _>>()?
        };
        let stale: Vec<String> = existing
            .into_iter()
            .filter(|p| !seen.contains(Path::new(p)))
            .collect();
        if stale.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.unchecked_transaction()?;
        for path in &stale {
            tx.execute("DELETE FROM tracks WHERE path = ?1", [path])?;
        }
        tx.commit()?;
        Ok(stale.len() as u64)
    }

    /// Deletes `album_art` rows for albums no track references any more (e.g.
    /// after a folder or stale-track removal). Returns the count of removed rows.
    pub fn prune_orphaned_art(&self) -> Result<u64, DbError> {
        let n = self.conn.execute(
            "DELETE FROM track_art WHERE track_id NOT IN (SELECT track_id FROM tracks)",
            [],
        )?;
        // A blob is only orphaned once no track_art row points at its hash any more.
        self.conn.execute(
            "DELETE FROM art_blobs WHERE hash NOT IN (SELECT hash FROM track_art)",
            [],
        )?;
        Ok(n as u64)
    }

    /// Returns all tracks ordered by artist, then album, then track number.
    pub fn list_tracks(&self) -> Result<Vec<Track>, DbError> {
        self.query_tracks("ORDER BY artist, album, track_number", [])
    }

    /// Returns the track with `id`, or `None` when no such row exists.
    pub fn track_by_id(&self, id: TrackId) -> Result<Option<Track>, DbError> {
        Ok(self
            .query_tracks("WHERE track_id = ?1", [id.value()])?
            .into_iter()
            .next())
    }

    /// Returns the tracks matching `ids` in the same order as the input slice.
    /// IDs not present in the database are silently skipped. One query replaces
    /// the N-query loop a caller would otherwise need.
    pub fn tracks_by_ids(&self, ids: &[TrackId]) -> Result<Vec<Track>, DbError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let filter = format!("WHERE track_id IN ({placeholders})");
        let params = rusqlite::params_from_iter(ids.iter().map(|id| id.value()));
        let mut tracks = self.query_tracks(&filter, params)?;
        // Re-order to match the caller's id sequence; the IN clause doesn't
        // guarantee ordering relative to the parameter list.
        let position: HashMap<i64, usize> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.value(), i))
            .collect();
        tracks.sort_by_key(|t| position.get(&t.id.value()).copied().unwrap_or(usize::MAX));
        Ok(tracks)
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
                    duration_ms, track_number, disc_number, year,
                    album_artist, composer
             FROM tracks {filter_and_order}"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
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
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })?;
        rows.map(|r| build_track(r.map_err(DbError::from)?))
            .collect()
    }

    /// Distinct non-empty genres present in the library, alphabetically ordered.
    pub fn distinct_genres(&self) -> Result<Vec<Genre>, DbError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT genre FROM tracks WHERE genre IS NOT NULL ORDER BY genre",
        )?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .map(|r| r.map_err(DbError::from).map(Genre::new))
            .collect()
    }

    /// Distinct non-empty artists present in the library, alphabetically ordered.
    pub fn distinct_artists(&self) -> Result<Vec<Artist>, DbError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT artist FROM tracks WHERE artist IS NOT NULL ORDER BY artist",
        )?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .map(|r| r.map_err(DbError::from).map(Artist::new))
            .collect()
    }

    /// Distinct non-empty albums present in the library, alphabetically ordered.
    pub fn distinct_albums(&self) -> Result<Vec<AlbumTitle>, DbError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT DISTINCT album FROM tracks WHERE album IS NOT NULL ORDER BY album",
        )?;
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
        self.album_summaries_sorted(&AlbumSort::default())
    }

    /// Album summaries in `sort` order.
    pub fn album_summaries_sorted(&self, sort: &AlbumSort) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("", [], sort)
    }

    /// Album summaries whose genre equals `genre`.
    pub fn album_summaries_by_genre(&self, genre: &Genre) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_by_genre_sorted(genre, &AlbumSort::default())
    }

    /// Album summaries whose genre equals `genre`, in `sort` order.
    pub fn album_summaries_by_genre_sorted(
        &self,
        genre: &Genre,
        sort: &AlbumSort,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("WHERE genre = ?1", [genre.as_str()], sort)
    }

    /// Album summaries by `artist`.
    pub fn album_summaries_by_artist(&self, artist: &Artist) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_by_artist_sorted(artist, &AlbumSort::default())
    }

    /// Album summaries by `artist`, in `sort` order.
    pub fn album_summaries_by_artist_sorted(
        &self,
        artist: &Artist,
        sort: &AlbumSort,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("WHERE artist = ?1", [artist.as_str()], sort)
    }

    /// Album summaries whose album title equals `album`.
    pub fn album_summaries_by_album(
        &self,
        album: &AlbumTitle,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_by_album_sorted(album, &AlbumSort::default())
    }

    /// Album summaries whose album title equals `album`, in `sort` order.
    pub fn album_summaries_by_album_sorted(
        &self,
        album: &AlbumTitle,
        sort: &AlbumSort,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        self.album_summaries_query("WHERE album = ?1", [album.as_str()], sort)
    }

    /// Returns the tracks matching `filter`. This is the canonical public API;
    /// the per-filter helpers below are `pub(crate)` and exist for internal use.
    pub fn tracks_for(&self, filter: &LibraryFilter) -> Result<Vec<Track>, DbError> {
        match filter {
            LibraryFilter::All => self.list_tracks(),
            LibraryFilter::ByGenre(g) => self.tracks_by_genre(g),
            LibraryFilter::ByArtist(a) => self.tracks_by_artist(a),
            LibraryFilter::ByAlbum(a) => self.tracks_by_album(a),
        }
    }

    /// Returns the album summaries matching `filter`, in `sort` order. This is
    /// the canonical public API; the per-filter helpers below are `pub(crate)`.
    pub fn album_summaries_for(
        &self,
        filter: &LibraryFilter,
        sort: &AlbumSort,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        match filter {
            LibraryFilter::All => self.album_summaries_sorted(sort),
            LibraryFilter::ByGenre(g) => self.album_summaries_by_genre_sorted(g, sort),
            LibraryFilter::ByArtist(a) => self.album_summaries_by_artist_sorted(a, sort),
            LibraryFilter::ByAlbum(a) => self.album_summaries_by_album_sorted(a, sort),
        }
    }

    /// Runs the album-summary aggregate with the given `WHERE` clause. Albums are
    /// grouped by their album artist, `COALESCE(album_artist, artist)`, so a
    /// compilation collapses to one entry and a missing album-artist tag falls
    /// back to the track artist. `MAX` on genre/year skips NULLs. `has_art` is an
    /// `EXISTS` check rather than a data fetch — the grid only needs to know
    /// whether a cover exists, and fetches the bytes separately on demand.
    fn album_summaries_query(
        &self,
        where_clause: &str,
        params: impl rusqlite::Params,
        sort: &AlbumSort,
    ) -> Result<Vec<AlbumSummary>, DbError> {
        let order_by = sort.order_by_clause();
        // Correlated EXISTS avoids a JOIN that would make `album` and
        // `album_artist` ambiguous in GROUP BY and ORDER BY (both tables share
        // those column names). `SELECT 1` never reads the art_blobs BLOB —
        // the join only needs to know a matching track_art row exists.
        let sql = format!(
            "SELECT album,
                    COALESCE(album_artist, artist) AS album_artist,
                    MAX(genre) AS album_genre,
                    MAX(year)  AS album_year,
                    EXISTS(SELECT 1 FROM tracks t2
                           JOIN track_art ta ON ta.track_id = t2.track_id
                           WHERE t2.album = tracks.album
                             AND COALESCE(t2.album_artist, t2.artist)
                                 = COALESCE(tracks.album_artist, tracks.artist))
             FROM tracks
             {where_clause}
             GROUP BY album, album_artist
             {order_by}"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        stmt.query_map(params, |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, bool>(4)?,
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
            .prepare_cached("SELECT path FROM folders ORDER BY path")?;

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
    use crate::library::album::AlbumSortField;
    use crate::library::album::SortDirection;
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
    fn album_summaries_sorted_by_year_descending_orders_newest_first() {
        let db = Db::open_in_memory().unwrap();
        let older = Track {
            year: Year::new(1998),
            ..track_tagged(
                "/music/boc/mhtrtc.flac",
                "Boards of Canada",
                "Music Has the Right to Children",
                "Electronic",
            )
        };
        let newer = Track {
            year: Year::new(2002),
            ..track_tagged(
                "/music/boc/geogaddi.flac",
                "Boards of Canada",
                "Geogaddi",
                "Electronic",
            )
        };
        db.upsert_track(&older).unwrap();
        db.upsert_track(&newer).unwrap();

        let sort = AlbumSort::new(AlbumSortField::Year, SortDirection::Descending);
        let summaries = db.album_summaries_sorted(&sort).unwrap();
        assert_eq!(summaries[0].album.as_str(), "Geogaddi");
        assert_eq!(
            summaries[1].album.as_str(),
            "Music Has the Right to Children"
        );
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
    fn upsert_tracks_inserts_all_in_one_transaction() {
        let db = Db::open_in_memory().unwrap();
        let tracks = vec![
            full_track("/music/boc/roygbiv.flac"),
            full_track("/music/boc/aquarius.flac"),
        ];
        db.upsert_tracks(&tracks).unwrap();
        assert_eq!(db.track_count().unwrap(), 2);
    }

    #[test]
    fn upsert_tracks_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let tracks = vec![full_track("/music/boc/roygbiv.flac")];
        db.upsert_tracks(&tracks).unwrap();
        db.upsert_tracks(&tracks).unwrap();
        assert_eq!(db.track_count().unwrap(), 1);
    }

    #[test]
    fn tracks_by_ids_returns_empty_for_empty_input() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.tracks_by_ids(&[]).unwrap().is_empty());
    }

    #[test]
    fn tracks_by_ids_skips_ids_not_in_the_library() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        let unknown = TrackId::new(9999);
        let tracks = db.tracks_by_ids(&[id, unknown]).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_str(), "Roygbiv");
    }

    #[test]
    fn tracks_by_ids_preserves_input_order() {
        let db = Db::open_in_memory().unwrap();
        let id1 = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        let id2 = db
            .upsert_track(&full_track("/music/boc/aquarius.flac"))
            .unwrap();
        // Request in reverse insertion order.
        let tracks = db.tracks_by_ids(&[id2, id1]).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].id, id2);
        assert_eq!(tracks[1].id, id1);
    }

    fn geogaddi_key() -> ArtKey {
        ArtKey::new(AlbumTitle::new("Geogaddi"), Artist::new("Boards of Canada")).unwrap()
    }

    fn mhtrtc_key() -> ArtKey {
        ArtKey::new(
            AlbumTitle::new("Music Has the Right to Children"),
            Artist::new("Boards of Canada"),
        )
        .unwrap()
    }

    fn art_blob_count(db: &Db) -> u32 {
        db.conn
            .query_row("SELECT COUNT(*) FROM art_blobs", [], |row| {
                row.get::<_, u32>(0)
            })
            .unwrap()
    }

    #[test]
    fn album_summaries_reports_has_art_when_a_track_has_art() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        // Art is stored per track, not in the track row itself.
        Db::upsert_art_for_track(&db.conn, id, &[0xFF, 0xD8, 0xFF]).unwrap();

        let summaries = db.album_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].art, CoverArt::Available(mhtrtc_key()));
    }

    #[test]
    fn album_summaries_reports_no_art_when_no_track_has_art() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();

        let summaries = db.album_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].art, CoverArt::Absent);
    }

    #[test]
    fn art_for_returns_stored_bytes() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .upsert_track(&track_tagged(
                "/music/boc/geogaddi.flac",
                "Boards of Canada",
                "Geogaddi",
                "Electronic",
            ))
            .unwrap();
        Db::upsert_art_for_track(&db.conn, id, &[0xFF, 0xD8, 0xFF]).unwrap();

        let art = db.art_for(&geogaddi_key()).unwrap();
        assert_eq!(
            art.as_ref().map(AlbumArtData::as_bytes),
            Some(&[0xFF, 0xD8, 0xFF][..])
        );
    }

    #[test]
    fn art_for_returns_none_for_unknown_album() {
        let db = Db::open_in_memory().unwrap();
        let key = ArtKey::new(AlbumTitle::new("Missing"), Artist::new("Nobody")).unwrap();
        let art = db.art_for(&key).unwrap();
        assert!(art.is_none());
    }

    #[test]
    fn art_for_track_returns_the_bytes_embedded_in_that_track() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        Db::upsert_art_for_track(&db.conn, id, &[0xFF, 0xD8, 0xFF]).unwrap();

        let art = db.art_for_track(id).unwrap();
        assert_eq!(
            art.as_ref().map(AlbumArtData::as_bytes),
            Some(&[0xFF, 0xD8, 0xFF][..])
        );
    }

    #[test]
    fn art_for_track_returns_none_when_the_track_has_no_art() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        assert!(db.art_for_track(id).unwrap().is_none());
    }

    #[test]
    fn art_for_returns_the_lowest_disc_and_track_number_cover() {
        let db = Db::open_in_memory().unwrap();
        let mut second = full_track("/music/boc/aquarius.flac");
        second.track_number = TrackNumber::new(2);
        let mut first = full_track("/music/boc/roygbiv.flac");
        first.track_number = TrackNumber::new(1);
        let second_id = db.upsert_track(&second).unwrap();
        let first_id = db.upsert_track(&first).unwrap();
        Db::upsert_art_for_track(&db.conn, second_id, &[0xAA]).unwrap();
        Db::upsert_art_for_track(&db.conn, first_id, &[0xBB]).unwrap();

        let art = db.art_for(&mhtrtc_key()).unwrap().unwrap();
        assert_eq!(art.as_bytes(), &[0xBB][..]);
    }

    #[test]
    fn upsert_art_for_track_stores_identical_bytes_once() {
        let db = Db::open_in_memory().unwrap();
        let id1 = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        let id2 = db
            .upsert_track(&full_track("/music/boc/aquarius.flac"))
            .unwrap();
        Db::upsert_art_for_track(&db.conn, id1, &[0xFF, 0xD8, 0xFF]).unwrap();
        Db::upsert_art_for_track(&db.conn, id2, &[0xFF, 0xD8, 0xFF]).unwrap();

        assert_eq!(
            art_blob_count(&db),
            1,
            "identical bytes across two tracks must share one blob row"
        );
    }

    #[test]
    fn art_hash_is_stable() {
        assert_eq!(art_hash(b""), -3750763034362895579);
        assert_eq!(art_hash(b"geogaddi"), 6552777080131098409);
    }

    #[test]
    fn prune_orphaned_art_removes_art_for_albums_with_no_surviving_tracks() {
        let db = Db::open_in_memory().unwrap();
        // A track_art row with no matching tracks row — orphaned from the start.
        Db::upsert_art_for_track(&db.conn, TrackId::new(1), &[0xFF, 0xD8, 0xFF]).unwrap();

        let removed = db.prune_orphaned_art().unwrap();

        assert_eq!(removed, 1);
        assert_eq!(art_blob_count(&db), 0);
    }

    #[test]
    fn prune_orphaned_art_keeps_art_for_albums_with_surviving_tracks() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        Db::upsert_art_for_track(&db.conn, id, &[0xFF, 0xD8, 0xFF]).unwrap();

        let removed = db.prune_orphaned_art().unwrap();

        assert_eq!(removed, 0);
        assert!(db.art_for(&mhtrtc_key()).unwrap().is_some());
    }

    #[test]
    fn prune_orphaned_art_removes_unreferenced_blobs() {
        let db = Db::open_in_memory().unwrap();
        Db::upsert_art_for_track(&db.conn, TrackId::new(1), &[0xFF, 0xD8, 0xFF]).unwrap();

        db.prune_orphaned_art().unwrap();

        assert_eq!(art_blob_count(&db), 0);
    }

    #[test]
    fn remove_folder_prunes_orphaned_art() {
        let db = Db::open_in_memory().unwrap();
        let music = LibraryFolder::new("/home/user/Music").unwrap();
        db.add_folder(&music).unwrap();
        let id = db
            .upsert_track(&full_track("/home/user/Music/boc/roygbiv.flac"))
            .unwrap();
        Db::upsert_art_for_track(&db.conn, id, &[0xFF, 0xD8, 0xFF]).unwrap();

        db.remove_folder(&music).unwrap();

        assert!(db.art_for(&mhtrtc_key()).unwrap().is_none());
        assert_eq!(art_blob_count(&db), 0);
    }

    #[test]
    fn schema_has_no_art_column_on_a_fresh_database() {
        let db = Db::open_in_memory().unwrap();
        let columns = db.track_columns().unwrap();
        assert!(!columns.iter().any(|c| c == "art"));
    }

    #[test]
    fn known_file_stats_returns_mtime_and_size_for_indexed_paths() {
        let db = Db::open_in_memory().unwrap();
        let track = full_track("/music/boc/roygbiv.flac");
        Db::upsert_one(&db.conn, &track, 1_700_000_000, 4096).unwrap();

        let folder = LibraryFolder::new("/music/boc").unwrap();
        let stats = db.known_file_stats(&folder).unwrap();

        assert_eq!(
            stats.get(&PathBuf::from("/music/boc/roygbiv.flac")),
            Some(&(1_700_000_000, 4096))
        );
    }

    #[test]
    fn known_file_stats_excludes_paths_outside_the_folder() {
        let db = Db::open_in_memory().unwrap();
        Db::upsert_one(&db.conn, &full_track("/music/boc/roygbiv.flac"), 100, 200).unwrap();
        Db::upsert_one(&db.conn, &full_track("/other/track.flac"), 300, 400).unwrap();

        let folder = LibraryFolder::new("/music/boc").unwrap();
        let stats = db.known_file_stats(&folder).unwrap();

        assert_eq!(stats.len(), 1);
        assert!(stats.contains_key(&PathBuf::from("/music/boc/roygbiv.flac")));
    }

    #[test]
    fn remove_stale_tracks_deletes_paths_absent_from_seen() {
        let db = Db::open_in_memory().unwrap();
        let id1 = db
            .upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        let id2 = db
            .upsert_track(&full_track("/music/boc/aquarius.flac"))
            .unwrap();

        let seen: HashSet<PathBuf> = [PathBuf::from("/music/boc/roygbiv.flac")]
            .into_iter()
            .collect();
        let folder = LibraryFolder::new("/music/boc").unwrap();
        let removed = db.remove_stale_tracks(&folder, &seen).unwrap();

        assert_eq!(removed, 1);
        assert!(
            db.track_by_id(id1).unwrap().is_some(),
            "seen track must survive"
        );
        assert!(
            db.track_by_id(id2).unwrap().is_none(),
            "unseen track must be removed"
        );
    }

    #[test]
    fn remove_stale_tracks_returns_zero_when_all_paths_are_seen() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();

        let seen: HashSet<PathBuf> = [PathBuf::from("/music/boc/roygbiv.flac")]
            .into_iter()
            .collect();
        let folder = LibraryFolder::new("/music/boc").unwrap();
        let removed = db.remove_stale_tracks(&folder, &seen).unwrap();

        assert_eq!(removed, 0);
        assert_eq!(db.track_count().unwrap(), 1);
    }

    #[test]
    fn remove_stale_tracks_spares_paths_outside_the_folder() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&full_track("/music/boc/roygbiv.flac"))
            .unwrap();
        db.upsert_track(&full_track("/other/track.flac")).unwrap();

        // Scan only /music/boc; the other track is outside and must not be touched.
        let seen: HashSet<PathBuf> = HashSet::new();
        let folder = LibraryFolder::new("/music/boc").unwrap();
        db.remove_stale_tracks(&folder, &seen).unwrap();

        assert_eq!(
            db.track_count().unwrap(),
            1,
            "/other/track.flac must survive"
        );
    }

    fn two_artist_library() -> Db {
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
        db
    }

    #[test]
    fn tracks_for_all_returns_every_track() {
        let db = two_artist_library();
        assert_eq!(db.tracks_for(&LibraryFilter::All).unwrap().len(), 2);
    }

    #[test]
    fn tracks_for_by_genre_returns_only_that_genre() {
        let db = two_artist_library();
        let tracks = db
            .tracks_for(&LibraryFilter::ByGenre(Genre::new("Ambient")))
            .unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].artist.as_str(), "Aphex Twin");
    }

    #[test]
    fn tracks_for_by_artist_returns_only_that_artist() {
        let db = two_artist_library();
        let tracks = db
            .tracks_for(&LibraryFilter::ByArtist(Artist::new("Boards of Canada")))
            .unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].album.as_str(), "Music Has the Right to Children");
    }

    #[test]
    fn tracks_for_by_album_returns_only_that_album() {
        let db = two_artist_library();
        let tracks = db
            .tracks_for(&LibraryFilter::ByAlbum(AlbumTitle::new(
                "Selected Ambient Works 85-92",
            )))
            .unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].genre.as_str(), "Ambient");
    }

    #[test]
    fn album_summaries_for_all_returns_every_album() {
        let db = two_artist_library();
        assert_eq!(
            db.album_summaries_for(&LibraryFilter::All, &AlbumSort::default())
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn album_summaries_for_by_genre_returns_only_that_genre() {
        let db = two_artist_library();
        let albums = db
            .album_summaries_for(
                &LibraryFilter::ByGenre(Genre::new("Ambient")),
                &AlbumSort::default(),
            )
            .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist.as_str(), "Aphex Twin");
    }

    #[test]
    fn album_summaries_for_by_artist_returns_only_that_artist() {
        let db = two_artist_library();
        let albums = db
            .album_summaries_for(
                &LibraryFilter::ByArtist(Artist::new("Boards of Canada")),
                &AlbumSort::default(),
            )
            .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].album.as_str(), "Music Has the Right to Children");
    }

    #[test]
    fn album_summaries_for_by_album_returns_only_that_album() {
        let db = two_artist_library();
        let albums = db
            .album_summaries_for(
                &LibraryFilter::ByAlbum(AlbumTitle::new("Selected Ambient Works 85-92")),
                &AlbumSort::default(),
            )
            .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].genre.as_str(), "Ambient");
    }
}
