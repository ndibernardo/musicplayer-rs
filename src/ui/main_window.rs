use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use glib;
use gtk4::Application;
use gtk4::ApplicationWindow;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::FileDialog;
use gtk4::HeaderBar;
use gtk4::Label;
use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::Paned;
use gtk4::ScrolledWindow;
use gtk4::prelude::*;

use crate::adapters::db::sqlite::Db;
use crate::adapters::metadata::lofty as lofty_adapter;
use crate::application::folders;
use crate::application::scanner;
use crate::application::tracks;
use crate::domain::library::LibraryFolder;
use crate::ui::library_view::LibraryView;

pub fn build(app: &Application, db: Rc<RefCell<Db>>, db_path: PathBuf) -> ApplicationWindow {
    let header = HeaderBar::new();

    let add_btn = Button::from_icon_name("folder-new-symbolic");
    add_btn.set_tooltip_text(Some("Add music folder"));
    header.pack_start(&add_btn);

    let scan_btn = Button::from_icon_name("media-playback-start-symbolic");
    scan_btn.set_tooltip_text(Some("Scan library"));
    header.pack_start(&scan_btn);

    let folder_list = ListBox::new();
    folder_list.set_selection_mode(gtk4::SelectionMode::None);

    let status_label = Label::new(Some("Ready"));
    status_label.set_xalign(0.0);
    status_label.set_margin_start(8);
    status_label.set_margin_end(8);
    status_label.set_margin_top(4);
    status_label.set_margin_bottom(4);

    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.set_width_request(220);

    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&folder_list));
    sidebar.append(&scrolled);
    sidebar.append(&status_label);

    let library_view = LibraryView::new();

    let content = GtkBox::new(Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.append(&library_view.widget);

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

    refresh_folder_list(&folder_list, &db);

    if let Ok(tracks) = tracks::all_tracks(&db.borrow()) {
        library_view.set_tracks(tracks);
    }

    // Add Folder — window captured directly, no btn.root() needed
    {
        let db = Rc::clone(&db);
        let folder_list = folder_list.clone();
        let window = window.clone();
        add_btn.connect_clicked(move |_| {
            let db = Rc::clone(&db);
            let folder_list = folder_list.clone();
            let window = window.clone();
            glib::spawn_future_local(async move {
                let dialog = FileDialog::new();
                dialog.set_title("Add Music Folder");
                let Ok(file) = dialog.select_folder_future(Some(&window)).await else {
                    return;
                };
                let Some(path) = file.path() else { return };
                let Ok(folder) = LibraryFolder::new(path) else {
                    return;
                };
                if let Err(e) = folders::add_folder(&db.borrow(), &folder) {
                    eprintln!("Failed to add folder: {e}");
                    return;
                }
                refresh_folder_list(&folder_list, &db);
            });
        });
    }

    // Scan — background thread opens its own DB connection (WAL allows concurrent access)
    {
        let db = Rc::clone(&db);
        let library_view = library_view.clone();
        let status_label = status_label.clone();
        scan_btn.connect_clicked(move |_| {
            let configured = match folders::list_folders(&db.borrow()) {
                Ok(f) => f,
                Err(e) => {
                    status_label.set_text(&format!("Error: {e}"));
                    return;
                }
            };

            if configured.is_empty() {
                status_label.set_text("No folders configured");
                return;
            }

            status_label.set_text("Scanning…");

            let (tx, rx) = mpsc::channel::<Result<u32, String>>();
            let path = db_path.clone();

            std::thread::spawn(move || {
                let scan_db = match Db::open(&path) {
                    Ok(db) => db,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let mut total = 0u32;
                for folder in &configured {
                    match scanner::scan_folder(folder, &scan_db, |p| lofty_adapter::read(p).ok()) {
                        Ok(n) => total += n,
                        Err(e) => {
                            let _ = tx.send(Err(e.to_string()));
                            return;
                        }
                    }
                }
                let _ = tx.send(Ok(total));
            });

            let db = Rc::clone(&db);
            let library_view = library_view.clone();
            let status_label = status_label.clone();
            glib::idle_add_local(move || match rx.try_recv() {
                Ok(Ok(n)) => {
                    status_label.set_text(&format!("Indexed {n} tracks"));
                    if let Ok(tracks) = tracks::all_tracks(&db.borrow()) {
                        library_view.set_tracks(tracks);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    status_label.set_text(&format!("Scan error: {e}"));
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            });
        });
    }

    window
}

fn refresh_folder_list(list: &ListBox, db: &Rc<RefCell<Db>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let configured = folders::list_folders(&db.borrow()).unwrap_or_default();
    for folder in configured {
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

    let db = Rc::clone(db);
    let list = list.clone();
    remove_btn.connect_clicked(move |_| {
        if let Err(e) = folders::remove_folder(&db.borrow(), &folder) {
            eprintln!("Failed to remove folder: {e}");
            return;
        }
        refresh_folder_list(&list, &db);
    });

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
