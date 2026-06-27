use crate::adapters::db::sqlite::Db;
use crate::domain::library::LibraryFolder;
use glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, Button, FileDialog, HeaderBar, Label,
    ListBox, ListBoxRow, Orientation, Paned, ScrolledWindow,
};
use std::cell::RefCell;
use std::rc::Rc;

pub fn build(app: &Application, db: Rc<RefCell<Db>>) -> ApplicationWindow {
    let header = HeaderBar::new();

    let add_btn = Button::from_icon_name("folder-new-symbolic");
    add_btn.set_tooltip_text(Some("Add music folder"));
    header.pack_start(&add_btn);

    let folder_list = ListBox::new();
    folder_list.set_selection_mode(gtk4::SelectionMode::None);

    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.set_width_request(220);

    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&folder_list));
    sidebar.append(&scrolled);

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

    // Populate folder list on startup
    refresh_folder_list(&folder_list, &db);

    // Add Folder button
    let db_clone = db.clone();
    let folder_list_clone = folder_list.clone();
    add_btn.connect_clicked(move |btn| {
        let window = btn
            .root()
            .and_downcast::<gtk4::Window>()
            .expect("button must have a root window");
        let db = db_clone.clone();
        let folder_list = folder_list_clone.clone();

        glib::spawn_future_local(async move {
            let dialog = FileDialog::new();
            dialog.set_title("Add Music Folder");
            let Ok(file) = dialog.select_folder_future(Some(&window)).await else {
                return; // user cancelled
            };
            let Some(path) = file.path() else { return };
            let Ok(folder) = LibraryFolder::new(path) else { return };

            if let Err(e) = db.borrow().add_folder(&folder) {
                eprintln!("Failed to add folder: {e}");
                return;
            }
            refresh_folder_list(&folder_list, &db);
        });
    });

    window
}

fn refresh_folder_list(list: &ListBox, db: &Rc<RefCell<Db>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let folders = db.borrow().list_folders().unwrap_or_default();
    for folder in folders {
        list.append(&folder_row(folder, list, db));
    }
}

fn folder_row(folder: LibraryFolder, list: &ListBox, db: &Rc<RefCell<Db>>) -> ListBoxRow {
    let path_label = Label::new(folder.as_path().to_str());
    path_label.set_hexpand(true);
    path_label.set_xalign(0.0);
    path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    path_label.set_margin_start(8);

    let remove_btn = Button::from_icon_name("list-remove-symbolic");
    remove_btn.add_css_class("flat");
    remove_btn.set_margin_end(4);

    let row_box = GtkBox::new(Orientation::Horizontal, 0);
    row_box.set_margin_top(2);
    row_box.set_margin_bottom(2);
    row_box.append(&path_label);
    row_box.append(&remove_btn);

    let db_clone = db.clone();
    let list_clone = list.clone();
    remove_btn.connect_clicked(move |_| {
        if let Err(e) = db_clone.borrow().remove_folder(&folder) {
            eprintln!("Failed to remove folder: {e}");
            return;
        }
        refresh_folder_list(&list_clone, &db_clone);
    });

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
