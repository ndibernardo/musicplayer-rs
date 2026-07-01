use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;

use crate::adapters::db::sqlite::Db;
use crate::adapters::db::sqlite::DbError;
use crate::application::ports::RepositoryError;
use crate::application::ports::library::Library;
use crate::application::ports::scanner::Scanner;
use crate::domain::library::LibraryFolder;
use crate::domain::track::Track;
use crate::domain::track::TrackId;

pub struct SqliteLibrary {
    db: Rc<RefCell<Db>>,
    db_path: PathBuf,
}

impl SqliteLibrary {
    pub fn open(db_path: PathBuf) -> Result<Self, DbError> {
        let db = Db::open(&db_path)?;
        Ok(Self {
            db: Rc::new(RefCell::new(db)),
            db_path,
        })
    }
}

impl Library for SqliteLibrary {
    fn add_folder(&self, folder: &LibraryFolder) -> Result<(), RepositoryError> {
        self.db
            .borrow()
            .add_folder(folder)
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn remove_folder(&self, folder: &LibraryFolder) -> Result<(), RepositoryError> {
        self.db
            .borrow()
            .remove_folder(folder)
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn list_folders(&self) -> Result<Vec<LibraryFolder>, RepositoryError> {
        self.db
            .borrow()
            .list_folders()
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn upsert_track(&self, track: &Track) -> Result<TrackId, RepositoryError> {
        self.db
            .borrow()
            .upsert_track(track)
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn all_tracks(&self) -> Result<Vec<Track>, RepositoryError> {
        self.db
            .borrow()
            .list_tracks()
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }
}

// Wraps a borrowed `Db` as `&dyn Library` for use in the background scan thread.
struct DbLibrary<'a>(&'a Db);

impl Library for DbLibrary<'_> {
    fn add_folder(&self, folder: &LibraryFolder) -> Result<(), RepositoryError> {
        self.0
            .add_folder(folder)
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn remove_folder(&self, folder: &LibraryFolder) -> Result<(), RepositoryError> {
        self.0
            .remove_folder(folder)
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn list_folders(&self) -> Result<Vec<LibraryFolder>, RepositoryError> {
        self.0
            .list_folders()
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn upsert_track(&self, track: &Track) -> Result<TrackId, RepositoryError> {
        self.0
            .upsert_track(track)
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }

    fn all_tracks(&self) -> Result<Vec<Track>, RepositoryError> {
        self.0
            .list_tracks()
            .map_err(|e| RepositoryError::Storage(e.to_string()))
    }
}

impl Scanner for SqliteLibrary {
    fn scan(&self) -> Receiver<Result<u32, String>> {
        let (tx, rx) = mpsc::channel();

        let folders = match self.list_folders() {
            Ok(f) => f,
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
                return rx;
            }
        };

        let db_path = self.db_path.clone();

        std::thread::spawn(move || {
            let scan_db = match Db::open(&db_path) {
                Ok(db) => db,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };

            let db_lib = DbLibrary(&scan_db);
            let mut total = 0u32;
            for folder in &folders {
                match crate::application::scanner::scan_folder(
                    folder,
                    &db_lib,
                    |p| crate::adapters::metadata::lofty::read(p).ok(),
                ) {
                    Ok(n) => total += n,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                }
            }

            let _ = tx.send(Ok(total));
        });

        rx
    }
}
