use crate::adapters::db::sqlite::Db;
use crate::adapters::db::sqlite::DbError;
use crate::domain::library::LibraryFolder;

pub fn add_folder(db: &Db, folder: &LibraryFolder) -> Result<(), DbError> {
    db.add_folder(folder)
}

pub fn remove_folder(db: &Db, folder: &LibraryFolder) -> Result<(), DbError> {
    db.remove_folder(folder)
}

pub fn list_folders(db: &Db) -> Result<Vec<LibraryFolder>, DbError> {
    db.list_folders()
}
