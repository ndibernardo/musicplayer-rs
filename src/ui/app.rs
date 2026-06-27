use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::Application;

use crate::adapters::db::sqlite::Db;

pub fn run() {
    let app = Application::builder()
        .application_id("io.github.musicplayer_rs")
        .build();

    app.connect_activate(|app| {
        let db_path = data_dir().join("library.db");
        let db = match Db::open(&db_path) {
            Ok(db) => Rc::new(RefCell::new(db)),
            Err(e) => {
                eprintln!("Failed to open database: {e}");
                return;
            }
        };
        crate::ui::main_window::build(app, db, db_path).present();
    });

    app.run();
}

fn data_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
        });
    let dir = base.join("musicplayer-rs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
