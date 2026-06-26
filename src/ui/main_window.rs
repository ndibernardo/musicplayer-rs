use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, Box as GtkBox, HeaderBar, Orientation, Paned};

pub fn build(app: &Application) -> ApplicationWindow {
    let header = HeaderBar::new();

    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.set_width_request(220);

    let content = GtkBox::new(Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);

    let paned = Paned::new(Orientation::Horizontal);
    paned.set_start_child(Some(&sidebar));
    paned.set_end_child(Some(&content));
    paned.set_position(220);
    paned.set_vexpand(true);

    let player_bar = GtkBox::new(Orientation::Horizontal, 0);
    player_bar.set_height_request(80);

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.append(&paned);
    root.append(&player_bar);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Music Player")
        .default_width(1200)
        .default_height(700)
        .child(&root)
        .build();

    window.set_titlebar(Some(&header));

    window
}
