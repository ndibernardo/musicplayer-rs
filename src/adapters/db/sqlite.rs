use crate::domain::library::LibraryFolder;
use rusqlite::Connection;
use std::path::Path;

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

pub struct Db {
    pub(crate) conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
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
    fn list_folders_returns_all_configured_paths() {
        let db = Db::open_in_memory().unwrap();
        db.add_folder(&LibraryFolder::new("/home/user/Music").unwrap()).unwrap();
        db.add_folder(&LibraryFolder::new("/home/user/Downloads").unwrap()).unwrap();
        assert_eq!(db.list_folders().unwrap().len(), 2);
    }
}
