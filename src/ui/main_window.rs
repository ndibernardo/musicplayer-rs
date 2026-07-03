use std::cell::Cell;
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
use gtk4::Overlay;
use gtk4::Paned;
use gtk4::Scale;
use gtk4::ScrolledWindow;
use gtk4::Spinner;
use gtk4::Stack;
use gtk4::ToggleButton;
use gtk4::prelude::*;

use crate::library::db::Db;
use crate::library::db::LibraryFolder;
use crate::library::query::LibraryFilter;
use crate::library::query::album_summaries_for;
use crate::library::query::tracks_for;
use crate::library::scan::ScanEvent;
use crate::library::scan::spawn_scan;
use crate::library::watch::FolderWatcher;
use crate::library::watch::watch_folders;
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
/// Settings key for the persisted album-cover size (px).
const COVER_SIZE_KEY: &str = "cover_size";
/// Settings key for the persisted playback volume (0–100).
const VOLUME_KEY: &str = "volume";

/// Album-cover size slider bounds (px).
const COVER_SIZE_MIN: f64 = 200.0;
const COVER_SIZE_MAX: f64 = 500.0;

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

    // Album-art size slider, sitting next to the scan button.
    let size_scale = Scale::with_range(
        Orientation::Horizontal,
        COVER_SIZE_MIN,
        COVER_SIZE_MAX,
        10.0,
    );
    size_scale.set_size_request(120, -1);
    size_scale.set_draw_value(false);
    size_scale.set_tooltip_text(Some("Album art size"));
    size_scale.set_valign(gtk4::Align::Center);
    header.pack_start(&size_scale);

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

    // A scanning spinner + count, centred over the content while a scan runs.
    let scan_spinner = Spinner::new();
    scan_spinner.set_size_request(48, 48);
    let scan_status = Label::new(None);
    scan_status.add_css_class("title-4");
    let scan_indicator = GtkBox::new(Orientation::Vertical, 12);
    scan_indicator.add_css_class("osd");
    scan_indicator.set_halign(gtk4::Align::Center);
    scan_indicator.set_valign(gtk4::Align::Center);
    scan_indicator.set_margin_top(24);
    scan_indicator.set_margin_bottom(24);
    scan_indicator.set_margin_start(24);
    scan_indicator.set_margin_end(24);
    scan_indicator.append(&scan_spinner);
    scan_indicator.append(&scan_status);
    scan_indicator.set_visible(false);

    let content_overlay = Overlay::new();
    content_overlay.set_child(Some(&content));
    content_overlay.add_overlay(&scan_indicator);

    let paned = Paned::new(Orientation::Horizontal);
    paned.set_start_child(Some(&sidebar));
    paned.set_end_child(Some(&content_overlay));
    paned.set_position(220);
    paned.set_vexpand(true);

    let player_bar = PlayerBar::new(player.clone(), load_volume(&db));

    // Persist the volume whenever the user moves the slider.
    {
        let db = Rc::clone(&db);
        player_bar.connect_volume_changed(move |percent| {
            save_setting(&db, VOLUME_KEY, &(percent as i64).to_string());
        });
    }

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

    // Restore the album-cover size, then wire the slider to resize + persist.
    let initial_cover_size = load_cover_size(&db);
    size_scale.set_value(initial_cover_size as f64);
    album_grid.set_cover_size(initial_cover_size);
    {
        let album_grid = album_grid.clone();
        let db = Rc::clone(&db);
        size_scale.connect_value_changed(move |scale| {
            let size = scale.value() as i32;
            album_grid.set_cover_size(size);
            save_setting(&db, COVER_SIZE_KEY, &size.to_string());
        });
    }

    // Reloads the tracks/sidebar/grid after the library changes (e.g. a folder
    // and its tracks were removed), keeping the active filter applied.
    let refresh_views: Rc<dyn Fn()> = {
        let db = Rc::clone(&db);
        let library_view = library_view.clone();
        let album_grid = album_grid.clone();
        let filter_sidebar = filter_sidebar.clone();
        let current_filter = Rc::clone(&current_filter);
        Rc::new(move || {
            let filter = current_filter.borrow();
            match tracks_for(&filter, &db) {
                Ok(tracks) => library_view.set_tracks(tracks),
                Err(e) => eprintln!("Reload after library change failed: {e}"),
            }
            refresh_sidebar(&filter_sidebar, &db);
            refresh_album_grid(&album_grid, &db, &filter);
        })
    };

    // Watches the folders on disk; a change sends `()` on `watch_rx`. `rewatch`
    // re-arms the watcher for the current folder set (kept alive in `folder_watcher`).
    let (watch_tx, watch_rx) = mpsc::channel::<()>();
    let folder_watcher: Rc<RefCell<Option<FolderWatcher>>> = Rc::new(RefCell::new(None));
    let rewatch: Rc<dyn Fn()> = {
        let db = Rc::clone(&db);
        let watch_tx = watch_tx.clone();
        let folder_watcher = Rc::clone(&folder_watcher);
        Rc::new(move || {
            // Drop the previous watcher before starting a fresh one.
            *folder_watcher.borrow_mut() = None;
            let folders = db.list_folders().unwrap_or_default();
            if folders.is_empty() {
                return;
            }
            match watch_folders(&folders, watch_tx.clone()) {
                Ok(watcher) => *folder_watcher.borrow_mut() = Some(watcher),
                Err(e) => eprintln!("Failed to watch folders: {e}"),
            }
        })
    };

    // Adding/removing a watched folder reloads the views and re-arms the watcher.
    let on_folders_changed: Rc<dyn Fn()> = {
        let refresh_views = Rc::clone(&refresh_views);
        let rewatch = Rc::clone(&rewatch);
        Rc::new(move || {
            refresh_views();
            rewatch();
        })
    };

    refresh_folder_list(&folder_list, &db, &on_folders_changed);
    refresh_sidebar(&filter_sidebar, &db);
    refresh_album_grid(&album_grid, &db, &current_filter.borrow());
    rewatch();

    if let Ok(tracks) = db.list_tracks() {
        library_view.set_tracks(tracks);
    }

    // A sidebar selection filters both the track list and the album grid.
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

    // Double-clicking a track inside an album drawer plays it.
    {
        let player = player.clone();
        let player_bar = player_bar.clone();
        album_grid.connect_track_activated(move |track| {
            player_bar.set_track(&track);
            player.send(PlayerCommand::Play(track));
        });
    }

    // Double-clicking a track plays it.
    {
        let player = player.clone();
        let player_bar = player_bar.clone();
        library_view.connect_track_activated(move |track| {
            player_bar.set_track(&track);
            player.send(PlayerCommand::Play(track));
        });
    }

    // Starts a background scan of every watched folder, with live progress and
    // the spinner overlay. Shared by the Scan button and the Add-Folder flow.
    let start_scan: Rc<dyn Fn()> = {
        let db = Rc::clone(&db);
        let db_path = db_path.clone();
        let library_view = library_view.clone();
        let album_grid = album_grid.clone();
        let status_label = status_label.clone();
        let filter_sidebar = filter_sidebar.clone();
        let current_filter = Rc::clone(&current_filter);
        let scan_spinner = scan_spinner.clone();
        let scan_status = scan_status.clone();
        let scan_indicator = scan_indicator.clone();
        Rc::new(move || {
            let folders = match db.list_folders() {
                Ok(folders) => folders,
                Err(e) => {
                    status_label.set_text(&format!("Scan error: {e}"));
                    return;
                }
            };
            if folders.is_empty() {
                return;
            }

            status_label.set_text("Scanning…");
            scan_status.set_text("Scanning…");
            scan_indicator.set_visible(true);
            scan_spinner.start();
            let rx = spawn_scan(db_path.clone(), folders);

            let db = Rc::clone(&db);
            let library_view = library_view.clone();
            let album_grid = album_grid.clone();
            let status_label = status_label.clone();
            let filter_sidebar = filter_sidebar.clone();
            let current_filter = Rc::clone(&current_filter);
            let scan_spinner = scan_spinner.clone();
            let scan_status = scan_status.clone();
            let scan_indicator = scan_indicator.clone();
            let stop_indicator = move |spinner: &Spinner, indicator: &GtkBox| {
                spinner.stop();
                indicator.set_visible(false);
            };
            // Timeout, not idle: an idle callback returning Continue runs every
            // main-loop iteration and pins a core for the whole scan. Drain all
            // pending events each tick so the count keeps up with the scan.
            glib::timeout_add_local(Duration::from_millis(100), move || {
                loop {
                    match rx.try_recv() {
                        Ok(ScanEvent::Progress(n)) => {
                            let msg = format!("Scanning… {n} files");
                            status_label.set_text(&msg);
                            scan_status.set_text(&msg);
                        }
                        Ok(ScanEvent::Finished(Ok(n))) => {
                            status_label.set_text(&format!("Indexed {n} tracks"));
                            stop_indicator(&scan_spinner, &scan_indicator);
                            if let Ok(tracks) = db.list_tracks() {
                                library_view.set_tracks(tracks);
                            }
                            refresh_sidebar(&filter_sidebar, &db);
                            refresh_album_grid(&album_grid, &db, &current_filter.borrow());
                            return glib::ControlFlow::Break;
                        }
                        Ok(ScanEvent::Finished(Err(e))) => {
                            status_label.set_text(&format!("Scan error: {e}"));
                            stop_indicator(&scan_spinner, &scan_indicator);
                            return glib::ControlFlow::Break;
                        }
                        Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            stop_indicator(&scan_spinner, &scan_indicator);
                            return glib::ControlFlow::Break;
                        }
                    }
                }
            });
        })
    };

    // The scan button scans every watched folder.
    {
        let start_scan = Rc::clone(&start_scan);
        scan_btn.connect_clicked(move |_| start_scan());
    }

    // Adding a folder persists it, refreshes the views, starts watching it, and then scans automatically.
    {
        let db = Rc::clone(&db);
        let folder_list = folder_list.clone();
        let window = window.clone();
        let on_folders_changed = Rc::clone(&on_folders_changed);
        let rewatch = Rc::clone(&rewatch);
        let start_scan = Rc::clone(&start_scan);
        add_btn.connect_clicked(move |_| {
            let db = Rc::clone(&db);
            let folder_list = folder_list.clone();
            let window = window.clone();
            let on_folders_changed = Rc::clone(&on_folders_changed);
            let rewatch = Rc::clone(&rewatch);
            let start_scan = Rc::clone(&start_scan);
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
                refresh_folder_list(&folder_list, &db, &on_folders_changed);
                rewatch();
                start_scan();
            });
        });
    }

    // Debounced auto-rescan: coalesce filesystem events and rescan once the
    // changes settle, so a bulk copy triggers a single scan, not one per file.
    {
        let start_scan = Rc::clone(&start_scan);
        let dirty = Rc::new(Cell::new(false));
        glib::timeout_add_local(Duration::from_millis(700), move || {
            let mut changed = false;
            while watch_rx.try_recv().is_ok() {
                changed = true;
            }
            if changed {
                dirty.set(true); // still changing — wait for a quiet tick
            } else if dirty.replace(false) {
                start_scan();
            }
            glib::ControlFlow::Continue
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
    sidebar.populate(genres, artists);
}

fn refresh_album_grid(grid: &AlbumGrid, db: &Rc<Db>, filter: &LibraryFilter) {
    grid.set_albums(album_summaries_for(filter, db).unwrap_or_default());
}

/// Persists a setting; a failed write is non-fatal (logged only).
fn save_setting(db: &Rc<Db>, key: &str, value: &str) {
    if let Err(e) = db.set_setting(key, value) {
        eprintln!("Failed to save setting {key}: {e}");
    }
}

/// Persists the chosen view.
fn save_view_mode(db: &Rc<Db>, mode: ViewMode) {
    save_setting(db, VIEW_MODE_KEY, mode.child_name());
}

/// Loads the persisted view, defaulting to the track list when unset or invalid.
fn load_view_mode(db: &Rc<Db>) -> ViewMode {
    db.get_setting(VIEW_MODE_KEY)
        .ok()
        .flatten()
        .and_then(|name| ViewMode::from_name(&name))
        .unwrap_or(ViewMode::List)
}

/// Loads the persisted cover size, clamped to the slider bounds; defaults to the
/// minimum when unset or invalid.
fn load_cover_size(db: &Rc<Db>) -> i32 {
    db.get_setting(COVER_SIZE_KEY)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i32>().ok())
        .map(|n| n.clamp(COVER_SIZE_MIN as i32, COVER_SIZE_MAX as i32))
        .unwrap_or(COVER_SIZE_MIN as i32)
}

/// Loads the persisted volume (0–100), defaulting to 70 when unset or invalid.
fn load_volume(db: &Rc<Db>) -> f64 {
    db.get_setting(VOLUME_KEY)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|v| v.clamp(0.0, 100.0))
        .unwrap_or(70.0)
}

fn refresh_folder_list(list: &ListBox, db: &Rc<Db>, on_change: &Rc<dyn Fn()>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let configured = db.list_folders().unwrap_or_default();
    for folder in configured {
        list.append(&folder_row(folder, list, db, on_change));
    }
}

fn folder_row(
    folder: LibraryFolder,
    list: &ListBox,
    db: &Rc<Db>,
    on_change: &Rc<dyn Fn()>,
) -> ListBoxRow {
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
    let on_change = Rc::clone(on_change);
    remove_btn.connect_clicked(move |_| {
        if let Err(e) = db.remove_folder(&folder) {
            eprintln!("Failed to remove folder: {e}");
            return;
        }
        refresh_folder_list(&list, &db, &on_change);
        on_change();
    });

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
