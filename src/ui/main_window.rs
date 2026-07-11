use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use async_channel::Sender;
use gtk4::Application;
use gtk4::ApplicationWindow;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::DropDown;
use gtk4::Expander;
use gtk4::FileDialog;
use gtk4::GestureClick;
use gtk4::HeaderBar;
use gtk4::Image;
use gtk4::Label;
use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::Overlay;
use gtk4::Paned;
use gtk4::PropagationPhase;
use gtk4::Scale;
use gtk4::ScrolledWindow;
use gtk4::Spinner;
use gtk4::Stack;
use gtk4::ToggleButton;
use gtk4::gdk::ModifierType;
use gtk4::prelude::*;

use crate::library::album::AlbumSort;
use crate::library::album::AlbumSortField;
use crate::library::album::ArtKey;
use crate::library::album::SortDirection;
use crate::library::column::ColumnPrefs;
use crate::library::db::Db;
use crate::library::db::LibraryFolder;
use crate::library::filter::LibraryFilter;
use crate::library::metadata_edit;
use crate::library::metadata_edit::TrackEdit;
use crate::library::scan::ScanEvent;
use crate::library::scan::spawn_scan;
use crate::library::settings::COVER_SIZE_MAX;
use crate::library::settings::COVER_SIZE_MIN;
use crate::library::settings::Settings;
use crate::library::track::AlbumArtData;
use crate::library::track::Track;
use crate::library::track::TrackId;
use crate::library::view_mode::ViewMode;
use crate::library::watch::FolderWatcher;
use crate::library::watch::watch_folders;
use crate::library::window_state::WindowMessage;
use crate::library::window_state::WindowState;
use crate::library::window_state::reduce;
use crate::player::PlaybackState;
use crate::player::PlayerCommand;
use crate::player::PlayerHandle;
use crate::player::SeekPosition;
use crate::ui::album_grid::AlbumGrid;
use crate::ui::column_picker::ColumnPicker;
use crate::ui::edit_dialog;
use crate::ui::library_view::LibraryView;
use crate::ui::player_bar::PlayerBar;
use crate::ui::queue_view::QueueView;
use crate::ui::sidebar::Sidebar;

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
.album-cell {
    padding: 10px;
    border-radius: 10px;
}
.album-selected {
    background-color: rgba(0, 0, 0, 0.24);
}
.album-multi-selected {
    background-color: rgba(60, 120, 220, 0.28);
}
.album-drawer {
    background-color: rgba(0, 0, 0, 0.24);
    border-radius: 10px;
}
.album-drawer list,
.album-drawer row {
    background-color: transparent;
}
.album-drawer row:hover {
    background-color: rgba(255, 255, 255, 0.12);
}
.album-drawer row:selected {
    background-color: rgba(60, 120, 220, 0.35);
}
";

/// Every persisted setting read once at startup, before `db` moves into `Context`.
struct InitialSettings {
    cover_size: i32,
    sort: AlbumSort,
    view_mode: ViewMode,
    volume: f64,
    queue_ids: Vec<TrackId>,
    queue_current: Option<i64>,
    queue_position_ms: u64,
    window_size: Option<(i32, i32)>,
    window_maximized: bool,
    column_prefs: ColumnPrefs,
    left_sidebar_open: bool,
    left_sidebar_width: i32,
    right_sidebar_open: bool,
    right_sidebar_width: i32,
}

impl InitialSettings {
    fn read(db: &Db) -> Self {
        let s = Settings::new(db);
        Self {
            cover_size: s.cover_size(),
            sort: s.album_sort(),
            view_mode: s.view_mode().unwrap_or(ViewMode::List),
            volume: s.volume(),
            queue_ids: s.queue_track_ids(),
            queue_current: s.queue_current_id(),
            queue_position_ms: s.queue_position_millis(),
            window_size: s.window_size(),
            window_maximized: s.window_maximized(),
            column_prefs: s.column_prefs(),
            left_sidebar_open: s.left_sidebar_open(),
            left_sidebar_width: s.left_sidebar_width(),
            right_sidebar_open: s.right_sidebar_open(),
            right_sidebar_width: s.right_sidebar_width(),
        }
    }
}

/// Everything `apply` needs to turn a reduced `WindowState` + the `WindowMessage` that
/// produced it into DB queries, player commands, and widget updates. Owned
/// solely by the one dispatch loop in `build` — never `Clone`, never shared,
/// so none of its fields need `Rc<RefCell<_>>`.
struct Context {
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    library_view: LibraryView,
    album_grid: AlbumGrid,
    filter_sidebar: Sidebar,
    queue_view: QueueView,
    player_bar: PlayerBar,
    window: ApplicationWindow,
    folder_list: ListBox,
    status_label: Label,
    scan_spinner: Spinner,
    scan_status: Label,
    scan_indicator: GtkBox,
    content: Stack,
    toggle_grid_controls: Rc<dyn Fn(ViewMode)>,
    watch_tx: Sender<()>,
    tx: Sender<WindowMessage>,
    // Track-change display bookkeeping — plain fields, not `Cell`, since
    // `apply` takes `&mut self` and nothing else ever touches `Context`.
    last_shown: Option<TrackId>,
    last_saved_secs: Option<u64>,
}

impl Context {
    /// Builds every widget, wiring each GTK signal to send a `WindowMessage` (or, for
    /// the handful of concerns that never touch `WindowState`, a direct
    /// closure). Returns the `Context` plus the raw folder-watcher receiver,
    /// which `build` bridges into the `WindowMessage` stream separately.
    fn new(
        app: &Application,
        db: Rc<Db>,
        db_path: PathBuf,
        player: PlayerHandle,
        initial: &InitialSettings,
        tx: Sender<WindowMessage>,
    ) -> (Self, async_channel::Receiver<()>) {
        install_styles();

        let header = HeaderBar::new();
        // Under a tiling WM the minimize/maximize buttons are unusable and render
        // with a mismatched background on focus changes; show only the close button.
        header.set_decoration_layout(Some(":close"));

        // Built early (before `root` exists) so it can be passed to
        // `wire_library_view`/`wire_album_grid`, which parent the edit
        // dialogs on it; its child is attached later via `set_child`, once
        // `root` is built, in place of the builder's usual `.child(&root)`.
        let (initial_width, initial_height) = initial.window_size.unwrap_or((1200, 700));
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Music Player")
            .default_width(initial_width)
            .default_height(initial_height)
            .maximized(initial.window_maximized)
            .build();

        // Packed first/last so the sidebar toggles stay the outermost header
        // widgets; wiring to the paned they control happens once those exist,
        // further down. `set_active` runs before `connect_toggled` so
        // restoring the persisted state doesn't re-trigger a save.
        let left_sidebar_toggle = ToggleButton::new();
        left_sidebar_toggle.set_icon_name("sidebar-show-symbolic");
        left_sidebar_toggle.set_tooltip_text(Some("Toggle filters sidebar"));
        left_sidebar_toggle.set_active(initial.left_sidebar_open);
        header.pack_start(&left_sidebar_toggle);

        let right_sidebar_toggle = ToggleButton::new();
        right_sidebar_toggle.set_icon_name("sidebar-show-right-symbolic");
        right_sidebar_toggle.set_tooltip_text(Some("Toggle queue sidebar"));
        right_sidebar_toggle.set_active(initial.right_sidebar_open);
        header.pack_end(&right_sidebar_toggle);

        let add_btn = Button::from_icon_name("folder-new-symbolic");
        add_btn.set_tooltip_text(Some("Add music folder"));
        header.pack_start(&add_btn);

        let scan_btn = Button::from_icon_name("view-refresh-symbolic");
        scan_btn.set_tooltip_text(Some("Scan library"));
        header.pack_start(&scan_btn);
        wire_scan_button(&scan_btn, &tx);

        let (sort_controls, sort_field, sort_dir) = build_sort_controls();
        wire_sort_controls(&sort_field, &sort_dir, initial.sort, &tx);
        let size_scale = build_size_scale();
        let (list_toggle, grid_toggle) = build_view_toggles();
        let column_picker = ColumnPicker::new(initial.column_prefs.clone());
        wire_column_picker(&column_picker, &tx);

        // One header slot generalised over both view modes: the column
        // picker in list view, the cover-size slider in grid view — never
        // both, so they share a single spot instead of two.
        let view_settings = Stack::new();
        view_settings.set_hhomogeneous(false);
        view_settings.set_vhomogeneous(false);
        view_settings.add_named(&column_picker.widget, Some("columns"));
        view_settings.add_named(&size_scale, Some("cover_size"));

        header.pack_start(&sort_controls);
        header.pack_end(&grid_toggle);
        header.pack_end(&list_toggle);
        header.pack_end(&view_settings);

        // Shows or hides the grid-only sort controls, and switches the
        // shared header slot between the list-only column picker and the
        // grid-only cover-size slider.
        let toggle_grid_controls: Rc<dyn Fn(ViewMode)> = {
            let sort_controls = sort_controls.clone();
            let view_settings = view_settings.clone();
            Rc::new(move |mode: ViewMode| {
                let grid_visible = matches!(mode, ViewMode::Grid);
                sort_controls.set_visible(grid_visible);
                view_settings.set_visible_child_name(if grid_visible {
                    "cover_size"
                } else {
                    "columns"
                });
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
        wire_sidebar(&filter_sidebar, &tx);
        let queue_view = QueueView::new();
        wire_queue_view(&queue_view, &tx);
        let left_sidebar = build_left_sidebar_box(&filter_sidebar);
        let right_sidebar = build_right_sidebar_box(&queue_view, &folder_list, &status_label);

        let library_view = LibraryView::new(&initial.column_prefs);
        wire_library_view(&library_view, &db, &db_path, &window, &status_label, &tx);

        let album_grid = AlbumGrid::new();
        wire_cover_size(&size_scale, &album_grid, initial.cover_size, &tx);
        wire_album_grid(&album_grid, &db, &db_path, &window, &status_label, &tx);
        wire_click_elsewhere_deselect(&window, &library_view, &album_grid);

        let content = Stack::new();
        content.set_hexpand(true);
        content.set_vexpand(true);
        content.add_named(&library_view.widget, Some(ViewMode::List.child_name()));
        content.add_named(&album_grid.widget, Some(ViewMode::Grid.child_name()));

        let (scan_spinner, scan_status, scan_indicator) = build_scan_indicator();

        let content_overlay = Overlay::new();
        content_overlay.set_child(Some(&content));
        content_overlay.add_overlay(&scan_indicator);

        let inner_paned = Paned::new(Orientation::Horizontal);
        inner_paned.set_start_child(Some(&content_overlay));
        inner_paned.set_end_child(Some(&right_sidebar));
        inner_paned.set_resize_end_child(false);
        inner_paned.set_hexpand(true);
        inner_paned.set_vexpand(true);
        // The right sidebar's width is `inner_paned`'s total width minus its
        // divider position, so before the window is first realized (no real
        // width to measure) this is only an estimate; it self-corrects the
        // moment the divider is dragged or the toggle is used, both of which
        // read `inner_paned`'s actual allocated width.
        let estimated_content_width = initial_width
            - if initial.left_sidebar_open {
                initial.left_sidebar_width
            } else {
                0
            };
        inner_paned.set_position(if initial.right_sidebar_open {
            (estimated_content_width - initial.right_sidebar_width).max(0)
        } else {
            estimated_content_width
        });

        let paned = Paned::new(Orientation::Horizontal);
        paned.set_start_child(Some(&left_sidebar));
        paned.set_end_child(Some(&inner_paned));
        paned.set_position(if initial.left_sidebar_open {
            initial.left_sidebar_width
        } else {
            0
        });
        paned.set_hexpand(true);
        paned.set_vexpand(true);

        wire_left_sidebar_toggle(&left_sidebar_toggle, &paned, Rc::clone(&db));
        wire_right_sidebar_toggle(&right_sidebar_toggle, &inner_paned, Rc::clone(&db));

        wire_view_toggles(
            &list_toggle,
            &grid_toggle,
            &content,
            Rc::clone(&toggle_grid_controls),
            initial.view_mode,
            &tx,
        );

        let player_bar = PlayerBar::new(player.clone(), initial.volume);
        wire_volume(&player_bar, &tx);

        let root = GtkBox::new(Orientation::Vertical, 0);
        root.append(&paned);
        root.append(&player_bar.widget);
        window.set_child(Some(&root));
        window.set_titlebar(Some(&header));

        wire_window_geometry(&window, &db);
        wire_add_folder_button(&add_btn, &window, &tx);

        let (watch_tx, watch_rx) = async_channel::unbounded::<()>();

        let ctx = Self {
            db,
            db_path,
            player,
            library_view,
            album_grid,
            filter_sidebar,
            queue_view,
            player_bar,
            window,
            folder_list,
            status_label,
            scan_spinner,
            scan_status,
            scan_indicator,
            content,
            toggle_grid_controls,
            watch_tx,
            tx,
            last_shown: None,
            last_saved_secs: None,
        };

        (ctx, watch_rx)
    }

    fn settings(&self) -> Settings<'_> {
        Settings::new(&self.db)
    }

    /// The impure half of handling `msg`: DB queries, player commands, widget
    /// updates, using `state` (already reduced) and `watcher` (infrastructure
    /// state, not part of `WindowState` — see `library::window_state`).
    fn apply(
        &mut self,
        state: &WindowState,
        msg: &WindowMessage,
        watcher: &mut Option<FolderWatcher>,
    ) {
        match msg {
            WindowMessage::FilterSelected(filter) => {
                match self.db.tracks_for(filter) {
                    Ok(tracks) => self.library_view.set_tracks(tracks),
                    Err(e) => tracing::error!("Filter query failed: {e}"),
                }
                self.refresh_album_grid_with(&state.filter, state.sort);
            }
            WindowMessage::SortFieldChanged(_) | WindowMessage::SortDirectionChanged(_) => {
                self.settings().set_album_sort(state.sort);
                self.refresh_album_grid_with(&state.filter, state.sort);
            }
            WindowMessage::ViewModeChanged(mode) => {
                self.content.set_visible_child_name(mode.child_name());
                self.settings().set_view_mode(*mode);
                (self.toggle_grid_controls)(*mode);
            }
            WindowMessage::CoverSizeChanged(size) => {
                self.album_grid.set_cover_size(*size);
                self.settings().set_cover_size(*size);
            }
            WindowMessage::ColumnPrefsChanged(prefs) => {
                self.library_view.set_column_prefs(prefs);
                self.settings().set_column_prefs(prefs);
            }
            WindowMessage::ColumnWidthChanged(field, width) => {
                // The header drag already resized the column natively — no
                // need to rebuild `library_view`, just persist the result.
                self.settings().set_column_width(*field, Some(*width));
            }
            WindowMessage::VolumeChanged(percent) => {
                self.settings().set_volume(*percent);
            }
            WindowMessage::Enqueue(tracks, start) => {
                self.do_enqueue(tracks.clone(), *start);
            }
            WindowMessage::QueueTrackSelected(index) => {
                self.do_enqueue(state.queue.clone(), *index);
            }
            WindowMessage::AppendToQueue(tracks) => {
                self.player.send(PlayerCommand::Enqueue(tracks.clone()));
            }
            WindowMessage::PlayerQueueChanged(tracks) => {
                self.apply_queue_changed(tracks);
            }
            WindowMessage::PlayerStateChanged(playback_state) => {
                self.apply_player_state(state, playback_state);
            }
            WindowMessage::ScanRequested | WindowMessage::RescanRequested => {
                self.start_scan();
            }
            WindowMessage::ScanEvent(event) => {
                self.apply_scan_event(state, event);
            }
            WindowMessage::FolderAdded(folder) => {
                if let Err(e) = self.db.add_folder(folder) {
                    tracing::error!("Failed to add folder: {e}");
                    return;
                }
                self.refresh_folder_list();
                self.rewatch(watcher);
                self.start_scan();
            }
            WindowMessage::FolderRemoved(folder) => {
                if let Err(e) = self.db.remove_folder(folder) {
                    tracing::error!("Failed to remove folder: {e}");
                    return;
                }
                self.refresh_folder_list();
                self.rewatch(watcher);
                self.refresh_views(state);
                self.prune_queue(state);
            }
            WindowMessage::EditSaved {
                affects_art,
                outcome,
            } => {
                // Only an edit that changed art bytes or ArtKey fields can
                // stale the texture cache, and only for the album(s) actually
                // touched — clearing the whole cache would blank every cover
                // in the grid while they all re-decode, for a single edit.
                if *affects_art {
                    let keys: Vec<ArtKey> =
                        outcome.saved.iter().filter_map(ArtKey::for_track).collect();
                    self.album_grid.invalidate_art_for(&keys);
                }
                self.refresh_views(state);
                self.sync_queue_after_edit(state, &outcome.saved);
                match &outcome.failed {
                    None => self
                        .status_label
                        .set_text(&format!("Updated {} track(s)", outcome.saved.len())),
                    Some(e) => {
                        tracing::error!("Edit failed: {e}");
                        self.status_label.set_text(&format!(
                            "Updated {} track(s), then failed: {e}",
                            outcome.saved.len()
                        ));
                    }
                }
            }
        }
    }

    /// Sends `tracks` to the player positioned at `start`. `queue_view` itself
    /// isn't updated here — the player is the sole owner of the queue, so the
    /// `PlayerQueueChanged` echo that follows is what renders it.
    fn do_enqueue(&self, tracks: Vec<Track>, start: usize) {
        if tracks.is_empty() {
            return;
        }
        let start = start.min(tracks.len() - 1);
        let current_id = tracks[start].id;
        self.player_bar.set_track(&tracks[start]);
        self.settings().set_queue_current(current_id);
        self.player.send(PlayerCommand::PlayQueue { tracks, start });
    }

    /// Renders the player's authoritative queue snapshot and persists it.
    fn apply_queue_changed(&self, tracks: &[Track]) {
        self.queue_view.set_tracks(tracks.to_vec());
        self.settings()
            .set_queue(&tracks.iter().map(|t| t.id).collect::<Vec<_>>());
        // `set_tracks` rebuilds every row, dropping the highlight — restore it
        // if the currently playing track is still in the new list.
        let current = self
            .settings()
            .queue_current_id()
            .map(TrackId::new)
            .filter(|id| tracks.iter().any(|t| t.id == *id));
        self.queue_view.set_current(current);
    }

    /// Updates the player bar, queue highlight, and status label for a new
    /// playback state, and throttles position persistence to once per second.
    fn apply_player_state(&mut self, state: &WindowState, playback_state: &PlaybackState) {
        let track_id = playback_state.current_track();
        if track_id != self.last_shown {
            self.last_shown = track_id;
            match track_id {
                Some(id) => {
                    let track = state.queue.iter().find(|t| t.id == id).cloned();
                    if let Some(track) = track {
                        self.player_bar.set_track(&track);
                    }
                    self.queue_view.set_current(Some(id));
                    self.settings().set_queue_current(id);
                }
                None => self.queue_view.set_current(None),
            }
        }
        if let PlaybackState::Failed { error, .. } = playback_state {
            self.status_label
                .set_text(&format!("Playback error: {error}"));
        }
        self.player_bar.update_state(playback_state);
        if let Some(position) = playback_state.position()
            && self.last_saved_secs != Some(position.as_secs())
        {
            self.last_saved_secs = Some(position.as_secs());
            let millis = position.as_duration().as_millis() as u64;
            self.settings().set_queue_position_millis(millis);
        }
    }

    /// Reloads the tracks/sidebar/grid after the library changes, keeping the
    /// active filter applied.
    fn refresh_views(&self, state: &WindowState) {
        match self.db.tracks_for(&state.filter) {
            Ok(tracks) => self.library_view.set_tracks(tracks),
            Err(e) => tracing::error!("Reload after library change failed: {e}"),
        }
        self.refresh_sidebar();
        self.refresh_album_grid_with(&state.filter, state.sort);
    }

    fn refresh_sidebar(&self) {
        let genres = match self.db.distinct_genres() {
            Ok(v) => v,
            Err(e) => return self.status_label.set_text(&format!("Error: {e}")),
        };
        let artists = match self.db.distinct_artists() {
            Ok(v) => v,
            Err(e) => return self.status_label.set_text(&format!("Error: {e}")),
        };
        self.filter_sidebar.populate(genres, artists);
    }

    fn refresh_album_grid_with(&self, filter: &LibraryFilter, sort: AlbumSort) {
        match self.db.album_summaries_for(filter, &sort) {
            Ok(albums) => self.album_grid.set_albums(albums),
            Err(e) => self.status_label.set_text(&format!("Error: {e}")),
        }
    }

    /// Re-arms the folder watcher for the current folder set.
    fn rewatch(&self, watcher: &mut Option<FolderWatcher>) {
        *watcher = None;
        let folders = match self.db.list_folders() {
            Ok(f) => f,
            Err(e) => {
                self.status_label.set_text(&format!("Error: {e}"));
                return;
            }
        };
        if folders.is_empty() {
            return;
        }
        match watch_folders(&folders, self.watch_tx.clone()) {
            Ok(w) => *watcher = Some(w),
            Err(e) => tracing::error!("Failed to watch folders: {e}"),
        }
    }

    /// Asks the player to adopt whatever queue tracks still exist in the
    /// library, if the library change dropped any. The player is the sole
    /// owner of the queue, so this is a request, not a direct mutation — the
    /// resulting `PlayerQueueChanged` is what actually updates the UI.
    fn prune_queue(&self, state: &WindowState) {
        let surviving = match self.db.list_tracks() {
            Ok(tracks) => tracks.into_iter().map(|t| t.id).collect::<HashSet<_>>(),
            Err(e) => {
                tracing::error!("Failed to check queue against library: {e}");
                return;
            }
        };
        let remaining: Vec<Track> = state
            .queue
            .iter()
            .filter(|t| surviving.contains(&t.id))
            .cloned()
            .collect();
        if remaining.len() != state.queue.len() {
            self.player.send(PlayerCommand::SetQueue(remaining));
        }
    }

    /// Replaces any of `state.queue`'s tracks that were just edited (matched
    /// by id) with their saved version, and asks the player to adopt the
    /// result if anything actually changed. Mirrors `prune_queue`'s shape:
    /// the player is still the sole queue owner, so this is a request — the
    /// real update arrives back through the `PlayerQueueChanged` echo.
    /// `PlayerCommand::SetQueue` keeps playback running when the current
    /// track's id survives the swap, so editing the playing track never
    /// interrupts it.
    fn sync_queue_after_edit(&self, state: &WindowState, saved: &[Track]) {
        if saved.is_empty() {
            return;
        }
        let by_id: HashMap<TrackId, &Track> = saved.iter().map(|t| (t.id, t)).collect();
        let updated: Vec<Track> = state
            .queue
            .iter()
            .map(|t| {
                by_id
                    .get(&t.id)
                    .copied()
                    .cloned()
                    .unwrap_or_else(|| t.clone())
            })
            .collect();
        if updated != state.queue {
            self.player.send(PlayerCommand::SetQueue(updated));
        }
    }

    /// Starts a background scan of every watched folder, showing a spinner
    /// overlay, and forwards its events into the `WindowMessage` stream as `ScanEvent`.
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

        let rx = spawn_scan(self.db_path.clone(), folders);
        let tx = self.tx.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                let finished = matches!(event, ScanEvent::Finished(_));
                if tx.send(WindowMessage::ScanEvent(event)).await.is_err() || finished {
                    break;
                }
            }
        });
    }

    fn apply_scan_event(&self, state: &WindowState, event: &ScanEvent) {
        match event {
            ScanEvent::Progress(n) => {
                let msg = format!("Scanning… {n} files");
                self.status_label.set_text(&msg);
                self.scan_status.set_text(&msg);
            }
            ScanEvent::Finished(Ok(summary)) => {
                self.status_label.set_text(&format!(
                    "Indexed {} tracks, removed {}",
                    summary.indexed, summary.removed
                ));
                self.scan_spinner.stop();
                self.scan_indicator.set_visible(false);
                // A rescan may have changed an album's embedded art; the
                // texture cache is keyed only by ArtKey, so it must be
                // dropped before the grid repopulates.
                self.album_grid.invalidate_art_cache();
                self.refresh_views(state);
            }
            ScanEvent::Finished(Err(e)) => {
                self.status_label.set_text(&format!("Scan error: {e}"));
                self.scan_spinner.stop();
                self.scan_indicator.set_visible(false);
            }
        }
    }

    fn refresh_folder_list(&self) {
        let list = &self.folder_list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let folders = match self.db.list_folders() {
            Ok(f) => f,
            Err(e) => {
                self.status_label.set_text(&format!("Error: {e}"));
                return;
            }
        };
        for folder in folders {
            list.append(&folder_row(folder, &self.tx));
        }
    }

    /// Restores the queue from the previous session, paused at its saved
    /// position. Runs once during bootstrap, before the dispatch loop takes
    /// ownership of `state` — so it writes `state.queue` directly rather than
    /// through a `WindowMessage`.
    fn restore_queue(&self, initial: &InitialSettings, state: &mut WindowState) {
        if initial.queue_ids.is_empty() {
            return;
        }
        let restored = self
            .db
            .tracks_by_ids(&initial.queue_ids)
            .unwrap_or_default();
        if restored.is_empty() {
            return;
        }
        let start = initial
            .queue_current
            .and_then(|id| restored.iter().position(|t| t.id.value() == id))
            .unwrap_or(0);
        let position = SeekPosition::from_millis(initial.queue_position_ms);
        self.queue_view.set_tracks(restored.clone());
        self.queue_view
            .set_current(restored.get(start).map(|t| t.id));
        if let Some(track) = restored.get(start) {
            self.player_bar.set_track(track);
        }
        self.player.send(PlayerCommand::RestorePaused {
            tracks: restored.clone(),
            start,
            position,
        });
        state.queue = restored;
    }
}

fn folder_row(folder: LibraryFolder, tx: &Sender<WindowMessage>) -> ListBoxRow {
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

    let tx = tx.clone();
    remove_btn.connect_clicked(move |_| {
        let _ = tx.send_blocking(WindowMessage::FolderRemoved(folder.clone()));
    });

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}

/// Restores the sort controls and wires further changes to send `WindowMessage`s.
fn wire_sort_controls(
    sort_field: &DropDown,
    sort_dir: &ToggleButton,
    initial: AlbumSort,
    tx: &Sender<WindowMessage>,
) {
    sort_field.set_selected(sort_field_index(initial.field));
    sort_dir.set_active(matches!(initial.direction, SortDirection::Descending));
    sort_dir.set_icon_name(direction_icon(initial.direction));

    let tx_field = tx.clone();
    sort_field.connect_selected_notify(move |dropdown| {
        let _ = tx_field.send_blocking(WindowMessage::SortFieldChanged(sort_field_at(
            dropdown.selected(),
        )));
    });

    let tx_dir = tx.clone();
    sort_dir.connect_toggled(move |btn| {
        let direction = if btn.is_active() {
            SortDirection::Descending
        } else {
            SortDirection::Ascending
        };
        btn.set_icon_name(direction_icon(direction));
        let _ = tx_dir.send_blocking(WindowMessage::SortDirectionChanged(direction));
    });
}

/// Restores the cover-size slider and wires further changes to send `WindowMessage`s.
fn wire_cover_size(
    size_scale: &Scale,
    album_grid: &AlbumGrid,
    initial_cover_size: i32,
    tx: &Sender<WindowMessage>,
) {
    size_scale.set_value(initial_cover_size as f64);
    album_grid.set_cover_size(initial_cover_size);

    let tx = tx.clone();
    size_scale.connect_value_changed(move |scale| {
        let _ = tx.send_blocking(WindowMessage::CoverSizeChanged(scale.value() as i32));
    });
}

/// Wires the column picker's change callback to send the new prefs onward.
fn wire_column_picker(column_picker: &ColumnPicker, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    column_picker.connect_changed(move |prefs| {
        let _ = tx.send_blocking(WindowMessage::ColumnPrefsChanged(prefs));
    });
}

/// Restores the active view and wires the list/grid toggle buttons.
fn wire_view_toggles(
    list_toggle: &ToggleButton,
    grid_toggle: &ToggleButton,
    content: &Stack,
    toggle_grid_controls: Rc<dyn Fn(ViewMode)>,
    initial_view_mode: ViewMode,
    tx: &Sender<WindowMessage>,
) {
    match initial_view_mode {
        ViewMode::List => list_toggle.set_active(true),
        ViewMode::Grid => grid_toggle.set_active(true),
    }
    content.set_visible_child_name(initial_view_mode.child_name());
    toggle_grid_controls(initial_view_mode);

    let tx_list = tx.clone();
    list_toggle.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = tx_list.send_blocking(WindowMessage::ViewModeChanged(ViewMode::List));
        }
    });

    let tx_grid = tx.clone();
    grid_toggle.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = tx_grid.send_blocking(WindowMessage::ViewModeChanged(ViewMode::Grid));
        }
    });
}

fn wire_volume(player_bar: &PlayerBar, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    player_bar.connect_volume_changed(move |percent| {
        let _ = tx.send_blocking(WindowMessage::VolumeChanged(percent));
    });
}

fn wire_sidebar(filter_sidebar: &Sidebar, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    filter_sidebar.connect_filter_selected(move |filter| {
        let _ = tx.send_blocking(WindowMessage::FilterSelected(filter));
    });
}

fn wire_queue_view(queue_view: &QueueView, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    queue_view.connect_track_selected(move |index| {
        let _ = tx.send_blocking(WindowMessage::QueueTrackSelected(index));
    });
}

/// Clears both views' multi-selections on a plain left-click anywhere in the
/// window, so a selection doesn't linger indefinitely once the user has moved
/// on to something else. Attached in the capture phase (runs before any
/// widget's own click handling) so it observes every press without consuming
/// it — a click that lands on a row/cover still reaches that widget's own
/// handler afterward and can re-establish a selection there. Restricted to
/// the primary button (a right-click must see the selection exactly as it
/// was, to decide single- vs batch-action) and skipped entirely when
/// ctrl/shift is held (those are what build a multi-selection in the first
/// place).
fn wire_click_elsewhere_deselect(
    window: &ApplicationWindow,
    library_view: &LibraryView,
    album_grid: &AlbumGrid,
) {
    let gesture = GestureClick::new();
    gesture.set_propagation_phase(PropagationPhase::Capture);
    gesture.set_button(gtk4::gdk::BUTTON_PRIMARY);
    let library_view = library_view.clone();
    let album_grid = album_grid.clone();
    gesture.connect_pressed(move |gesture, _, _, _| {
        let mods = gesture.current_event_state();
        if mods.contains(ModifierType::CONTROL_MASK) || mods.contains(ModifierType::SHIFT_MASK) {
            return;
        }
        library_view.clear_selection();
        album_grid.clear_selection();
    });
    window.add_controller(gesture);
}

fn wire_library_view(
    library_view: &LibraryView,
    db: &Rc<Db>,
    db_path: &Path,
    window: &ApplicationWindow,
    status_label: &Label,
    tx: &Sender<WindowMessage>,
) {
    let tx_activated = tx.clone();
    library_view.connect_track_activated(move |tracks, index| {
        if let Some(track) = tracks.get(index) {
            let _ = tx_activated.send_blocking(WindowMessage::Enqueue(vec![track.clone()], 0));
        }
    });

    let tx_enqueue = tx.clone();
    library_view.connect_track_enqueue(move |track| {
        let _ = tx_enqueue.send_blocking(WindowMessage::AppendToQueue(vec![track]));
    });

    let tx_tracks_enqueue = tx.clone();
    library_view.connect_tracks_enqueue(move |tracks| {
        let _ = tx_tracks_enqueue.send_blocking(WindowMessage::AppendToQueue(tracks));
    });

    let tx_resized = tx.clone();
    library_view.connect_column_resized(move |field, width| {
        let _ = tx_resized.send_blocking(WindowMessage::ColumnWidthChanged(field, width));
    });

    let db_edit = Rc::clone(db);
    let db_path_edit = db_path.to_path_buf();
    let window_edit = window.clone();
    let status_edit = status_label.clone();
    let tx_edit = tx.clone();
    library_view.connect_track_edit_requested(move |track| {
        let art = db_edit.art_for_track(track.id).unwrap_or_else(|e| {
            tracing::error!("Art lookup failed: {e}");
            None
        });
        let db_path = db_path_edit.clone();
        let status = status_edit.clone();
        let tx = tx_edit.clone();
        let track_for_save = track.clone();
        edit_dialog::open_track_editor(&window_edit, track, art, move |edit, art| {
            status.set_text("Saving…");
            spawn_edit_save(
                db_path.clone(),
                vec![track_for_save.clone()],
                edit,
                art,
                tx.clone(),
            );
        });
    });

    let db_tracks_edit = Rc::clone(db);
    let db_path_tracks_edit = db_path.to_path_buf();
    let window_tracks_edit = window.clone();
    let status_tracks_edit = status_label.clone();
    let tx_tracks_edit = tx.clone();
    library_view.connect_tracks_edit_requested(move |tracks| {
        let art = tracks.first().and_then(|t| {
            db_tracks_edit.art_for_track(t.id).unwrap_or_else(|e| {
                tracing::error!("Art lookup failed: {e}");
                None
            })
        });
        let db_path = db_path_tracks_edit.clone();
        let status = status_tracks_edit.clone();
        let tx = tx_tracks_edit.clone();
        let tracks_for_save = tracks.clone();
        edit_dialog::open_album_editor(&window_tracks_edit, tracks, art, move |edit, art| {
            status.set_text("Saving…");
            spawn_edit_save(
                db_path.clone(),
                tracks_for_save.clone(),
                edit,
                art,
                tx.clone(),
            );
        });
    });
}

/// The album grid's track and cover-art providers (synchronous DB reads, not
/// messages — the grid needs the answer inline to render a drawer or a cover)
/// plus its activation/enqueue/edit callbacks (which do go through `WindowMessage`).
fn wire_album_grid(
    album_grid: &AlbumGrid,
    db: &Rc<Db>,
    db_path: &Path,
    window: &ApplicationWindow,
    status_label: &Label,
    tx: &Sender<WindowMessage>,
) {
    let db_tracks = Rc::clone(db);
    album_grid.set_track_provider(move |summary| {
        db_tracks
            .tracks_for(&LibraryFilter::ByAlbum(summary.album.clone()))
            .unwrap_or_else(|e| {
                tracing::error!("Album query failed: {e}");
                Vec::new()
            })
    });

    let db_art = Rc::clone(db);
    album_grid.set_art_provider(move |key| {
        db_art.art_for(key).unwrap_or_else(|e| {
            tracing::error!("Art lookup failed: {e}");
            None
        })
    });

    let tx_activated = tx.clone();
    album_grid.connect_album_activated(move |tracks| {
        let _ = tx_activated.send_blocking(WindowMessage::Enqueue(tracks, 0));
    });

    let tx_track_activated = tx.clone();
    album_grid.connect_track_activated(move |tracks, index| {
        if let Some(track) = tracks.get(index) {
            let _ =
                tx_track_activated.send_blocking(WindowMessage::Enqueue(vec![track.clone()], 0));
        }
    });

    let tx_album_enqueue = tx.clone();
    album_grid.connect_album_enqueue(move |tracks| {
        let _ = tx_album_enqueue.send_blocking(WindowMessage::AppendToQueue(tracks));
    });

    let tx_track_enqueue = tx.clone();
    album_grid.connect_track_enqueue(move |track| {
        let _ = tx_track_enqueue.send_blocking(WindowMessage::AppendToQueue(vec![track]));
    });

    let db_album_edit = Rc::clone(db);
    let db_path_album_edit = db_path.to_path_buf();
    let window_album_edit = window.clone();
    let status_album_edit = status_label.clone();
    let tx_album_edit = tx.clone();
    album_grid.connect_album_edit_requested(move |tracks| {
        let art = tracks.first().and_then(ArtKey::for_track).and_then(|key| {
            db_album_edit.art_for(&key).unwrap_or_else(|e| {
                tracing::error!("Art lookup failed: {e}");
                None
            })
        });
        let db_path = db_path_album_edit.clone();
        let status = status_album_edit.clone();
        let tx = tx_album_edit.clone();
        let tracks_for_save = tracks.clone();
        edit_dialog::open_album_editor(&window_album_edit, tracks, art, move |edit, art| {
            status.set_text("Saving…");
            spawn_edit_save(
                db_path.clone(),
                tracks_for_save.clone(),
                edit,
                art,
                tx.clone(),
            );
        });
    });

    let db_track_edit = Rc::clone(db);
    let db_path_track_edit = db_path.to_path_buf();
    let window_track_edit = window.clone();
    let status_track_edit = status_label.clone();
    let tx_track_edit = tx.clone();
    album_grid.connect_track_edit_requested(move |track| {
        let art = db_track_edit.art_for_track(track.id).unwrap_or_else(|e| {
            tracing::error!("Art lookup failed: {e}");
            None
        });
        let db_path = db_path_track_edit.clone();
        let status = status_track_edit.clone();
        let tx = tx_track_edit.clone();
        let track_for_save = track.clone();
        edit_dialog::open_track_editor(&window_track_edit, track, art, move |edit, art| {
            status.set_text("Saving…");
            spawn_edit_save(
                db_path.clone(),
                vec![track_for_save.clone()],
                edit,
                art,
                tx.clone(),
            );
        });
    });

    let tx_tracks_enqueue = tx.clone();
    album_grid.connect_tracks_enqueue(move |tracks| {
        let _ = tx_tracks_enqueue.send_blocking(WindowMessage::AppendToQueue(tracks));
    });

    let db_tracks_edit = Rc::clone(db);
    let db_path_tracks_edit = db_path.to_path_buf();
    let window_tracks_edit = window.clone();
    let status_tracks_edit = status_label.clone();
    let tx_tracks_edit = tx.clone();
    album_grid.connect_tracks_edit_requested(move |tracks| {
        let art = tracks.first().and_then(|t| {
            db_tracks_edit.art_for_track(t.id).unwrap_or_else(|e| {
                tracing::error!("Art lookup failed: {e}");
                None
            })
        });
        let db_path = db_path_tracks_edit.clone();
        let status = status_tracks_edit.clone();
        let tx = tx_tracks_edit.clone();
        let tracks_for_save = tracks.clone();
        edit_dialog::open_album_editor(&window_tracks_edit, tracks, art, move |edit, art| {
            status.set_text("Saving…");
            spawn_edit_save(
                db_path.clone(),
                tracks_for_save.clone(),
                edit,
                art,
                tx.clone(),
            );
        });
    });
}

/// Spawns the background file+DB save, computes whether the edit can change
/// which cover an album shows (before the edit itself is dropped — see
/// `WindowMessage::EditSaved`'s doc comment), and bridges the single result
/// into the `WindowMessage` stream. Mirrors `start_scan`'s forwarding loop,
/// but for one value instead of a stream.
fn spawn_edit_save(
    db_path: PathBuf,
    tracks: Vec<Track>,
    edit: TrackEdit,
    art: Option<AlbumArtData>,
    tx: Sender<WindowMessage>,
) {
    let affects_art = art.is_some() || edit.affects_art_grouping();
    let rx = metadata_edit::spawn_save_edits(db_path, tracks, edit, art);
    glib::spawn_future_local(async move {
        if let Ok(outcome) = rx.recv().await {
            let _ = tx
                .send(WindowMessage::EditSaved {
                    affects_art,
                    outcome,
                })
                .await;
        }
    });
}

fn wire_scan_button(scan_btn: &Button, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    scan_btn.connect_clicked(move |_| {
        let _ = tx.send_blocking(WindowMessage::ScanRequested);
    });
}

/// Add-folder button: pick a folder, then dispatch `WindowMessage::FolderAdded`. The DB
/// write itself happens in `Context::apply` — this only handles the async
/// file-dialog interaction, which doesn't belong in a state transition.
fn wire_add_folder_button(
    add_btn: &Button,
    window: &ApplicationWindow,
    tx: &Sender<WindowMessage>,
) {
    let window = window.clone();
    let tx = tx.clone();
    add_btn.connect_clicked(move |_| {
        let window = window.clone();
        let tx = tx.clone();
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
            let _ = tx.send(WindowMessage::FolderAdded(folder)).await;
        });
    });
}

/// Persist window geometry so the next launch reopens at the same size.
/// `default_width`/`default_height` hold the pre-maximize size even while
/// maximized, but the guard below is a defensive no-op either way: don't
/// overwrite the restore size with whatever a maximized window reports. Never
/// touches `WindowState`, so it stays a direct closure rather than a `WindowMessage`.
fn wire_window_geometry(window: &ApplicationWindow, db: &Rc<Db>) {
    let db = Rc::clone(db);
    window.connect_close_request(move |window| {
        let s = Settings::new(&db);
        if !window.is_maximized() {
            s.set_window_size(window.default_width(), window.default_height());
        }
        s.set_window_maximized(window.is_maximized());
        glib::Propagation::Proceed
    });
}

/// Debounced auto-rescan: wait for an fs event then rescan after 700 ms of
/// silence. If more events arrive during the wait, the timer resets.
fn wire_debounced_watcher(watch_rx: async_channel::Receiver<()>, tx: Sender<WindowMessage>) {
    glib::spawn_future_local(async move {
        while watch_rx.recv().await.is_ok() {
            while watch_rx.try_recv().is_ok() {}
            loop {
                glib::timeout_future(Duration::from_millis(700)).await;
                match watch_rx.try_recv() {
                    Ok(()) => while watch_rx.try_recv().is_ok() {},
                    Err(async_channel::TryRecvError::Empty) => break,
                    Err(async_channel::TryRecvError::Closed) => return,
                }
            }
            if tx.send(WindowMessage::RescanRequested).await.is_err() {
                return;
            }
        }
    });
}

/// Forwards every value from `rx` into `tx`, transformed by `wrap`, until
/// either side closes. Used to bridge the player's plain-value channels
/// (`PlaybackState`, `Vec<Track>`) into the single `WindowMessage` stream.
fn spawn_forward<T: 'static>(
    rx: async_channel::Receiver<T>,
    tx: Sender<WindowMessage>,
    wrap: impl Fn(T) -> WindowMessage + 'static,
) {
    glib::spawn_future_local(async move {
        while let Ok(value) = rx.recv().await {
            if tx.send(wrap(value)).await.is_err() {
                break;
            }
        }
    });
}

fn build_sort_controls() -> (GtkBox, DropDown, ToggleButton) {
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
    (sort_controls, sort_field, sort_dir)
}

fn build_size_scale() -> Scale {
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
    size_scale
}

fn build_view_toggles() -> (ToggleButton, ToggleButton) {
    let list_toggle = ToggleButton::new();
    list_toggle.set_icon_name("view-list-symbolic");
    list_toggle.set_tooltip_text(Some("Track list"));
    list_toggle.set_active(true);
    let grid_toggle = ToggleButton::new();
    grid_toggle.set_icon_name("view-grid-symbolic");
    grid_toggle.set_tooltip_text(Some("Album grid"));
    grid_toggle.set_group(Some(&list_toggle));
    (list_toggle, grid_toggle)
}

fn build_scan_indicator() -> (Spinner, Label, GtkBox) {
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
    (scan_spinner, scan_status, scan_indicator)
}

fn build_left_sidebar_box(filter_sidebar: &Sidebar) -> GtkBox {
    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.set_width_request(220);
    sidebar.append(&filter_sidebar.widget);
    sidebar
}

fn build_right_sidebar_box(
    queue_view: &QueueView,
    folder_list: &ListBox,
    status_label: &Label,
) -> GtkBox {
    let folders_scrolled = ScrolledWindow::new();
    folders_scrolled.set_min_content_height(120);
    folders_scrolled.set_child(Some(folder_list));
    let folders_expander = Expander::new(Some("Watched Folders"));
    folders_expander.set_margin_start(4);
    folders_expander.set_child(Some(&folders_scrolled));

    // The queue expands to fill leftover height, which pins watched folders
    // and status to the bottom of the sidebar rather than right below it.
    queue_view.widget.set_vexpand(true);

    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.set_width_request(220);
    sidebar.set_vexpand(true);
    sidebar.append(&queue_view.widget);
    sidebar.append(&folders_expander);
    sidebar.append(status_label);
    sidebar
}

/// Links the left-sidebar toggle to `paned`'s divider and persists both the
/// open state and, while open, the dragged width — so the sidebar reopens
/// (this session or a future one) at the size it was last left at. A
/// `Revealer` here would leave the paned's fixed `position` allocating full
/// width to an empty child, so the divider itself is the collapse mechanism.
fn wire_left_sidebar_toggle(toggle: &ToggleButton, paned: &Paned, db: Rc<Db>) {
    let paned_for_toggle = paned.clone();
    let db_for_toggle = Rc::clone(&db);
    toggle.connect_toggled(move |btn| {
        let settings = Settings::new(&db_for_toggle);
        let open = btn.is_active();
        settings.set_left_sidebar_open(open);
        paned_for_toggle.set_position(if open {
            settings.left_sidebar_width()
        } else {
            0
        });
    });

    let toggle_for_drag = toggle.clone();
    paned.connect_position_notify(move |paned| {
        if toggle_for_drag.is_active() && paned.position() > 0 {
            Settings::new(&db).set_left_sidebar_width(paned.position());
        }
    });
}

/// Links the right-sidebar toggle to `paned`'s divider and persists both the
/// open state and, while open, the dragged width. The right sidebar is the
/// *end* child, so its width is `paned`'s total allocated width minus the
/// divider position — read at toggle/drag time, once the widget is realized
/// and that width is accurate (see `estimated_content_width` at construction
/// for the one case where it isn't yet).
fn wire_right_sidebar_toggle(toggle: &ToggleButton, paned: &Paned, db: Rc<Db>) {
    let paned_for_toggle = paned.clone();
    let db_for_toggle = Rc::clone(&db);
    toggle.connect_toggled(move |btn| {
        let settings = Settings::new(&db_for_toggle);
        let open = btn.is_active();
        settings.set_right_sidebar_open(open);
        let total_width = paned_for_toggle.width();
        paned_for_toggle.set_position(if open {
            (total_width - settings.right_sidebar_width()).max(0)
        } else {
            total_width
        });
    });

    let toggle_for_drag = toggle.clone();
    paned.connect_position_notify(move |paned| {
        if !toggle_for_drag.is_active() {
            return;
        }
        let width = paned.width() - paned.position();
        if width > 0 {
            Settings::new(&db).set_right_sidebar_width(width);
        }
    });
}

pub fn build(
    app: &Application,
    db: Rc<Db>,
    db_path: PathBuf,
    player: PlayerHandle,
    state_rx: async_channel::Receiver<PlaybackState>,
    queue_rx: async_channel::Receiver<Vec<Track>>,
) -> ApplicationWindow {
    let initial = InitialSettings::read(&db);
    let (tx, rx) = async_channel::unbounded::<WindowMessage>();
    let (mut ctx, watch_rx) = Context::new(app, db, db_path, player, &initial, tx.clone());
    let window = ctx.window.clone();

    spawn_forward(state_rx, tx.clone(), WindowMessage::PlayerStateChanged);
    spawn_forward(queue_rx, tx.clone(), WindowMessage::PlayerQueueChanged);
    wire_debounced_watcher(watch_rx, tx);

    let mut state = WindowState {
        filter: LibraryFilter::All,
        sort: initial.sort,
        queue: Vec::new(),
    };
    let mut watcher: Option<FolderWatcher> = None;

    // First population, plus restoring the previous session's queue — direct
    // calls, not dispatched `WindowMessage`s, since this runs once before the dispatch
    // loop below takes ownership of `state`.
    ctx.refresh_folder_list();
    ctx.refresh_sidebar();
    ctx.refresh_album_grid_with(&state.filter, state.sort);
    ctx.rewatch(&mut watcher);
    if let Ok(tracks) = ctx.db.tracks_for(&LibraryFilter::All) {
        ctx.library_view.set_tracks(tracks);
    }
    ctx.restore_queue(&initial, &mut state);

    glib::spawn_future_local(async move {
        while let Ok(msg) = rx.recv().await {
            state = reduce(state, &msg);
            ctx.apply(&state, &msg, &mut watcher);
        }
    });

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
