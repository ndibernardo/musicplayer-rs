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
use crate::library::filter::LibraryFilter;
use crate::library::scan::ScanEvent;
use crate::library::scan::spawn_scan;
use crate::library::settings::COVER_SIZE_MAX;
use crate::library::settings::COVER_SIZE_MIN;
use crate::library::settings::Settings;
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

/// The album-grid sort fields in dropdown order.
const SORT_FIELDS: [AlbumSortField; 4] = [
    AlbumSortField::AlbumArtist,
    AlbumSortField::Year,
    AlbumSortField::Genre,
    AlbumSortField::Album,
];
/// Human labels for `SORT_FIELDS`, in the same order.
const SORT_FIELD_LABELS: [&str; 4] = ["Album Artist", "Year", "Genre", "Album"];

/// Application-wide styling.
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

/// Shared application controller. All fields are ref-counted so the struct is
/// cheap to clone; signal handlers capture `this = self.clone()`.
#[derive(Clone)]
struct MainWindow {
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    // Views
    library_view: LibraryView,
    album_grid: AlbumGrid,
    filter_sidebar: Sidebar,
    queue_view: QueueView,
    player_bar: PlayerBar,
    // Widgets owned by the controller
    window: ApplicationWindow,
    folder_list: ListBox,
    status_label: Label,
    scan_spinner: Spinner,
    scan_status: Label,
    scan_indicator: GtkBox,
    // Shared state
    current_filter: Rc<RefCell<LibraryFilter>>,
    current_sort: Rc<RefCell<AlbumSort>>,
    current_queue: Rc<RefCell<Vec<Track>>>,
    folder_watcher: Rc<RefCell<Option<FolderWatcher>>>,
    watch_tx: mpsc::Sender<()>,
}

impl MainWindow {
    fn settings(&self) -> Settings<'_> {
        Settings::new(&self.db)
    }

    /// Replaces the queue with `tracks` at `start`: mirrors it to the sidebar,
    /// shows the starting track, persists it, and plays it.
    fn enqueue(&self, tracks: Vec<Track>, start: usize) {
        if tracks.is_empty() {
            return;
        }
        let start = start.min(tracks.len() - 1);
        let current_id = tracks[start].id;
        *self.current_queue.borrow_mut() = tracks.clone();
        self.queue_view.set_tracks(tracks.clone());
        self.queue_view.set_current(Some(current_id));
        self.player_bar.set_track(&tracks[start]);
        let s = self.settings();
        s.set_queue(&tracks.iter().map(|t| t.id).collect::<Vec<_>>());
        s.set_queue_current(current_id);
        self.player.send(PlayerCommand::PlayQueue { tracks, start });
    }

    /// Reloads the tracks/sidebar/grid after the library changes, keeping the
    /// active filter applied.
    fn refresh_views(&self) {
        let filter = self.current_filter.borrow();
        match self.db.tracks_for(&filter) {
            Ok(tracks) => self.library_view.set_tracks(tracks),
            Err(e) => tracing::error!("Reload after library change failed: {e}"),
        }
        self.refresh_sidebar();
        self.refresh_album_grid_with(&filter, *self.current_sort.borrow());
    }

    fn refresh_sidebar(&self) {
        let genres = self.db.distinct_genres().unwrap_or_default();
        let artists = self.db.distinct_artists().unwrap_or_default();
        self.filter_sidebar.populate(genres, artists);
    }

    fn refresh_album_grid_with(&self, filter: &LibraryFilter, sort: AlbumSort) {
        self.album_grid.set_albums(
            self.db
                .album_summaries_for(filter, &sort)
                .unwrap_or_default(),
        );
    }

    /// Re-arms the folder watcher for the current folder set.
    fn rewatch(&self) {
        *self.folder_watcher.borrow_mut() = None;
        let folders = self.db.list_folders().unwrap_or_default();
        if folders.is_empty() {
            return;
        }
        match watch_folders(&folders, self.watch_tx.clone()) {
            Ok(watcher) => *self.folder_watcher.borrow_mut() = Some(watcher),
            Err(e) => tracing::error!("Failed to watch folders: {e}"),
        }
    }

    fn on_folders_changed(&self) {
        self.refresh_views();
        self.rewatch();
    }

    /// Starts a background scan of every watched folder, showing a spinner
    /// overlay and live progress, then refreshes the library on completion.
    fn start_scan(&self) {
        let folders = match self.db.list_folders() {
            Ok(f) => f,
            Err(e) => {
                self.status_label.set_text(&format!("Scan error: {e}"));
                return;
            }
        };
        if folders.is_empty() {
            return;
        }
        self.status_label.set_text("Scanning…");
        self.scan_status.set_text("Scanning…");
        self.scan_indicator.set_visible(true);
        self.scan_spinner.start();
        self.install_scan_poll(spawn_scan(self.db_path.clone(), folders));
    }

    /// Installs a 100 ms GLib timeout that drains scan events from `rx` until
    /// the scan finishes or the channel closes, then refreshes the library views.
    ///
    /// Timeout (not idle) so progress counts stay current without pinning a core.
    fn install_scan_poll(&self, rx: mpsc::Receiver<ScanEvent>) {
        let this = self.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            loop {
                match rx.try_recv() {
                    Ok(ScanEvent::Progress(n)) => {
                        let msg = format!("Scanning… {n} files");
                        this.status_label.set_text(&msg);
                        this.scan_status.set_text(&msg);
                    }
                    Ok(ScanEvent::Finished(Ok(n))) => {
                        this.status_label.set_text(&format!("Indexed {n} tracks"));
                        this.scan_spinner.stop();
                        this.scan_indicator.set_visible(false);
                        if let Ok(tracks) = this.db.tracks_for(&LibraryFilter::All) {
                            this.library_view.set_tracks(tracks);
                        }
                        this.refresh_sidebar();
                        this.refresh_album_grid_with(
                            &this.current_filter.borrow(),
                            *this.current_sort.borrow(),
                        );
                        return glib::ControlFlow::Break;
                    }
                    Ok(ScanEvent::Finished(Err(e))) => {
                        this.status_label.set_text(&format!("Scan error: {e}"));
                        this.scan_spinner.stop();
                        this.scan_indicator.set_visible(false);
                        return glib::ControlFlow::Break;
                    }
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        this.scan_spinner.stop();
                        this.scan_indicator.set_visible(false);
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
    }

    fn refresh_folder_list(&self) {
        let list = &self.folder_list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        for folder in self.db.list_folders().unwrap_or_default() {
            list.append(&self.folder_row(folder));
        }
    }

    fn folder_row(&self, folder: LibraryFolder) -> ListBoxRow {
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

        let this = self.clone();
        remove_btn.connect_clicked(move |_| {
            if let Err(e) = this.db.remove_folder(&folder) {
                tracing::error!("Failed to remove folder: {e}");
                return;
            }
            this.refresh_folder_list();
            this.on_folders_changed();
        });

        let row = ListBoxRow::new();
        row.set_child(Some(&row_box));
        row
    }
}

pub fn build(
    app: &Application,
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    state_rx: mpsc::Receiver<PlaybackState>,
) -> ApplicationWindow {
    install_styles();

    // Read all persisted values before `db` is consumed by the MainWindow struct.
    let (
        initial_cover_size,
        initial_sort,
        initial_view_mode,
        initial_volume,
        initial_queue_ids,
        initial_queue_current,
        initial_queue_position_ms,
    ) = {
        let s = Settings::new(&db);
        (
            s.cover_size(),
            s.album_sort(),
            s.view_mode_name()
                .as_deref()
                .and_then(ViewMode::from_name)
                .unwrap_or(ViewMode::List),
            s.volume(),
            s.queue_track_ids(),
            s.queue_current_id(),
            s.queue_position_millis(),
        )
    };

    let header = HeaderBar::new();
    // Under a tiling WM the minimize/maximize buttons are unusable and render
    // with a mismatched background on focus changes; show only the close button.
    header.set_decoration_layout(Some(":close"));

    let add_btn = Button::from_icon_name("folder-new-symbolic");
    add_btn.set_tooltip_text(Some("Add music folder"));
    header.pack_start(&add_btn);

    let scan_btn = Button::from_icon_name("view-refresh-symbolic");
    scan_btn.set_tooltip_text(Some("Scan library"));
    header.pack_start(&scan_btn);

    let sort_icon = Image::from_icon_name("view-sort-descending-symbolic");
    let sort_field = DropDown::from_strings(&SORT_FIELD_LABELS);
    sort_field.set_tooltip_text(Some("Sort albums by"));
    sort_field.set_valign(gtk4::Align::Center);
    let sort_dir = ToggleButton::new();
    sort_dir.set_tooltip_text(Some("Sort direction"));
    sort_dir.set_valign(gtk4::Align::Center);
    let sort_controls = GtkBox::new(Orientation::Horizontal, 6);
    sort_controls.set_valign(gtk4::Align::Center);
    sort_controls.set_margin_start(18);
    sort_controls.append(&sort_icon);
    sort_controls.append(&sort_field);
    sort_controls.append(&sort_dir);

    let size_scale = Scale::with_range(
        Orientation::Horizontal,
        COVER_SIZE_MIN as f64,
        COVER_SIZE_MAX as f64,
        10.0,
    );
    size_scale.set_size_request(120, -1);
    size_scale.set_draw_value(false);
    size_scale.set_tooltip_text(Some("Album art size"));
    size_scale.set_valign(gtk4::Align::Center);
    size_scale.set_margin_start(18);

    let list_toggle = ToggleButton::new();
    list_toggle.set_icon_name("view-list-symbolic");
    list_toggle.set_tooltip_text(Some("Track list"));
    list_toggle.set_active(true);
    let grid_toggle = ToggleButton::new();
    grid_toggle.set_icon_name("view-grid-symbolic");
    grid_toggle.set_tooltip_text(Some("Album grid"));
    grid_toggle.set_group(Some(&list_toggle));
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

    let content = Stack::new();
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_named(&library_view.widget, Some(ViewMode::List.child_name()));
    content.add_named(&album_grid.widget, Some(ViewMode::Grid.child_name()));

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

    let player_bar = PlayerBar::new(player.clone(), initial_volume);

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

    let (watch_tx, watch_rx) = mpsc::channel::<()>();

    let mw = MainWindow {
        db,
        db_path,
        player,
        library_view,
        album_grid,
        filter_sidebar,
        queue_view,
        player_bar,
        window: window.clone(),
        folder_list,
        status_label,
        scan_spinner,
        scan_status,
        scan_indicator,
        current_filter: Rc::new(RefCell::new(LibraryFilter::All)),
        current_sort: Rc::new(RefCell::new(initial_sort)),
        current_queue: Rc::new(RefCell::new(Vec::new())),
        folder_watcher: Rc::new(RefCell::new(None)),
        watch_tx,
    };

    // Restore cover size and wire the size slider.
    size_scale.set_value(initial_cover_size as f64);
    mw.album_grid.set_cover_size(initial_cover_size);
    {
        let mw = mw.clone();
        size_scale.connect_value_changed(move |scale| {
            let size = scale.value() as i32;
            mw.album_grid.set_cover_size(size);
            mw.settings().set_cover_size(size);
        });
    }

    // Restore sort controls and wire sort changes.
    {
        let sort = *mw.current_sort.borrow();
        sort_field.set_selected(sort_field_index(sort.field));
        sort_dir.set_active(matches!(sort.direction, SortDirection::Descending));
        sort_dir.set_icon_name(direction_icon(sort.direction));
    }
    {
        let mw = mw.clone();
        sort_field.connect_selected_notify(move |dropdown| {
            mw.current_sort.borrow_mut().field = sort_field_at(dropdown.selected());
            let sort = *mw.current_sort.borrow();
            mw.settings().set_album_sort(sort);
            mw.refresh_album_grid_with(&mw.current_filter.borrow(), sort);
        });
    }
    {
        let mw = mw.clone();
        sort_dir.connect_toggled(move |btn| {
            let direction = if btn.is_active() {
                SortDirection::Descending
            } else {
                SortDirection::Ascending
            };
            mw.current_sort.borrow_mut().direction = direction;
            btn.set_icon_name(direction_icon(direction));
            let sort = *mw.current_sort.borrow();
            mw.settings().set_album_sort(sort);
            mw.refresh_album_grid_with(&mw.current_filter.borrow(), sort);
        });
    }

    // Restore the view mode and wire the view toggles.
    {
        match initial_view_mode {
            ViewMode::List => list_toggle.set_active(true),
            ViewMode::Grid => grid_toggle.set_active(true),
        }
        content.set_visible_child_name(initial_view_mode.child_name());
        toggle_grid_controls(matches!(initial_view_mode, ViewMode::Grid));
    }
    {
        let content = content.clone();
        let mw = mw.clone();
        let toggle_grid_controls = Rc::clone(&toggle_grid_controls);
        list_toggle.connect_toggled(move |btn| {
            if btn.is_active() {
                content.set_visible_child_name(ViewMode::List.child_name());
                mw.settings()
                    .set_view_mode_name(ViewMode::List.child_name());
                toggle_grid_controls(false);
            }
        });
    }
    {
        let content = content.clone();
        let mw = mw.clone();
        let toggle_grid_controls = Rc::clone(&toggle_grid_controls);
        grid_toggle.connect_toggled(move |btn| {
            if btn.is_active() {
                content.set_visible_child_name(ViewMode::Grid.child_name());
                mw.settings()
                    .set_view_mode_name(ViewMode::Grid.child_name());
                toggle_grid_controls(true);
            }
        });
    }

    // Persist volume changes.
    {
        let this = mw.clone();
        mw.player_bar.connect_volume_changed(move |percent| {
            this.settings().set_volume(percent);
        });
    }

    // Initial population of library views and folder watcher.
    mw.refresh_folder_list();
    mw.refresh_sidebar();
    mw.refresh_album_grid_with(&mw.current_filter.borrow(), *mw.current_sort.borrow());
    mw.rewatch();
    if let Ok(tracks) = mw.db.tracks_for(&LibraryFilter::All) {
        mw.library_view.set_tracks(tracks);
    }

    // Restore the queue from the previous session.
    {
        let restored = if initial_queue_ids.is_empty() {
            Vec::new()
        } else {
            mw.db.tracks_by_ids(&initial_queue_ids).unwrap_or_default()
        };
        if !restored.is_empty() {
            let start = initial_queue_current
                .and_then(|id| restored.iter().position(|t| t.id.value() == id))
                .unwrap_or(0);
            let position = SeekPosition::from_millis(initial_queue_position_ms);
            mw.queue_view.set_tracks(restored.clone());
            mw.queue_view.set_current(restored.get(start).map(|t| t.id));
            if let Some(track) = restored.get(start) {
                mw.player_bar.set_track(track);
            }
            mw.player.send(PlayerCommand::RestorePaused {
                tracks: restored.clone(),
                start,
                position,
            });
            *mw.current_queue.borrow_mut() = restored;
        }
    }

    // Sidebar selection filters both views.
    {
        let this = mw.clone();
        mw.filter_sidebar.connect_filter_selected(move |filter| {
            *this.current_filter.borrow_mut() = filter.clone();
            match this.db.tracks_for(&filter) {
                Ok(tracks) => this.library_view.set_tracks(tracks),
                Err(e) => tracing::error!("Filter query failed: {e}"),
            }
            this.refresh_album_grid_with(&filter, *this.current_sort.borrow());
        });
    }

    // Album drawer opens when the user clicks a cover; it needs the album's tracks.
    {
        let this = mw.clone();
        mw.album_grid.set_track_provider(move |summary| {
            this.db
                .tracks_for(&LibraryFilter::ByAlbum(summary.album.clone()))
                .unwrap_or_else(|e| {
                    tracing::error!("Album query failed: {e}");
                    Vec::new()
                })
        });
    }

    // Cover click enqueues the whole album.
    {
        let this = mw.clone();
        mw.album_grid
            .connect_album_activated(move |tracks| this.enqueue(tracks, 0));
    }

    // Track click in album drawer enqueues just that track.
    {
        let this = mw.clone();
        mw.album_grid.connect_track_activated(move |tracks, index| {
            if let Some(track) = tracks.get(index) {
                this.enqueue(vec![track.clone()], 0);
            }
        });
    }

    // Track double-click in the library list enqueues just that track.
    {
        let this = mw.clone();
        mw.library_view
            .connect_track_activated(move |tracks, index| {
                if let Some(track) = tracks.get(index) {
                    this.enqueue(vec![track.clone()], 0);
                }
            });
    }

    // Queue row click jumps to that position in the current queue.
    {
        let this = mw.clone();
        mw.queue_view.connect_track_selected(move |index| {
            let tracks = this.current_queue.borrow().clone();
            this.enqueue(tracks, index);
        });
    }

    // Scan button.
    {
        let mw = mw.clone();
        scan_btn.connect_clicked(move |_| mw.start_scan());
    }

    // Add-folder button: pick a folder, persist, re-watch, scan.
    {
        let mw = mw.clone();
        add_btn.connect_clicked(move |_| {
            let mw = mw.clone();
            glib::spawn_future_local(async move {
                let dialog = FileDialog::new();
                dialog.set_title("Add Music Folder");
                let Ok(file) = dialog.select_folder_future(Some(&mw.window)).await else {
                    return;
                };
                let Some(path) = file.path() else { return };
                let Ok(folder) = LibraryFolder::new(path) else {
                    return;
                };
                if let Err(e) = mw.db.add_folder(&folder) {
                    tracing::error!("Failed to add folder: {e}");
                    return;
                }
                mw.refresh_folder_list();
                mw.rewatch();
                mw.start_scan();
            });
        });
    }

    // Debounced auto-rescan: coalesce fs events and rescan once changes settle.
    {
        let mw = mw.clone();
        let dirty = Rc::new(Cell::new(false));
        glib::timeout_add_local(Duration::from_millis(700), move || {
            let mut changed = false;
            while watch_rx.try_recv().is_ok() {
                changed = true;
            }
            if changed {
                dirty.set(true);
            } else if dirty.replace(false) {
                mw.start_scan();
            }
            glib::ControlFlow::Continue
        });
    }

    // Poll player state every 250 ms: update the player bar and persist position.
    {
        let mw = mw.clone();
        let mut last_shown: Option<TrackId> = None;
        let mut last_saved_secs: Option<u64> = None;
        glib::timeout_add_local(Duration::from_millis(250), move || {
            while let Ok(state) = state_rx.try_recv() {
                let track_id = state.current_track();
                if track_id != last_shown {
                    last_shown = track_id;
                    match track_id {
                        Some(id) => {
                            if let Some(track) = mw
                                .current_queue
                                .borrow()
                                .iter()
                                .find(|t| t.id == id)
                                .cloned()
                            {
                                mw.player_bar.set_track(&track);
                            }
                            mw.queue_view.set_current(Some(id));
                            mw.settings().set_queue_current(id);
                        }
                        None => mw.queue_view.set_current(None),
                    }
                }
                mw.player_bar.update_state(&state);
                if let Some(position) = state.position()
                    && last_saved_secs != Some(position.as_secs())
                {
                    last_saved_secs = Some(position.as_secs());
                    let millis = position.as_duration().as_millis() as u64;
                    mw.settings().set_queue_position_millis(millis);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    window
}

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

fn sort_field_index(field: AlbumSortField) -> u32 {
    SORT_FIELDS.iter().position(|f| *f == field).unwrap_or(0) as u32
}

fn sort_field_at(index: u32) -> AlbumSortField {
    SORT_FIELDS
        .get(index as usize)
        .copied()
        .unwrap_or(AlbumSortField::AlbumArtist)
}

fn direction_icon(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "view-sort-ascending-symbolic",
        SortDirection::Descending => "view-sort-descending-symbolic",
    }
}
