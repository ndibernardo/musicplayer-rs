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
use gtk4::DropDown;
use gtk4::Expander;
use gtk4::FileDialog;
use gtk4::HeaderBar;
use gtk4::Image;
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

use crate::library::album::AlbumSort;
use crate::library::album::AlbumSortField;
use crate::library::album::SortDirection;
use crate::library::db::Db;
use crate::library::db::LibraryFolder;
use crate::library::query::LibraryFilter;
use crate::library::query::album_summaries_for;
use crate::library::query::tracks_for;
use crate::library::scan::ScanEvent;
use crate::library::scan::spawn_scan;
use crate::library::track::Track;
use crate::library::track::TrackId;
use crate::library::watch::FolderWatcher;
use crate::library::watch::watch_folders;
use crate::player::PlaybackState;
use crate::player::PlayerCommand;
use crate::player::PlayerHandle;
use crate::player::SeekPosition;
use crate::ui::album_grid::AlbumGrid;
use crate::ui::library_view::LibraryView;
use crate::ui::player_bar::PlayerBar;
use crate::ui::queue_view::QueueView;
use crate::ui::sidebar::Sidebar;
use crate::ui::view_mode::ViewMode;

/// Settings key for the persisted list/grid view choice.
const VIEW_MODE_KEY: &str = "view_mode";
/// Settings key for the persisted album-cover size (px).
const COVER_SIZE_KEY: &str = "cover_size";
/// Settings key for the persisted playback volume (0–100).
const VOLUME_KEY: &str = "volume";
/// Settings key for the persisted queue, a comma-separated list of track ids.
const QUEUE_KEY: &str = "queue";
/// Settings key for the persisted current track id within the queue.
const QUEUE_CURRENT_KEY: &str = "queue_current";
/// Settings key for the persisted playback position (milliseconds).
const QUEUE_POSITION_KEY: &str = "queue_position";
/// Settings keys for the persisted album-grid sort field and direction.
const ALBUM_SORT_FIELD_KEY: &str = "album_sort_field";
const ALBUM_SORT_DIR_KEY: &str = "album_sort_dir";

/// The album-grid sort fields in dropdown order.
const SORT_FIELDS: [AlbumSortField; 4] = [
    AlbumSortField::AlbumArtist,
    AlbumSortField::Year,
    AlbumSortField::Genre,
    AlbumSortField::Album,
];
/// Human labels for `SORT_FIELDS`, in the same order.
const SORT_FIELD_LABELS: [&str; 4] = ["Album Artist", "Year", "Genre", "Album"];

/// Album-cover size slider bounds (px).
const COVER_SIZE_MIN: f64 = 200.0;
const COVER_SIZE_MAX: f64 = 500.0;

/// Application-wide styling: a darker player bar than the surrounding window,
/// with visible slider and progress troughs against that darker background.
const APP_CSS: &str = "\
.player-bar { background-color: rgba(0, 0, 0, 0.25); }
.player-bar scale trough {
    background-color: alpha(currentColor, 0.22);
    min-height: 6px;
}
.player-bar scale.seek slider {
    min-width: 0;
    min-height: 0;
    margin: 0;
    background: none;
    border: none;
    box-shadow: none;
}
.album-selected {
    background-color: rgba(0, 0, 0, 0.24);
    border-radius: 10px;
    padding: 10px;
}
.album-drawer {
    background-color: rgba(0, 0, 0, 0.24);
    border-radius: 10px;
}
";

pub fn build(
    app: &Application,
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    state_rx: mpsc::Receiver<PlaybackState>,
) -> ApplicationWindow {
    install_styles();

    let header = HeaderBar::new();
    // Show only the close button. Under a tiling WM (e.g. sway) the minimize and
    // maximize buttons are unusable and render with a mismatched background on
    // focus changes; dropping them keeps the titlebar uniform.
    header.set_decoration_layout(Some(":close"));

    let add_btn = Button::from_icon_name("folder-new-symbolic");
    add_btn.set_tooltip_text(Some("Add music folder"));
    header.pack_start(&add_btn);

    let scan_btn = Button::from_icon_name("view-refresh-symbolic");
    scan_btn.set_tooltip_text(Some("Scan library"));
    header.pack_start(&scan_btn);

    // Sort controls (a sort icon, the field, and the direction), shown to the
    // left of the view toggles and only in grid view.
    let sort_icon = Image::from_icon_name("view-sort-descending-symbolic");

    let sort_field = DropDown::from_strings(&SORT_FIELD_LABELS);
    sort_field.set_tooltip_text(Some("Sort albums by"));
    sort_field.set_valign(gtk4::Align::Center);

    let sort_dir = ToggleButton::new();
    sort_dir.set_tooltip_text(Some("Sort direction"));
    sort_dir.set_valign(gtk4::Align::Center);

    let sort_controls = GtkBox::new(Orientation::Horizontal, 6);
    sort_controls.set_valign(gtk4::Align::Center);
    // Gap that separates the sort section from the folder buttons beside it.
    sort_controls.set_margin_start(18);
    sort_controls.append(&sort_icon);
    sort_controls.append(&sort_field);
    sort_controls.append(&sort_dir);

    // Album-art size slider, shown to the right of the toggles, grid view only.
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
    // Gap that separates the size slider from the view toggles beside it.
    size_scale.set_margin_start(18);

    let list_toggle = ToggleButton::new();
    list_toggle.set_icon_name("view-list-symbolic");
    list_toggle.set_tooltip_text(Some("Track list"));
    list_toggle.set_active(true);

    let grid_toggle = ToggleButton::new();
    grid_toggle.set_icon_name("view-grid-symbolic");
    grid_toggle.set_tooltip_text(Some("Album grid"));
    // One linked group: activating one visually releases the other.
    grid_toggle.set_group(Some(&list_toggle));
    // Sort controls go at the left, next to the folder buttons; the view toggles
    // and size slider stay at the right (packed end-first: list, grid, slider).
    header.pack_start(&sort_controls);
    header.pack_end(&size_scale);
    header.pack_end(&grid_toggle);
    header.pack_end(&list_toggle);

    // Shows or hides both grid-only control groups together.
    let toggle_grid_controls: Rc<dyn Fn(bool)> = {
        let sort_controls = sort_controls.clone();
        let size_scale = size_scale.clone();
        Rc::new(move |visible| {
            sort_controls.set_visible(visible);
            size_scale.set_visible(visible);
        })
    };

    let folder_list = ListBox::new();
    folder_list.set_selection_mode(gtk4::SelectionMode::None);

    let status_label = Label::new(Some("Ready"));
    status_label.set_xalign(0.0);
    status_label.set_margin_start(8);
    status_label.set_margin_end(8);
    status_label.set_margin_top(4);
    status_label.set_margin_bottom(4);

    let filter_sidebar = Sidebar::new();

    let queue_view = QueueView::new();
    let queue_expander = Expander::new(Some("Queue"));
    queue_expander.set_expanded(true);
    queue_expander.set_margin_start(4);
    queue_expander.set_child(Some(&queue_view.widget));

    let folders_scrolled = ScrolledWindow::new();
    folders_scrolled.set_min_content_height(120);
    folders_scrolled.set_child(Some(&folder_list));

    let folders_expander = Expander::new(Some("Watched Folders"));
    folders_expander.set_margin_start(4);
    folders_expander.set_child(Some(&folders_scrolled));

    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.set_width_request(220);
    sidebar.append(&filter_sidebar.widget);
    sidebar.append(&queue_expander);
    sidebar.append(&folders_expander);
    sidebar.append(&status_label);

    let library_view = LibraryView::new();
    let album_grid = AlbumGrid::new();

    // The album grid mirrors the active sidebar filter (all albums when unfiltered).
    let current_filter = Rc::new(RefCell::new(LibraryFilter::All));

    // The album grid's current sort, restored from the previous session.
    let current_sort = Rc::new(RefCell::new(load_album_sort(&db)));

    // The tracks currently in the play queue, mirrored to the sidebar queue view.
    let current_queue: Rc<RefCell<Vec<Track>>> = Rc::new(RefCell::new(Vec::new()));

    let content = Stack::new();
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_named(&library_view.widget, Some(ViewMode::List.child_name()));
    content.add_named(&album_grid.widget, Some(ViewMode::Grid.child_name()));

    // Header toggles flip the visible child and persist the choice.
    {
        let content = content.clone();
        let db = Rc::clone(&db);
        let toggle_grid_controls = Rc::clone(&toggle_grid_controls);
        list_toggle.connect_toggled(move |btn| {
            if btn.is_active() {
                content.set_visible_child_name(ViewMode::List.child_name());
                save_view_mode(&db, ViewMode::List);
                // The sort and cover-size controls are only meaningful in grid view.
                toggle_grid_controls(false);
            }
        });
    }
    {
        let content = content.clone();
        let db = Rc::clone(&db);
        let toggle_grid_controls = Rc::clone(&toggle_grid_controls);
        grid_toggle.connect_toggled(move |btn| {
            if btn.is_active() {
                content.set_visible_child_name(ViewMode::Grid.child_name());
                save_view_mode(&db, ViewMode::Grid);
                toggle_grid_controls(true);
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
        toggle_grid_controls(matches!(mode, ViewMode::Grid));
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

    // Replaces the queue with `tracks` at `start`: mirrors it to the sidebar,
    // shows the starting track, persists it, and plays it.
    let enqueue: Rc<dyn Fn(Vec<Track>, usize)> = {
        let player = player.clone();
        let player_bar = player_bar.clone();
        let queue_view = queue_view.clone();
        let current_queue = Rc::clone(&current_queue);
        let db = Rc::clone(&db);
        Rc::new(move |tracks: Vec<Track>, start: usize| {
            if tracks.is_empty() {
                return;
            }
            let start = start.min(tracks.len() - 1);
            let current_id = tracks[start].id;
            *current_queue.borrow_mut() = tracks.clone();
            queue_view.set_tracks(tracks.clone());
            queue_view.set_current(Some(current_id));
            player_bar.set_track(&tracks[start]);
            save_queue(&db, &tracks);
            save_current(&db, current_id);
            player.send(PlayerCommand::PlayQueue { tracks, start });
        })
    };

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

    // Show the restored sort in the header controls, then wire changes to re-sort
    // the grid and persist. State is set before connecting so it doesn't re-fire.
    {
        let sort = *current_sort.borrow();
        sort_field.set_selected(sort_field_index(sort.field));
        sort_dir.set_active(matches!(sort.direction, SortDirection::Descending));
        sort_dir.set_icon_name(direction_icon(sort.direction));
    }
    {
        let album_grid = album_grid.clone();
        let db = Rc::clone(&db);
        let current_filter = Rc::clone(&current_filter);
        let current_sort = Rc::clone(&current_sort);
        sort_field.connect_selected_notify(move |dropdown| {
            current_sort.borrow_mut().field = sort_field_at(dropdown.selected());
            let sort = *current_sort.borrow();
            save_album_sort(&db, sort);
            refresh_album_grid(&album_grid, &db, &current_filter.borrow(), sort);
        });
    }
    {
        let album_grid = album_grid.clone();
        let db = Rc::clone(&db);
        let current_filter = Rc::clone(&current_filter);
        let current_sort = Rc::clone(&current_sort);
        sort_dir.connect_toggled(move |btn| {
            let direction = if btn.is_active() {
                SortDirection::Descending
            } else {
                SortDirection::Ascending
            };
            current_sort.borrow_mut().direction = direction;
            btn.set_icon_name(direction_icon(direction));
            let sort = *current_sort.borrow();
            save_album_sort(&db, sort);
            refresh_album_grid(&album_grid, &db, &current_filter.borrow(), sort);
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
        let current_sort = Rc::clone(&current_sort);
        Rc::new(move || {
            let filter = current_filter.borrow();
            match tracks_for(&filter, &db) {
                Ok(tracks) => library_view.set_tracks(tracks),
                Err(e) => tracing::error!("Reload after library change failed: {e}"),
            }
            refresh_sidebar(&filter_sidebar, &db);
            refresh_album_grid(&album_grid, &db, &filter, *current_sort.borrow());
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
                Err(e) => tracing::error!("Failed to watch folders: {e}"),
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
    refresh_album_grid(
        &album_grid,
        &db,
        &current_filter.borrow(),
        *current_sort.borrow(),
    );
    rewatch();

    if let Ok(tracks) = db.list_tracks() {
        library_view.set_tracks(tracks);
    }

    // Restore the queue from the previous session and reopen the last track
    // paused at exactly where it was closed (RestorePaused does not resume).
    {
        let restored = load_queue(&db);
        if !restored.is_empty() {
            let current_id = load_current(&db);
            let start = current_id
                .and_then(|id| restored.iter().position(|t| t.id.value() == id))
                .unwrap_or(0);
            let position = load_position(&db);
            queue_view.set_tracks(restored.clone());
            queue_view.set_current(restored.get(start).map(|t| t.id));
            if let Some(track) = restored.get(start) {
                player_bar.set_track(track);
            }
            player.send(PlayerCommand::RestorePaused {
                tracks: restored.clone(),
                start,
                position,
            });
            *current_queue.borrow_mut() = restored;
        }
    }

    // A sidebar selection filters both the track list and the album grid.
    {
        let db = Rc::clone(&db);
        let library_view = library_view.clone();
        let album_grid = album_grid.clone();
        let current_filter = Rc::clone(&current_filter);
        let current_sort = Rc::clone(&current_sort);
        filter_sidebar.connect_filter_selected(move |filter| {
            *current_filter.borrow_mut() = filter.clone();
            match tracks_for(&filter, &db) {
                Ok(tracks) => library_view.set_tracks(tracks),
                Err(e) => tracing::error!("Filter query failed: {e}"),
            }
            refresh_album_grid(&album_grid, &db, &filter, *current_sort.borrow());
        });
    }

    // Album drawer needs that album's tracks when it opens
    {
        let db = Rc::clone(&db);
        album_grid.set_track_provider(move |summary| {
            db.tracks_by_album(&summary.album).unwrap_or_else(|e| {
                tracing::error!("Album query failed: {e}");
                Vec::new()
            })
        });
    }

    // Opening an album cover enqueues the whole album (clearing the queue).
    {
        let enqueue = Rc::clone(&enqueue);
        album_grid.connect_album_activated(move |tracks| enqueue(tracks, 0));
    }

    // Activating a single track in an album drawer replaces the queue with it.
    {
        let enqueue = Rc::clone(&enqueue);
        album_grid.connect_track_activated(move |tracks, index| {
            if let Some(track) = tracks.get(index) {
                enqueue(vec![track.clone()], 0);
            }
        });
    }

    // Activating a track in the list replaces the queue with just that track.
    {
        let enqueue = Rc::clone(&enqueue);
        library_view.connect_track_activated(move |tracks, index| {
            if let Some(track) = tracks.get(index) {
                enqueue(vec![track.clone()], 0);
            }
        });
    }

    // Clicking a queue entry jumps playback to it within the current queue.
    {
        let enqueue = Rc::clone(&enqueue);
        let current_queue = Rc::clone(&current_queue);
        queue_view.connect_track_selected(move |index| {
            let tracks = current_queue.borrow().clone();
            enqueue(tracks, index);
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
        let current_sort = Rc::clone(&current_sort);
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
            let current_sort = Rc::clone(&current_sort);
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
                            refresh_album_grid(
                                &album_grid,
                                &db,
                                &current_filter.borrow(),
                                *current_sort.borrow(),
                            );
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
                    tracing::error!("Failed to add folder: {e}");
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

    // Poll player state every 250 ms and update the player bar. When the playing
    // track changes (Next/Previous/auto-advance), resolve it against the queue to
    // refresh the title and the queue highlight.
    {
        let player_bar = player_bar.clone();
        let queue_view = queue_view.clone();
        let current_queue = Rc::clone(&current_queue);
        let db = Rc::clone(&db);
        let mut last_shown: Option<TrackId> = None;
        let mut last_saved_secs: Option<u64> = None;
        glib::timeout_add_local(Duration::from_millis(250), move || {
            while let Ok(state) = state_rx.try_recv() {
                // Apply a track change before update_state: set_track resets the
                // progress bar, so it must run first or it would wipe the position
                // that update_state is about to show (notably on a restored track).
                let track_id = state.current_track();
                if track_id != last_shown {
                    last_shown = track_id;
                    match track_id {
                        Some(id) => {
                            if let Some(track) =
                                current_queue.borrow().iter().find(|t| t.id == id).cloned()
                            {
                                player_bar.set_track(&track);
                            }
                            queue_view.set_current(Some(id));
                            save_current(&db, id);
                        }
                        None => queue_view.set_current(None),
                    }
                }
                player_bar.update_state(&state);
                // Persist the position at most once per whole second, so reopening
                // resumes near where the session was closed.
                if let Some(position) = state.position()
                    && last_saved_secs != Some(position.as_secs())
                {
                    last_saved_secs = Some(position.as_secs());
                    save_position(&db, position);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    window
}

/// Registers the application CSS on the default display, once at window build.
fn install_styles() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(APP_CSS);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn refresh_sidebar(sidebar: &Sidebar, db: &Rc<Db>) {
    let genres = db.distinct_genres().unwrap_or_default();
    let artists = db.distinct_artists().unwrap_or_default();
    sidebar.populate(genres, artists);
}

fn refresh_album_grid(grid: &AlbumGrid, db: &Rc<Db>, filter: &LibraryFilter, sort: AlbumSort) {
    grid.set_albums(album_summaries_for(filter, &sort, db).unwrap_or_default());
}

/// Persists a setting; a failed write is non-fatal (logged only).
fn save_setting(db: &Rc<Db>, key: &str, value: &str) {
    if let Err(e) = db.set_setting(key, value) {
        tracing::error!("Failed to save setting {key}: {e}");
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

/// Persists the queue as a comma-separated list of track ids.
fn save_queue(db: &Rc<Db>, tracks: &[Track]) {
    let ids = tracks
        .iter()
        .map(|t| t.id.value().to_string())
        .collect::<Vec<_>>()
        .join(",");
    save_setting(db, QUEUE_KEY, &ids);
}

/// Persists the current track id within the queue.
fn save_current(db: &Rc<Db>, id: TrackId) {
    save_setting(db, QUEUE_CURRENT_KEY, &id.value().to_string());
}

/// Persists the playback position in milliseconds.
fn save_position(db: &Rc<Db>, position: SeekPosition) {
    let millis = position.as_duration().as_millis() as u64;
    save_setting(db, QUEUE_POSITION_KEY, &millis.to_string());
}

/// Loads the persisted playback position, defaulting to the start when unset.
fn load_position(db: &Rc<Db>) -> SeekPosition {
    db.get_setting(QUEUE_POSITION_KEY)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .map(SeekPosition::from_millis)
        .unwrap_or_else(SeekPosition::zero)
}

/// Rebuilds the persisted queue from its stored track ids in a single query,
/// skipping any ids no longer present in the library.
fn load_queue(db: &Rc<Db>) -> Vec<Track> {
    let Some(raw) = db.get_setting(QUEUE_KEY).ok().flatten() else {
        return Vec::new();
    };
    let ids: Vec<TrackId> = raw
        .split(',')
        .filter_map(|s| s.parse::<i64>().ok())
        .map(TrackId::new)
        .collect();
    if ids.is_empty() {
        return Vec::new();
    }
    db.tracks_by_ids(&ids).unwrap_or_default()
}

/// Loads the persisted current track id, or `None` when unset or invalid.
fn load_current(db: &Rc<Db>) -> Option<i64> {
    db.get_setting(QUEUE_CURRENT_KEY)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok())
}

/// Persists the album-grid sort field and direction.
fn save_album_sort(db: &Rc<Db>, sort: AlbumSort) {
    save_setting(db, ALBUM_SORT_FIELD_KEY, sort.field.as_key());
    save_setting(db, ALBUM_SORT_DIR_KEY, sort.direction.as_key());
}

/// Loads the persisted album-grid sort, defaulting to album artist ascending.
fn load_album_sort(db: &Rc<Db>) -> AlbumSort {
    let field = db
        .get_setting(ALBUM_SORT_FIELD_KEY)
        .ok()
        .flatten()
        .and_then(|k| AlbumSortField::from_key(&k))
        .unwrap_or(AlbumSortField::AlbumArtist);
    let direction = db
        .get_setting(ALBUM_SORT_DIR_KEY)
        .ok()
        .flatten()
        .and_then(|k| SortDirection::from_key(&k))
        .unwrap_or(SortDirection::Ascending);
    AlbumSort::new(field, direction)
}

/// The dropdown position of `field`.
fn sort_field_index(field: AlbumSortField) -> u32 {
    SORT_FIELDS.iter().position(|f| *f == field).unwrap_or(0) as u32
}

/// The sort field at dropdown position `index`.
fn sort_field_at(index: u32) -> AlbumSortField {
    SORT_FIELDS
        .get(index as usize)
        .copied()
        .unwrap_or(AlbumSortField::AlbumArtist)
}

/// The icon name for a sort direction.
fn direction_icon(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "view-sort-ascending-symbolic",
        SortDirection::Descending => "view-sort-descending-symbolic",
    }
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
            tracing::error!("Failed to remove folder: {e}");
            return;
        }
        refresh_folder_list(&list, &db, &on_change);
        on_change();
    });

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
