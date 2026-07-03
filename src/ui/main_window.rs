use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

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

use crate::application::player::PlayerHandle;
use crate::application::ports::library::Library;
use crate::application::ports::scanner::Scanner;
use crate::domain::library::LibraryFolder;
use crate::domain::player::PlaybackState;
use crate::domain::player::PlayerCommand;
use crate::ui::library_view::LibraryView;
use crate::ui::player_bar::PlayerBar;

pub fn build(
    app: &Application,
    library: Rc<dyn Library>,
    scanner: Rc<dyn Scanner>,
    player: PlayerHandle,
    state_rx: mpsc::Receiver<PlaybackState>,
) -> ApplicationWindow {
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

    let player_bar = PlayerBar::new(player.clone());

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.append(&paned);
    root.append(&player_bar.widget);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Music Player")
        .default_width(1200)
        .default_height(700)
        .child(&root)
        .build();

    window.set_titlebar(Some(&header));

    refresh_folder_list(&folder_list, &library);

    if let Ok(tracks) = library.all_tracks() {
        library_view.set_tracks(tracks);
    }

    // Double-click on a track → play it
    {
        let player = player.clone();
        let player_bar = player_bar.clone();
        library_view.connect_track_activated(move |track| {
            player_bar.set_track(&track);
            player.send(PlayerCommand::Play(track));
        });
    }

    // Add Folder
    {
        let library = Rc::clone(&library);
        let folder_list = folder_list.clone();
        let window = window.clone();
        add_btn.connect_clicked(move |_| {
            let library = Rc::clone(&library);
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
                if let Err(e) = library.add_folder(&folder) {
                    eprintln!("Failed to add folder: {e}");
                    return;
                }
                refresh_folder_list(&folder_list, &library);
            });
        });
    }

    // Scan
    {
        let library = Rc::clone(&library);
        let scanner = Rc::clone(&scanner);
        let library_view = library_view.clone();
        let status_label = status_label.clone();
        scan_btn.connect_clicked(move |_| {
            status_label.set_text("Scanning…");

            let rx = scanner.scan();

            let library = Rc::clone(&library);
            let library_view = library_view.clone();
            let status_label = status_label.clone();
            // Timeout, not idle: an idle callback returning Continue runs every
            // main-loop iteration and pins a core for the whole scan.
            glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
                Ok(Ok(n)) => {
                    status_label.set_text(&format!("Indexed {n} tracks"));
                    if let Ok(tracks) = library.all_tracks() {
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

    // Poll player state every 250 ms and update the player bar
    {
        let player_bar = player_bar.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            while let Ok(state) = state_rx.try_recv() {
                player_bar.update_state(&state);
            }
            glib::ControlFlow::Continue
        });
    }

    window
}

fn refresh_folder_list(list: &ListBox, library: &Rc<dyn Library>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let configured = library.list_folders().unwrap_or_default();
    for folder in configured {
        list.append(&folder_row(folder, list, library));
    }
}

fn folder_row(folder: LibraryFolder, list: &ListBox, library: &Rc<dyn Library>) -> ListBoxRow {
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

    let library = Rc::clone(library);
    let list = list.clone();
    remove_btn.connect_clicked(move |_| {
        if let Err(e) = library.remove_folder(&folder) {
            eprintln!("Failed to remove folder: {e}");
            return;
        }
        refresh_folder_list(&list, &library);
    });

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
