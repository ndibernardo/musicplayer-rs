use gtk4::prelude::*;
use gtk4::Application;

pub fn run() {
    let app = Application::builder()
        .application_id("io.github.musicplayer_rs")
        .build();

    app.connect_activate(|app| {
        crate::ui::main_window::build(app).present();
    });

    app.run();
}
