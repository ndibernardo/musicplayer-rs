mod adapters;
mod application;
mod domain;
#[cfg(feature = "ui")]
mod ui;

fn main() {
    #[cfg(feature = "ui")]
    run_ui();
}

#[cfg(feature = "ui")]
fn run_ui() {
    use std::rc::Rc;

    use crate::adapters::library::SqliteLibrary;
    use crate::application::ports::library::Library;
    use crate::application::ports::scanner::Scanner;

    let db_path = data_dir().join("library.db");
    let lib = match SqliteLibrary::open(db_path) {
        Ok(lib) => Rc::new(lib),
        Err(e) => {
            eprintln!("Failed to open database: {e}");
            return;
        }
    };

    let library: Rc<dyn Library> = lib.clone();
    let scanner: Rc<dyn Scanner> = lib;

    ui::run(library, scanner);
}

#[cfg(feature = "ui")]
fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".local/share")
        });
    let dir = base.join("musicplayer-rs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
