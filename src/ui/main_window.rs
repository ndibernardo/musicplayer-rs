use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk4::Application;
use gtk4::ApplicationWindow;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::Expander;
use gtk4::FileDialog;
use gtk4::HeaderBar;
use gtk4::Label;
use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::Paned;
use gtk4::ScrolledWindow;
use gtk4::Stack;
use gtk4::ToggleButton;
use gtk4::prelude::*;

use crate::library::db::Db;
use crate::library::db::LibraryFolder;
use crate::library::query::LibraryFilter;
use crate::library::query::album_summaries_for;
use crate::library::query::tracks_for;
use crate::library::scan::spawn_scan;
use crate::player::PlaybackState;
use crate::player::PlayerCommand;
use crate::player::PlayerHandle;
use crate::ui::album_grid::AlbumGrid;
use crate::ui::library_view::LibraryView;
use crate::ui::player_bar::PlayerBar;
use crate::ui::sidebar::Sidebar;
use crate::ui::view_mode::ViewMode;

/// Settings key for the persisted list/grid view choice.
const VIEW_MODE_KEY: &str = "view_mode";

pub fn build(
    app: &Application,
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    state_rx: mpsc::Receiver<PlaybackState>,
) -> ApplicationWindow {
    let header = HeaderBar::new();

    let add_btn = Button::from_icon_name("folder-new-symbolic");
    add_btn.set_tooltip_text(Some("Add music folder"));
    header.pack_start(&add_btn);

    let scan_btn = Button::from_icon_name("view-refresh-symbolic");
    scan_btn.set_tooltip_text(Some("Scan library"));
    header.pack_start(&scan_btn);

    let list_toggle = ToggleButton::new();
    list_toggle.set_icon_name("view-list-symbolic");
    list_toggle.set_tooltip_text(Some("Track list"));
    list_toggle.set_active(true);

    let grid_toggle = ToggleButton::new();
    grid_toggle.set_icon_name("view-grid-symbolic");
    grid_toggle.set_tooltip_text(Some("Album grid"));
    // One linked group: activating one visually releases the other.
    grid_toggle.set_group(Some(&list_toggle));
    header.pack_end(&grid_toggle);
    header.pack_end(&list_toggle);

    let folder_list = ListBox::new();
    folder_list.set_selection_mode(gtk4::SelectionMode::None);

    let status_label = Label::new(Some("Ready"));
    status_label.set_xalign(0.0);
    status_label.set_margin_start(8);
    status_label.set_margin_end(8);
    status_label.set_margin_top(4);
    status_label.set_margin_bottom(4);

    let filter_sidebar = Sidebar::new();

    let folders_scrolled = ScrolledWindow::new();
    folders_scrolled.set_min_content_height(120);
    folders_scrolled.set_child(Some(&folder_list));

    let folders_expander = Expander::new(Some("Watched Folders"));
    folders_expander.set_margin_start(4);
    folders_expander.set_child(Some(&folders_scrolled));

    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.set_width_request(220);
    sidebar.append(&filter_sidebar.widget);
    sidebar.append(&folders_expander);
    sidebar.append(&status_label);

    let library_view = LibraryView::new();
    let album_grid = AlbumGrid::new();

    // The album grid mirrors the active sidebar filter (all albums when unfiltered).
    let current_filter = Rc::new(RefCell::new(LibraryFilter::All));

    let content = Stack::new();
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_named(&library_view.widget, Some(ViewMode::List.child_name()));
    content.add_named(&album_grid.widget, Some(ViewMode::Grid.child_name()));

    // Header toggles flip the visible child and persist the choice.
    {
        let content = content.clone();
        let db = Rc::clone(&db);
        list_toggle.connect_toggled(move |btn| {
            if btn.is_active() {
                content.set_visible_child_name(ViewMode::List.child_name());
                save_view_mode(&db, ViewMode::List);
            }
        });
    }
    {
        let content = content.clone();
        let db = Rc::clone(&db);
        grid_toggle.connect_toggled(move |btn| {
            if btn.is_active() {
                content.set_visible_child_name(ViewMode::Grid.child_name());
                save_view_mode(&db, ViewMode::Grid);
            }
        });
    }

    // Restore the view chosen in the previous session.
    {
        let mode = load_view_mode(&db);
        match mode {
            ViewMode::List => list_toggle.set_active(true),
            ViewMode::Grid => grid_toggle.set_active(true),
        }
        content.set_visible_child_name(mode.child_name());
    }

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

    refresh_folder_list(&folder_list, &db);
    refresh_sidebar(&filter_sidebar, &db);
    refresh_album_grid(&album_grid, &db, &current_filter.borrow());

    if let Ok(tracks) = db.list_tracks() {
        library_view.set_tracks(tracks);
    }

    // Sidebar selection → filter both the track list and the album grid
    {
        let db = Rc::clone(&db);
        let library_view = library_view.clone();
        let album_grid = album_grid.clone();
        let current_filter = Rc::clone(&current_filter);
        filter_sidebar.connect_filter_selected(move |filter| {
            *current_filter.borrow_mut() = filter.clone();
            match tracks_for(&filter, &db) {
                Ok(tracks) => library_view.set_tracks(tracks),
                Err(e) => eprintln!("Filter query failed: {e}"),
            }
            refresh_album_grid(&album_grid, &db, &filter);
        });
    }

    // Album drawer needs that album's tracks when it opens
    {
        let db = Rc::clone(&db);
        album_grid.set_track_provider(move |summary| {
            db.tracks_by_album(&summary.album).unwrap_or_else(|e| {
                eprintln!("Album query failed: {e}");
                Vec::new()
            })
        });
    }

    // Double-click a track inside an album drawer → play it
    {
        let player = player.clone();
        let player_bar = player_bar.clone();
        album_grid.connect_track_activated(move |track| {
            player_bar.set_track(&track);
            player.send(PlayerCommand::Play(track));
        });
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
                if let Err(e) = db.add_folder(&folder) {
                    eprintln!("Failed to add folder: {e}");
                    return;
                }
                refresh_folder_list(&folder_list, &db);
            });
        });
    }

    // Scan
    {
        let db = Rc::clone(&db);
        let library_view = library_view.clone();
        let album_grid = album_grid.clone();
        let status_label = status_label.clone();
        let filter_sidebar = filter_sidebar.clone();
        let current_filter = Rc::clone(&current_filter);
        scan_btn.connect_clicked(move |_| {
            let folders = match db.list_folders() {
                Ok(folders) => folders,
                Err(e) => {
                    status_label.set_text(&format!("Scan error: {e}"));
                    return;
                }
            };

            status_label.set_text("Scanning…");
            let rx = spawn_scan(db_path.clone(), folders);

            let db = Rc::clone(&db);
            let library_view = library_view.clone();
            let album_grid = album_grid.clone();
            let status_label = status_label.clone();
            let filter_sidebar = filter_sidebar.clone();
            let current_filter = Rc::clone(&current_filter);
            // Timeout, not idle: an idle callback returning Continue runs every
            // main-loop iteration and pins a core for the whole scan.
            glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
                Ok(Ok(n)) => {
                    status_label.set_text(&format!("Indexed {n} tracks"));
                    if let Ok(tracks) = db.list_tracks() {
                        library_view.set_tracks(tracks);
                    }
                    refresh_sidebar(&filter_sidebar, &db);
                    refresh_album_grid(&album_grid, &db, &current_filter.borrow());
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

fn refresh_sidebar(sidebar: &Sidebar, db: &Rc<Db>) {
    let genres = db.distinct_genres().unwrap_or_default();
    let artists = db.distinct_artists().unwrap_or_default();
    let albums = db.distinct_albums().unwrap_or_default();
    sidebar.populate(genres, artists, albums);
}

fn refresh_album_grid(grid: &AlbumGrid, db: &Rc<Db>, filter: &LibraryFilter) {
    grid.set_albums(album_summaries_for(filter, db).unwrap_or_default());
}

/// Persists the chosen view; a failed write is non-fatal (logged only).
fn save_view_mode(db: &Rc<Db>, mode: ViewMode) {
    if let Err(e) = db.set_setting(VIEW_MODE_KEY, mode.child_name()) {
        eprintln!("Failed to save view mode: {e}");
    }
}

/// Loads the persisted view, defaulting to the track list when unset or invalid.
fn load_view_mode(db: &Rc<Db>) -> ViewMode {
    db.get_setting(VIEW_MODE_KEY)
        .ok()
        .flatten()
        .and_then(|name| ViewMode::from_name(&name))
        .unwrap_or(ViewMode::List)
}

fn refresh_folder_list(list: &ListBox, db: &Rc<Db>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let configured = db.list_folders().unwrap_or_default();
    for folder in configured {
        list.append(&folder_row(folder, list, db));
    }
}

fn folder_row(folder: LibraryFolder, list: &ListBox, db: &Rc<Db>) -> ListBoxRow {
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
        if let Err(e) = db.remove_folder(&folder) {
            eprintln!("Failed to remove folder: {e}");
            return;
        }
        refresh_folder_list(&list, &db);
    });

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
