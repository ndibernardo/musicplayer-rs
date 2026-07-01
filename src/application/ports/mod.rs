pub mod audio;
pub mod library;
pub mod scanner;

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("{0}")]
    Storage(String),
}
