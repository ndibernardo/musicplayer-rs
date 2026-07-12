use std::cell::Cell;
use std::cell::OnceCell;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::Align;
use gtk4::Box as GtkBox;
use gtk4::GestureClick;
use gtk4::Image;
use gtk4::Label;
use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::ScrolledWindow;
use gtk4::SelectionMode;
use gtk4::Widget;
use gtk4::gdk::MemoryFormat;
use gtk4::gdk::MemoryTexture;
use gtk4::gdk::ModifierType;
use gtk4::gdk::Texture;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::gio::Cancellable;
use gtk4::gio::MemoryInputStream;
use gtk4::prelude::*;

use crate::library::album::AlbumSummary;
use crate::library::album::ArtKey;
use crate::library::album::CoverArt;
use crate::library::track::AlbumArtData;
use crate::library::track::Track;
use crate::ui::context_menu;
use crate::ui::context_menu::show_context_menu;
use crate::ui::format;
use crate::ui::format::format_duration;
use crate::ui::style;
use crate::ui::style::StyleClass;
use crate::ui::widgets::AppIcon;
use crate::ui::widgets::Callback;
use crate::ui::widgets::body_label;
use crate::ui::widgets::caption_label;
use crate::ui::widgets::flat_icon_button;
use crate::ui::widgets::numeric_dim_label;
use crate::ui::widgets::remove_all_children;

/// Column count before the first width-driven reflow. The grid is hand-built
/// (not a `GridView`) so a full-width track drawer can be inserted between rows;
/// columns are recomputed from the viewport width (see `reflow`).
const DEFAULT_COLUMNS: usize = 4;

/// Per-cover horizontal budget beyond the cover itself (cell padding + inter-
/// cover spacing) — used to estimate how many covers fit a given width.
const COVER_SLOT_EXTRA: i32 = 34;

/// Per-row vertical budget beyond the cover itself (album/artist/year labels
/// plus inter-row spacing) — used only to estimate how many rows the
/// viewport can show, for `AlbumGrid::visible_rows`. An overestimate just
/// means a handful of extra cells build eagerly instead of on the next idle
/// tick; nothing here depends on it being exact.
const ROW_SLOT_EXTRA: i32 = 76;

/// Combined left+right margin of the grid container (px).
const GRID_MARGIN_TOTAL: i32 = 12;

/// Default cover side length (px), used until a saved or user-chosen value is
/// applied via `set_cover_size`.
const DEFAULT_COVER_SIZE: i32 = 200;

/// Side length (px) covers are decoded and downscaled to, regardless of the
/// display size chosen via `set_cover_size`. One fixed bucket means the decode
/// happens once per album no matter how the user resizes the cover slider
/// afterward — `set_cover_size` only calls `Image::set_pixel_size`, which
/// scales the already-decoded texture at render time.
const COVER_DECODE_SIZE: i32 = crate::library::settings::COVER_SIZE_MAX;

type TrackProvider = Rc<dyn Fn(&AlbumSummary) -> Vec<Track>>;
/// A registered handler for "the album's full track list plus the index of
/// the activated track", as handed out by `Callback::handler`.
type TrackHandler = Rc<dyn Fn((Vec<Track>, usize))>;
/// A registered handler for an album's full track list.
type AlbumHandler = Rc<dyn Fn(Vec<Track>)>;
/// Fetches an album's cover art bytes, called only on a texture-cache miss.
type ArtProvider = Rc<dyn Fn(&ArtKey) -> Option<AlbumArtData>>;

/// The inline track drawer: closed, or open beneath exactly one album. A half-
/// open drawer (an index with no widget, or vice versa) is unrepresentable.
/// `list` is the drawer's own track `ListBox`, kept alongside so
/// `clear_selection` can drop any row selection in it without a widget-tree
/// search.
enum DrawerState {
    Closed,
    Open {
        index: usize,
        widget: Widget,
        list: ListBox,
    },
}

/// One album's grid presence: its summary and the widgets rendering it.
struct AlbumCell {
    summary: AlbumSummary,
    cell: GtkBox,
    cover: Image,
}

/// The grid's mutable state, behind one `RefCell` so no invariant here can be
/// observed half-updated between two separate borrows.
struct GridState {
    cells: Vec<AlbumCell>,
    // Rows of cells, in album order; rebuilt by `lay_out` whenever the column
    // count changes.
    row_boxes: Vec<GtkBox>,
    drawer: DrawerState,
    // Cover indices multi-selected via ctrl/shift-click, independent of
    // `drawer` — any multi-select action closes an open drawer first, so the
    // two never mark the same cover at once (see `close_drawer`).
    multi_selected: BTreeSet<usize>,
    // Last definite click point, for shift-range-select; reset only when
    // `set_albums` invalidates every index.
    selection_anchor: Option<usize>,
    columns: usize,
    cover_size: i32,
    // Decoded, downscaled textures keyed by ArtKey. A cache hit skips both the
    // DB byte fetch and the JPEG/PNG decode on repeat set_albums calls (sort
    // change, filter change, reflow).
    texture_cache: HashMap<ArtKey, Texture>,
    // Images awaiting a decode result for a given key, so several cells (a grid
    // cell and, if opened before decoding finishes, its drawer cover) requesting
    // the same album's art all get updated from one decode.
    pending_covers: HashMap<ArtKey, Vec<Image>>,
}

/// Album-art browser: a grid of cover cells. Activating a cover opens a
/// full-width drawer with that album's full track list, inline on its own row
/// directly beneath the cover's row. Activating it again closes the drawer.
#[derive(Clone)]
pub struct AlbumGrid {
    pub widget: ScrolledWindow,
    inner: Rc<AlbumGridInner>,
}

struct AlbumGridInner {
    grid_box: GtkBox,
    state: RefCell<GridState>,
    // Bumped by every `set_albums` call. The chunked build it starts checks
    // this each idle tick and stops as soon as it no longer matches, so a
    // rapid filter/sort change doesn't leave a stale build still appending
    // rows behind the current one's back.
    build_generation: Cell<u64>,
    // Sends (key, bytes) to the background decode thread; see `spawn_art_decoder`.
    art_request_tx: async_channel::Sender<(ArtKey, AlbumArtData)>,
    art_provider: OnceCell<ArtProvider>,
    track_provider: OnceCell<TrackProvider>,
    on_track_activated: Callback<(Vec<Track>, usize)>,
    on_album_activated: Callback<Vec<Track>>,
    on_album_enqueue: Callback<Vec<Track>>,
    on_album_edit: Callback<Vec<Track>>,
    // A drawer row's enqueue/edit action — the selected tracks, one element
    // for a singular click, the whole multi-selection for a batch one. See
    // `context_menu::track_actions`.
    on_track_enqueue: Callback<Vec<Track>>,
    on_track_edit: Callback<Vec<Track>>,
}

impl AlbumGrid {
    pub fn new() -> Self {
        let grid_box = GtkBox::new(Orientation::Vertical, 16);
        grid_box.set_margin_start(6);
        grid_box.set_margin_end(6);
        grid_box.set_margin_top(6);
        grid_box.set_margin_bottom(6);

        let scrolled = ScrolledWindow::new();
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&grid_box));

        let (art_request_tx, art_result_rx) = spawn_art_decoder(COVER_DECODE_SIZE);

        let inner = Rc::new(AlbumGridInner {
            grid_box,
            build_generation: Cell::new(0),
            state: RefCell::new(GridState {
                cells: Vec::new(),
                row_boxes: Vec::new(),
                drawer: DrawerState::Closed,
                multi_selected: BTreeSet::new(),
                selection_anchor: None,
                columns: DEFAULT_COLUMNS,
                cover_size: DEFAULT_COVER_SIZE,
                texture_cache: HashMap::new(),
                pending_covers: HashMap::new(),
            }),
            art_request_tx,
            art_provider: OnceCell::new(),
            track_provider: OnceCell::new(),
            on_track_activated: Callback::new(),
            on_album_activated: Callback::new(),
            on_album_enqueue: Callback::new(),
            on_album_edit: Callback::new(),
            on_track_enqueue: Callback::new(),
            on_track_edit: Callback::new(),
        });

        let grid = Self {
            widget: scrolled,
            inner,
        };
        grid.install_reflow_handler();
        grid.install_art_decode_receiver(art_result_rx);
        grid
    }

    /// Consumes decoded textures from the background decoder and hands each one
    /// to every cell that was waiting on it — a grid cell, and its drawer cover
    /// if the drawer was opened before the decode finished.
    fn install_art_decode_receiver(&self, rx: async_channel::Receiver<(ArtKey, Texture)>) {
        let inner = Rc::clone(&self.inner);
        glib::spawn_future_local(async move {
            while let Ok((key, texture)) = rx.recv().await {
                let waiting = {
                    let mut state = inner.state.borrow_mut();
                    let waiting = state.pending_covers.remove(&key);
                    state.texture_cache.insert(key, texture.clone());
                    waiting
                };
                for image in waiting.into_iter().flatten() {
                    image.set_paintable(Some(&texture));
                }
            }
        });
    }

    /// Reflows whenever the scroll viewport's width changes.
    fn install_reflow_handler(&self) {
        let this = self.clone();
        self.widget
            .hadjustment()
            .connect_page_size_notify(move |_| this.reflow());
    }

    /// Rebuilds the cover grid and closes any open drawer. Enough rows to
    /// fill the visible viewport are built synchronously, immediately, so
    /// there's something on screen the instant this returns; the rest build
    /// off the critical path — see `build_cells_in_chunks`.
    pub fn set_albums(&self, albums: Vec<AlbumSummary>) {
        remove_all_children(&self.inner.grid_box);
        let columns = self.columns_for_current_width();
        {
            let mut state = self.inner.state.borrow_mut();
            state.drawer = DrawerState::Closed;
            state.multi_selected.clear();
            state.selection_anchor = None;
            state.cells = Vec::new();
            state.row_boxes = Vec::new();
            state.columns = columns;
        }

        let generation = self.inner.build_generation.get().wrapping_add(1);
        self.inner.build_generation.set(generation);

        let mut albums = albums.into_iter().enumerate();
        let eager_count = (self.visible_rows() * columns).min(albums.len());
        let mut eager: Vec<AlbumCell> = Vec::with_capacity(eager_count);
        for (index, summary) in albums.by_ref().take(eager_count) {
            eager.push(self.build_cell(index, summary));
            if eager.len() == columns {
                self.flush_pending_row(&mut eager);
            }
        }
        self.flush_pending_row(&mut eager);

        self.build_cells_in_chunks(albums, generation);
    }

    /// Rows tall enough to cover the current viewport, at least one — used to
    /// size the eager, synchronous part of `set_albums`. Falls back to a
    /// small fixed guess when the widget hasn't been allocated a real height
    /// yet (its first-ever population, right after the window presents,
    /// which is also where GTK pays its one-time font/CSS/type-system
    /// cold-start cost — exactly when this matters most).
    fn visible_rows(&self) -> usize {
        let page_size = self.widget.vadjustment().page_size() as i32;
        if page_size <= 0 {
            return 3;
        }
        let cover_size = self.inner.state.borrow().cover_size;
        let row_height = cover_size + ROW_SLOT_EXTRA;
        ((page_size / row_height) as usize).max(1) + 1
    }

    /// Builds `AlbumCell`s a chunk at a time on the GTK idle queue instead of
    /// all at once. `build_cell` allocates an `Image`, up to three `Label`s,
    /// and two `GestureClick` controllers per album — for a library with
    /// thousands of albums, doing that in one synchronous loop is what used
    /// to freeze the switch to Grid view (and, before the startup fix, the
    /// whole window). `generation` is re-checked every tick: a later
    /// `set_albums` call (a filter or sort change while this one is still
    /// running) bumps it, which makes this one stop rather than race it.
    fn build_cells_in_chunks(
        &self,
        mut albums: impl Iterator<Item = (usize, AlbumSummary)> + 'static,
        generation: u64,
    ) {
        const CHUNK: usize = 40;

        let grid = self.clone();
        let mut pending: Vec<AlbumCell> = Vec::new();
        glib::idle_add_local(move || {
            if grid.inner.build_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let columns = grid.inner.state.borrow().columns.max(1);
            for _ in 0..CHUNK {
                match albums.next() {
                    Some((index, summary)) => {
                        pending.push(grid.build_cell(index, summary));
                        if pending.len() == columns {
                            grid.flush_pending_row(&mut pending);
                        }
                    }
                    None => {
                        grid.flush_pending_row(&mut pending);
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    /// Parents `pending`'s cells into one new row and moves them into
    /// `state`, leaving `pending` empty — a no-op if it already is (the
    /// end-of-build call when the last row divided evenly). Cells only ever
    /// enter `state.cells` already parented into a row, so a `reflow` that
    /// runs mid-build (the window resized while a big library is still
    /// loading) only ever re-lays out cells that are actually placed, never
    /// one `build_cells_in_chunks` is still holding onto for the row in
    /// progress.
    fn flush_pending_row(&self, pending: &mut Vec<AlbumCell>) {
        if pending.is_empty() {
            return;
        }
        let widgets: Vec<GtkBox> = pending.iter().map(|c| c.cell.clone()).collect();
        let row_box = self.append_row(&widgets);
        let mut state = self.inner.state.borrow_mut();
        state.cells.append(pending);
        state.row_boxes.push(row_box);
    }

    /// Appends one row of `cells` to `grid_box` and returns its container,
    /// left-aligned so even a partial row starts at the same edge as every
    /// other row instead of floating to the middle.
    fn append_row(&self, cells: &[GtkBox]) -> GtkBox {
        let row_box = GtkBox::new(Orientation::Horizontal, 16);
        row_box.set_halign(Align::Start);
        for cell in cells {
            row_box.append(cell);
        }
        self.inner.grid_box.append(&row_box);
        row_box
    }

    /// Arranges the existing cover cells into rows of `columns`. Used by
    /// `reflow`, where every cell already exists and just needs re-grouping.
    fn lay_out(&self, columns: usize) {
        let cell_widgets: Vec<GtkBox> = {
            let state = self.inner.state.borrow();
            state.cells.iter().map(|c| c.cell.clone()).collect()
        };
        let row_boxes: Vec<GtkBox> = cell_widgets
            .chunks(columns)
            .map(|chunk| self.append_row(chunk))
            .collect();
        let mut state = self.inner.state.borrow_mut();
        state.columns = columns;
        state.row_boxes = row_boxes;
    }

    /// Re-lays the covers when the column count changes (window resize or a new
    /// cover size). Reuses the built cells — no texture re-decode.
    fn reflow(&self) {
        let columns = self.columns_for_current_width();
        if columns == self.inner.state.borrow().columns {
            return;
        }

        self.close_drawer();

        let cell_widgets: Vec<GtkBox> = {
            let state = self.inner.state.borrow();
            state.cells.iter().map(|c| c.cell.clone()).collect()
        };
        for cell in &cell_widgets {
            cell.unparent();
        }
        remove_all_children(&self.inner.grid_box);
        self.lay_out(columns);
    }

    /// How many covers fit the current viewport width (at least one).
    fn columns_for_current_width(&self) -> usize {
        let available = self.widget.hadjustment().page_size() as i32 - GRID_MARGIN_TOTAL;
        let cover_size = self.inner.state.borrow().cover_size;
        let slot = cover_size + COVER_SLOT_EXTRA;
        (available / slot).max(1) as usize
    }

    /// Supplies the tracks of an album, fetched when its drawer opens.
    pub fn set_track_provider<F: Fn(&AlbumSummary) -> Vec<Track> + 'static>(&self, f: F) {
        let _ = self.inner.track_provider.set(Rc::new(f));
    }

    /// Supplies an album's cover art bytes, called only on a texture-cache miss
    /// (once per album, ever, unless `invalidate_art_cache` runs).
    pub fn set_art_provider<F: Fn(&ArtKey) -> Option<AlbumArtData> + 'static>(&self, f: F) {
        let _ = self.inner.art_provider.set(Rc::new(f));
    }

    /// Clears every decoded texture, so the next display of each cover re-fetches
    /// and re-decodes its bytes. Call after a scan may have changed an album's
    /// embedded art — the cache is keyed only by `ArtKey`, so it would otherwise
    /// keep showing a stale cover until restart.
    pub fn invalidate_art_cache(&self) {
        self.inner.state.borrow_mut().texture_cache.clear();
    }

    /// Clears only `keys`' decoded textures, leaving every other album's cover
    /// cached. Call after an edit that may have changed one album's art —
    /// unlike `invalidate_art_cache`, this doesn't blank every cover in the
    /// grid while they all re-decode, just the one(s) that actually changed.
    pub fn invalidate_art_for(&self, keys: &[ArtKey]) {
        let mut state = self.inner.state.borrow_mut();
        for key in keys {
            state.texture_cache.remove(key);
        }
    }

    /// Registers `image` to receive `key`'s texture once decoded, fetching the
    /// art bytes and dispatching the decode on the first request for `key`.
    /// A second cell asking for the same album (e.g. its drawer, opened before
    /// the first decode finishes) just joins the waiting list.
    fn request_art(&self, key: ArtKey, image: Image) {
        let first_request = {
            let mut state = self.inner.state.borrow_mut();
            let waiting = state.pending_covers.entry(key.clone()).or_default();
            waiting.push(image);
            waiting.len() == 1
        };
        if !first_request {
            return;
        }
        let Some(art) = self.inner.art_provider.get().and_then(|f| f(&key)) else {
            self.inner.state.borrow_mut().pending_covers.remove(&key);
            return;
        };
        // The decoder thread is unbounded and never closes its receiver while
        // `self` is alive, so a send failure can't happen in practice; dropping
        // the request would just leave that cover blank, which is recoverable.
        let _ = self.inner.art_request_tx.try_send((key, art));
    }

    /// Registers the callback invoked when a track inside a drawer is activated.
    /// It receives the album's full track list and the activated track's index.
    pub fn connect_track_activated<F: Fn(Vec<Track>, usize) + 'static>(&self, f: F) {
        self.inner
            .on_track_activated
            .set(move |(tracks, index)| f(tracks, index));
    }

    /// Registers the callback invoked with an album's tracks when its cover is
    /// opened, so the caller can enqueue the whole album.
    pub fn connect_album_activated<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        self.inner.on_album_activated.set(f);
    }

    /// Registers the callback invoked with an album's tracks when "Add to
    /// Queue" is chosen from a cover's right-click menu.
    pub fn connect_album_enqueue<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        self.inner.on_album_enqueue.set(f);
    }

    /// Registers the callback invoked when "Add to Queue" (or, for a
    /// multi-selected row, "Add N to Queue") is chosen from a drawer row's
    /// right-click menu — the selected tracks, one element for the singular
    /// case.
    pub fn connect_track_enqueue<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        self.inner.on_track_enqueue.set(f);
    }

    /// Registers the callback invoked with an album's tracks when "Edit
    /// Album…" is chosen from a cover's right-click menu.
    pub fn connect_album_edit_requested<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        self.inner.on_album_edit.set(f);
    }

    /// Registers the callback invoked when "Edit Track…" (or, for a
    /// multi-selected row, "Edit N Tracks…") is chosen from a drawer row's
    /// right-click menu — the selected tracks, one element for the singular
    /// case.
    pub fn connect_track_edit_requested<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        self.inner.on_track_edit.set(f);
    }

    /// Resizes every cover in place to `size` px, then reflows since a different
    /// cover size changes how many fit per row. No texture re-decode.
    pub fn set_cover_size(&self, size: i32) {
        let cell_widgets: Vec<(Image, GtkBox)> = {
            let mut state = self.inner.state.borrow_mut();
            state.cover_size = size;
            state
                .cells
                .iter()
                .map(|c| (c.cover.clone(), c.cell.clone()))
                .collect()
        };
        for (image, cell) in &cell_widgets {
            image.set_pixel_size(size);
            cell.set_width_request(size);
        }
        self.reflow();
    }

    fn build_cell(&self, index: usize, summary: AlbumSummary) -> AlbumCell {
        let size = self.inner.state.borrow().cover_size;

        // GtkImage renders at a fixed pixel size; GtkPicture instead grows to the
        // texture's natural size (the covers-too-big bug), since size_request only
        // sets a minimum.
        let image = Image::new();
        image.set_pixel_size(size);
        image.set_halign(Align::Center);

        if let CoverArt::Available(key) = &summary.art {
            let cached = self.inner.state.borrow().texture_cache.get(key).cloned();
            match cached {
                Some(texture) => image.set_paintable(Some(&texture)),
                None => self.request_art(key.clone(), image.clone()),
            }
        }

        let album_label = body_label(summary.album.as_str());
        album_label.set_max_width_chars(16);
        album_label.add_css_class("heading");

        // Full opacity (no dim-label) so the artist reads clearly under the album.
        let artist_label = body_label(summary.artist.as_str());
        artist_label.set_max_width_chars(16);

        let cell = GtkBox::new(Orientation::Vertical, 4);
        cell.add_css_class("activatable");
        // Padding is reserved unconditionally so toggling `AlbumSelected` only
        // repaints the background — it never resizes the cell and shifts the grid.
        style::add_class(&cell, StyleClass::AlbumCell);
        // Pin the cell to the cover width so labels ellipsize instead of widening it.
        cell.set_width_request(size);
        cell.set_halign(Align::Start);
        cell.append(&image);
        cell.append(&album_label);
        cell.append(&artist_label);
        if !summary.year.is_unknown() {
            cell.append(&caption_label(&summary.year.value().to_string()));
        }

        // Plain click opens the track drawer without playing (the album is
        // played from the ▶ button in that drawer's header); ctrl/shift-click
        // instead manage a multi-selection, closing any open drawer first.
        // Only the first press acts, so a double-click doesn't flip a drawer
        // shut again or re-toggle a selection.
        let gesture = GestureClick::new();
        let this = self.clone();
        gesture.connect_pressed(move |gesture, n_press, _, _| {
            if n_press != 1 {
                return;
            }
            let mods = gesture.current_event_state();
            if mods.contains(ModifierType::SHIFT_MASK) {
                this.shift_select(index);
            } else if mods.contains(ModifierType::CONTROL_MASK) {
                this.ctrl_toggle(index);
            } else {
                this.clear_multi_selection(index);
                this.toggle_drawer(index);
            }
        });
        cell.add_controller(gesture);

        // Right-click offers "Add to Queue"/"Edit Album…" for the whole
        // album, fetched the same way the drawer does, without opening or
        // closing it — or, if this cover is part of a wider multi-selection,
        // the same two actions for every selected album's concatenated
        // tracks (reusing the same callbacks; a multi-album batch is just a
        // bigger `Vec<Track>`).
        let context_gesture = GestureClick::new();
        context_gesture.set_button(gtk4::gdk::BUTTON_SECONDARY);
        let this = self.clone();
        let context_summary = summary.clone();
        let cell_widget = cell.clone().upcast::<Widget>();
        context_gesture.connect_pressed(move |_, _, x, y| {
            let (tracks, album_count) = {
                let state = this.inner.state.borrow();
                if state.multi_selected.contains(&index) && state.multi_selected.len() > 1 {
                    let summaries: Vec<AlbumSummary> = state
                        .multi_selected
                        .iter()
                        .filter_map(|&i| state.cells.get(i).map(|c| c.summary.clone()))
                        .collect();
                    let n = summaries.len();
                    let tracks: Vec<Track> = summaries
                        .iter()
                        .flat_map(|s| {
                            this.inner
                                .track_provider
                                .get()
                                .map_or_else(Vec::new, |provide| provide(s))
                        })
                        .collect();
                    (tracks, Some(n))
                } else {
                    let tracks = this
                        .inner
                        .track_provider
                        .get()
                        .map_or_else(Vec::new, |provide| provide(&context_summary));
                    (tracks, None)
                }
            };
            if tracks.is_empty() {
                return;
            }
            let on_enqueue = this.inner.on_album_enqueue.handler();
            let enqueue_tracks = tracks.clone();
            let on_edit = this.inner.on_album_edit.handler();
            let (enqueue_label, edit_label) = match album_count {
                Some(n) => (
                    format!("Add {n} Albums to Queue"),
                    format!("Edit {n} Albums…"),
                ),
                None => ("Add to Queue".to_string(), "Edit Album…".to_string()),
            };
            show_context_menu(
                &cell_widget,
                x,
                y,
                vec![
                    (
                        enqueue_label,
                        Box::new(move || {
                            if let Some(callback) = &on_enqueue {
                                callback(enqueue_tracks.clone());
                            }
                        }) as Box<dyn Fn()>,
                    ),
                    (
                        edit_label,
                        Box::new(move || {
                            if let Some(callback) = &on_edit {
                                callback(tracks.clone());
                            }
                        }) as Box<dyn Fn()>,
                    ),
                ],
            );
        });
        cell.add_controller(context_gesture);

        AlbumCell {
            summary,
            cell,
            cover: image,
        }
    }

    /// Highlights the cover at `index` as selected and clears the others.
    fn highlight(&self, index: Option<usize>) {
        let cell_widgets: Vec<GtkBox> = {
            let state = self.inner.state.borrow();
            state.cells.iter().map(|c| c.cell.clone()).collect()
        };
        for (i, cell) in cell_widgets.iter().enumerate() {
            if Some(i) == index {
                style::add_class(cell, StyleClass::AlbumSelected);
            } else {
                style::remove_class(cell, StyleClass::AlbumSelected);
            }
        }
    }

    /// Closes any open drawer, removing its widget and clearing its
    /// highlight. Returns the index that was open, if any. Shared by
    /// `toggle_drawer` and every multi-select mutation — the two states
    /// never overlap, since selecting starts by closing whatever is open.
    fn close_drawer(&self) -> Option<usize> {
        let previously_open = {
            let mut state = self.inner.state.borrow_mut();
            std::mem::replace(&mut state.drawer, DrawerState::Closed)
        };
        match previously_open {
            DrawerState::Closed => None,
            DrawerState::Open { index, widget, .. } => {
                self.inner.grid_box.remove(&widget);
                self.highlight(None);
                Some(index)
            }
        }
    }

    /// Toggles `index` in the multi-selection, closing any open drawer first.
    fn ctrl_toggle(&self, index: usize) {
        self.close_drawer();
        {
            let mut state = self.inner.state.borrow_mut();
            if !state.multi_selected.remove(&index) {
                state.multi_selected.insert(index);
            }
            state.selection_anchor = Some(index);
        }
        self.apply_multi_highlight();
    }

    /// Replaces the multi-selection with the inclusive range between the last
    /// anchor and `index` (or starts a fresh one-cover selection if there is
    /// no anchor yet). The anchor itself never moves, so repeated shift-clicks
    /// re-range from the same point — standard behavior.
    fn shift_select(&self, index: usize) {
        self.close_drawer();
        {
            let mut state = self.inner.state.borrow_mut();
            let anchor = *state.selection_anchor.get_or_insert(index);
            let (start, end) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            state.multi_selected = (start..=end).collect();
        }
        self.apply_multi_highlight();
    }

    /// Clears the multi-selection, making `index` the new anchor for a future
    /// shift-click. A plain click always means "just let me look at this one
    /// album," so it leaves no multi-selection behind.
    fn clear_multi_selection(&self, index: usize) {
        {
            let mut state = self.inner.state.borrow_mut();
            state.multi_selected.clear();
            state.selection_anchor = Some(index);
        }
        self.apply_multi_highlight();
    }

    /// Marks every multi-selected cover with `album-multi-selected`,
    /// independent of `highlight`'s `album-selected` (which marks "this
    /// cover's drawer is open" — the two classes never apply to the same
    /// cover, since any multi-select action closes the open drawer first).
    fn apply_multi_highlight(&self) {
        let (cell_widgets, selected) = {
            let state = self.inner.state.borrow();
            let widgets: Vec<GtkBox> = state.cells.iter().map(|c| c.cell.clone()).collect();
            (widgets, state.multi_selected.clone())
        };
        for (i, cell) in cell_widgets.iter().enumerate() {
            if selected.contains(&i) {
                style::add_class(cell, StyleClass::AlbumMultiSelected);
            } else {
                style::remove_class(cell, StyleClass::AlbumMultiSelected);
            }
        }
    }

    /// Clears any multi-selected covers and any selected rows in an open
    /// drawer's tracklist, with no new anchor (unlike `clear_multi_selection`,
    /// there is no clicked album here — the user clicked somewhere else in
    /// the application entirely). Leaves the drawer itself open.
    pub fn clear_selection(&self) {
        let drawer_list = {
            let mut state = self.inner.state.borrow_mut();
            state.multi_selected.clear();
            match &state.drawer {
                DrawerState::Open { list, .. } => Some(list.clone()),
                DrawerState::Closed => None,
            }
        };
        self.apply_multi_highlight();
        if let Some(list) = drawer_list {
            list.unselect_all();
        }
    }

    /// Opens the drawer for `index` beneath its row, or closes it if already open.
    fn toggle_drawer(&self, index: usize) {
        let previously_open_index = self.close_drawer();
        if previously_open_index == Some(index) {
            return;
        }

        let Some(summary) = self
            .inner
            .state
            .borrow()
            .cells
            .get(index)
            .map(|c| c.summary.clone())
        else {
            return;
        };
        let tracks = self
            .inner
            .track_provider
            .get()
            .map_or_else(Vec::new, |provide| provide(&summary));
        let art_key = match &summary.art {
            CoverArt::Available(key) => Some(key.clone()),
            CoverArt::Absent => None,
        };
        let cached = art_key
            .as_ref()
            .and_then(|key| self.inner.state.borrow().texture_cache.get(key).cloned());
        let callbacks = DrawerCallbacks {
            on_track: self.inner.on_track_activated.handler(),
            on_album: self.inner.on_album_activated.handler(),
            on_enqueue: self.inner.on_track_enqueue.handler(),
            on_edit: self.inner.on_track_edit.handler(),
        };
        let (drawer_widget, cover, drawer_list) =
            build_drawer(&summary, tracks, callbacks, cached.as_ref());
        if cached.is_none()
            && let Some(key) = art_key
        {
            self.request_art(key, cover);
        }

        let columns = self.inner.state.borrow().columns;
        let row_box = self
            .inner
            .state
            .borrow()
            .row_boxes
            .get(index / columns)
            .cloned();
        let Some(row_box) = row_box else { return };
        self.inner
            .grid_box
            .insert_child_after(&drawer_widget, Some(&row_box));
        self.inner.state.borrow_mut().drawer = DrawerState::Open {
            index,
            widget: drawer_widget.clone().upcast(),
            list: drawer_list,
        };
        self.highlight(Some(index));
    }
}

/// Combined cover-art size shown at the left of a drawer (px).
const DRAWER_COVER_SIZE: i32 = 160;

/// The callbacks a drawer's rows and header can dispatch to, grouped since
/// `build_drawer` would otherwise take too many parameters.
struct DrawerCallbacks {
    on_track: Option<TrackHandler>,
    on_album: Option<AlbumHandler>,
    // A row's enqueue/edit action — the selected tracks, one element for a
    // singular click, the whole multi-selection for a batch one. See
    // `context_menu::track_actions`.
    on_enqueue: Option<AlbumHandler>,
    on_edit: Option<AlbumHandler>,
}

/// Builds the inline drawer: the album cover on the left, and on the right a
/// header (play button plus album heading) above the full track list. Returns
/// the drawer widget, its cover `Image` (so the caller can register it for an
/// async texture update on a cache miss), and its track `ListBox` (so the
/// caller can drop any row selection in it from `clear_selection`).
fn build_drawer(
    summary: &AlbumSummary,
    tracks: Vec<Track>,
    callbacks: DrawerCallbacks,
    cached: Option<&Texture>,
) -> (GtkBox, Image, ListBox) {
    let DrawerCallbacks {
        on_track,
        on_album,
        on_enqueue,
        on_edit,
    } = callbacks;
    let container = GtkBox::new(Orientation::Vertical, 0);
    container.set_hexpand(true);

    let play_btn = flat_icon_button(AppIcon::MediaPlaybackStart, "Play album");
    play_btn.set_valign(Align::Center);
    if let Some(callback) = on_album {
        let album_tracks = tracks.clone();
        play_btn.connect_clicked(move |_| callback(album_tracks.clone()));
    }

    let heading = Label::new(Some(&format::drawer_heading(summary)));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");

    let header = GtkBox::new(Orientation::Horizontal, 4);
    header.set_margin_start(8);
    header.set_margin_top(6);
    header.set_margin_bottom(4);
    header.append(&play_btn);
    header.append(&heading);
    container.append(&header);

    let list = ListBox::new();
    // Multiple gives ctrl/shift-click natively; `row-activated` (double-click
    // to play, wired below) is unaffected — selection and activation are
    // independent concepts in `ListBox`.
    list.set_selection_mode(SelectionMode::Multiple);
    list.set_activate_on_single_click(false);
    for (row_index, track) in tracks.iter().enumerate() {
        let row = track_row(track);
        let wants_menu = on_enqueue.is_some() || on_edit.is_some();
        if wants_menu {
            let context_gesture = GestureClick::new();
            context_gesture.set_button(gtk4::gdk::BUTTON_SECONDARY);
            let track = track.clone();
            let row_for_select = row.clone();
            let row_widget = row.clone().upcast::<Widget>();
            let list_for_gesture = list.clone();
            let all_tracks = tracks.clone();
            let on_enqueue = on_enqueue.clone();
            let on_edit = on_edit.clone();
            context_gesture.connect_pressed(move |_, _, x, y| {
                let selected: Vec<usize> = list_for_gesture
                    .selected_rows()
                    .iter()
                    .map(|r| r.index() as usize)
                    .collect();
                let batch = if selected.contains(&row_index) && selected.len() > 1 {
                    let mut ordered = selected.clone();
                    ordered.sort_unstable();
                    Some(
                        ordered
                            .iter()
                            .filter_map(|&i| all_tracks.get(i).cloned())
                            .collect(),
                    )
                } else {
                    // Right-clicking outside the current selection collapses
                    // it to just this row — standard file-manager convention.
                    list_for_gesture.select_row(Some(&row_for_select));
                    None
                };
                let actions =
                    context_menu::track_actions(&track, batch, on_enqueue.clone(), on_edit.clone());
                show_context_menu(&row_widget, x, y, actions);
            });
            row.add_controller(context_gesture);
        }
        list.append(&row);
    }
    if let Some(callback) = on_track {
        list.connect_row_activated(move |_, row| {
            let index = row.index() as usize;
            if index < tracks.len() {
                callback((tracks.clone(), index));
            }
        });
    }
    container.append(&list);

    // The album cover, mirrored from the grid, sits to the left of the list.
    let cover = Image::new();
    cover.set_pixel_size(DRAWER_COVER_SIZE);
    cover.set_valign(Align::Start);
    cover.set_margin_start(8);
    cover.set_margin_top(8);
    cover.set_margin_bottom(8);
    // The pre-decoded cached texture, if the grid cell already resolved one;
    // otherwise the caller registers `cover` to receive it once decoded.
    if let Some(texture) = cached {
        cover.set_paintable(Some(texture));
    }

    let outer = GtkBox::new(Orientation::Horizontal, 8);
    // Same dark tint as the selected cover, so the open album and its track list
    // read as one continuous selection.
    style::add_class(&outer, StyleClass::AlbumDrawer);
    outer.set_margin_top(2);
    outer.set_margin_bottom(2);
    outer.append(&cover);
    outer.append(&container);
    (outer, cover, list)
}

fn track_row(track: &Track) -> ListBoxRow {
    let number = numeric_dim_label(&format::track_number(track));
    number.set_width_chars(5);

    let title = body_label(track.title.as_str());
    title.set_hexpand(true);

    let duration = numeric_dim_label(&format_duration(track.duration));

    let row_box = GtkBox::new(Orientation::Horizontal, 8);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);
    row_box.set_margin_top(2);
    row_box.set_margin_bottom(2);
    row_box.append(&number);
    row_box.append(&title);
    row_box.append(&duration);

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}

/// Channel endpoints returned by `spawn_art_decoder`: the request sender and
/// the matching result receiver.
type ArtDecoder = (
    async_channel::Sender<(ArtKey, AlbumArtData)>,
    async_channel::Receiver<(ArtKey, Texture)>,
);

/// Spawns the background cover decoder. It receives `(key, bytes)` requests and
/// sends back `(key, texture)` once decoded and downscaled to at most
/// `max_size` px on each side. Runs on its own thread so a 1500x1500 JPEG never
/// blocks the main thread — `gdk4::Texture` is `Send`, and GDK4's object
/// construction has no main-thread requirement (unlike GDK3's).
fn spawn_art_decoder(max_size: i32) -> ArtDecoder {
    let (request_tx, request_rx) = async_channel::unbounded::<(ArtKey, AlbumArtData)>();
    let (result_tx, result_rx) = async_channel::unbounded::<(ArtKey, Texture)>();
    std::thread::spawn(move || {
        while let Ok((key, data)) = request_rx.recv_blocking() {
            if let Some(texture) = decode_and_scale(&data, max_size)
                && result_tx.send_blocking((key, texture)).is_err()
            {
                break; // the AlbumGrid was dropped — nothing left to decode for.
            }
        }
    });
    (request_tx, result_rx)
}

/// Decodes `data` and downscales it to fit within `max_size` x `max_size`,
/// preserving aspect ratio. `None` when the bytes aren't a decodable image.
pub(crate) fn decode_and_scale(data: &AlbumArtData, max_size: i32) -> Option<Texture> {
    let bytes = glib::Bytes::from(data.as_bytes());
    let stream = MemoryInputStream::from_bytes(&bytes);
    let pixbuf =
        Pixbuf::from_stream_at_scale(&stream, max_size, max_size, true, Cancellable::NONE).ok()?;
    let format = if pixbuf.has_alpha() {
        MemoryFormat::R8g8b8a8
    } else {
        MemoryFormat::R8g8b8
    };
    let row_stride = pixbuf.rowstride() as usize;
    let pixels = pixbuf.pixel_bytes()?;
    let texture = MemoryTexture::new(pixbuf.width(), pixbuf.height(), format, &pixels, row_stride);
    Some(texture.upcast())
}
