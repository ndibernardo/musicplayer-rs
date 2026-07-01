use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::Application;
use gtk4::prelude::*;

use crate::application::player::PlayerHandle;
use crate::application::ports::library::Library;
use crate::application::ports::scanner::Scanner;
use crate::domain::player::PlaybackState;

pub fn run(
    library: Rc<dyn Library>,
    scanner: Rc<dyn Scanner>,
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
        crate::ui::main_window::build(
            app,
            Rc::clone(&library),
            Rc::clone(&scanner),
            player.clone(),
            rx,
        )
        .present();
    });

    app.run();
}
