use crate::application::ports::RepositoryError;
use crate::domain::library::LibraryFolder;
use crate::domain::track::Track;
use crate::domain::track::TrackId;

pub trait Library {
    fn add_folder(&self, folder: &LibraryFolder) -> Result<(), RepositoryError>;
    fn remove_folder(&self, folder: &LibraryFolder) -> Result<(), RepositoryError>;
    fn list_folders(&self) -> Result<Vec<LibraryFolder>, RepositoryError>;
    fn upsert_track(&self, track: &Track) -> Result<TrackId, RepositoryError>;
    fn all_tracks(&self) -> Result<Vec<Track>, RepositoryError>;
}
