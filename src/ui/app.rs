use std::path::PathBuf;
use std::rc::Rc;

use gtk4::Application;
use gtk4::prelude::*;

use crate::library::db::Db;
use crate::player::PlaybackState;
use crate::player::PlayerHandle;

pub fn run(
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    state_rx: async_channel::Receiver<PlaybackState>,
) {
    let app = Application::builder()
        .application_id("io.github.musicplayer_rs")
        .build();

    app.connect_activate(move |app| {
        crate::ui::main_window::build(
            app,
            Rc::clone(&db),
            db_path.clone(),
            player.clone(),
            state_rx.clone(),
        )
        .present();
    });

    app.run();
}
