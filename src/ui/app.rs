use std::rc::Rc;

use gtk4::Application;
use gtk4::prelude::*;

use crate::application::ports::library::Library;
use crate::application::ports::scanner::Scanner;

pub fn run(library: Rc<dyn Library>, scanner: Rc<dyn Scanner>) {
    let app = Application::builder()
        .application_id("io.github.musicplayer_rs")
        .build();

    app.connect_activate(move |app| {
        crate::ui::main_window::build(app, Rc::clone(&library), Rc::clone(&scanner)).present();
    });

    app.run();
}
