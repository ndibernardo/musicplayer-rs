use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::Application;
use gtk4::prelude::*;

use crate::library::db::Db;
use crate::player::PlaybackState;
use crate::player::PlayerHandle;

pub fn run(
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    state_rx: mpsc::Receiver<PlaybackState>,
) {
    let app = Application::builder()
        .application_id("io.github.musicplayer_rs")
        .build();

    // `Receiver` is not `Clone`; wrap in `Option` so the closure can `take()` it on first call.
    let state_rx = Rc::new(RefCell::new(Some(state_rx)));

    app.connect_activate(move |app| {
        let Some(rx) = state_rx.borrow_mut().take() else {
            return;
        };
        crate::ui::main_window::build(app, Rc::clone(&db), db_path.clone(), player.clone(), rx)
            .present();
    });

    app.run();
}
