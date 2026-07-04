use std::cell::Cell;
use std::cell::RefCell;
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
use gtk4::gdk::Paintable;
use gtk4::gdk::Texture;
use gtk4::prelude::*;

use crate::library::album::AlbumSummary;
use crate::library::track::Track;
use crate::library::track::TrackDuration;

/// Column count before the first width-driven reflow. The grid is hand-built
/// (not a `GridView`) so a full-width track drawer can be inserted between rows;
/// columns are recomputed from the viewport width (see `reflow`).
const DEFAULT_COLUMNS: usize = 4;

/// Per-cover horizontal budget beyond the cover itself (button padding + inter-
/// cover spacing) — used to estimate how many covers fit a given width.
const COVER_SLOT_EXTRA: i32 = 24;

/// Combined left+right margin of the grid container (px).
const GRID_MARGIN_TOTAL: i32 = 12;

/// Default cover side length (px), used until a saved or user-chosen value is
/// applied via `set_cover_size`.
const DEFAULT_COVER_SIZE: i32 = 200;

type TrackProvider = Rc<dyn Fn(&AlbumSummary) -> Vec<Track>>;
/// Invoked with the album's full track list and the index of the activated one,
/// so the caller can enqueue the whole album starting at that track.
type TrackCallback = Rc<dyn Fn(Vec<Track>, usize)>;
/// Invoked with an album's full track list when its cover is opened.
type AlbumCallback = Rc<dyn Fn(Vec<Track>)>;

/// Album-art browser: a grid of cover cells. Activating a cover opens a
/// full-width drawer with that album's full track list, inline on its own row
/// directly beneath the cover's row. Activating it again closes the drawer.
#[derive(Clone)]
pub struct AlbumGrid {
    pub widget: ScrolledWindow,
    grid_box: GtkBox,
    row_boxes: Rc<RefCell<Vec<GtkBox>>>,
    albums: Rc<RefCell<Vec<AlbumSummary>>>,
    open_album: Rc<RefCell<Option<usize>>>,
    drawer: Rc<RefCell<Option<Widget>>>,
    cover_size: Rc<Cell<i32>>,
    // Current column count, recomputed from the viewport width.
    columns: Rc<Cell<usize>>,
    // Cover cells in album order, reused when the grid reflows (no re-decode).
    cells: Rc<RefCell<Vec<GtkBox>>>,
    // Cover image + its cell box, kept so `set_cover_size` can resize in place.
    covers: Rc<RefCell<Vec<(Image, GtkBox)>>>,
    track_provider: Rc<RefCell<Option<TrackProvider>>>,
    on_track_activated: Rc<RefCell<Option<TrackCallback>>>,
    on_album_activated: Rc<RefCell<Option<AlbumCallback>>>,
}

impl AlbumGrid {
    pub fn new() -> Self {
        let grid_box = GtkBox::new(Orientation::Vertical, 6);
        grid_box.set_margin_start(6);
        grid_box.set_margin_end(6);
        grid_box.set_margin_top(6);
        grid_box.set_margin_bottom(6);

        let scrolled = ScrolledWindow::new();
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&grid_box));

        let grid = Self {
            widget: scrolled,
            grid_box,
            row_boxes: Rc::new(RefCell::new(Vec::new())),
            albums: Rc::new(RefCell::new(Vec::new())),
            open_album: Rc::new(RefCell::new(None)),
            drawer: Rc::new(RefCell::new(None)),
            cover_size: Rc::new(Cell::new(DEFAULT_COVER_SIZE)),
            columns: Rc::new(Cell::new(DEFAULT_COLUMNS)),
            cells: Rc::new(RefCell::new(Vec::new())),
            covers: Rc::new(RefCell::new(Vec::new())),
            track_provider: Rc::new(RefCell::new(None)),
            on_track_activated: Rc::new(RefCell::new(None)),
            on_album_activated: Rc::new(RefCell::new(None)),
        };
        grid.install_reflow_handler();
        grid
    }

    /// Reflows whenever the scroll viewport's width changes.
    fn install_reflow_handler(&self) {
        let this = self.clone();
        self.widget
            .hadjustment()
            .connect_page_size_notify(move |_| this.reflow());
    }

    /// Rebuilds the cover grid and closes any open drawer.
    pub fn set_albums(&self, albums: Vec<AlbumSummary>) {
        while let Some(child) = self.grid_box.first_child() {
            self.grid_box.remove(&child);
        }
        *self.open_album.borrow_mut() = None;
        *self.drawer.borrow_mut() = None;
        self.covers.borrow_mut().clear();

        let cells: Vec<GtkBox> = albums
            .iter()
            .enumerate()
            .map(|(index, summary)| self.cover_cell(index, summary))
            .collect();
        *self.cells.borrow_mut() = cells;
        *self.albums.borrow_mut() = albums;

        self.lay_out(self.columns_for_current_width());
    }

    /// Arranges the existing cover buttons into rows of `columns`.
    fn lay_out(&self, columns: usize) {
        self.columns.set(columns);
        let cells = self.cells.borrow();
        let mut row_boxes = Vec::new();
        for chunk in cells.chunks(columns) {
            let row_box = GtkBox::new(Orientation::Horizontal, 6);
            for cell in chunk {
                row_box.append(cell);
            }
            self.grid_box.append(&row_box);
            row_boxes.push(row_box);
        }
        *self.row_boxes.borrow_mut() = row_boxes;
    }

    /// Re-lays the covers when the column count changes (window resize or a new
    /// cover size). Reuses the built cells — no texture re-decode.
    fn reflow(&self) {
        let columns = self.columns_for_current_width();
        if columns == self.columns.get() {
            return;
        }
        if let Some(open) = self.drawer.borrow_mut().take() {
            self.grid_box.remove(&open);
        }
        *self.open_album.borrow_mut() = None;
        for cell in self.cells.borrow().iter() {
            cell.unparent();
        }
        while let Some(child) = self.grid_box.first_child() {
            self.grid_box.remove(&child);
        }
        self.lay_out(columns);
    }

    /// How many covers fit the current viewport width (at least one).
    fn columns_for_current_width(&self) -> usize {
        let available = self.widget.hadjustment().page_size() as i32 - GRID_MARGIN_TOTAL;
        let slot = self.cover_size.get() + COVER_SLOT_EXTRA;
        (available / slot).max(1) as usize
    }

    /// Supplies the tracks of an album, fetched when its drawer opens.
    pub fn set_track_provider<F: Fn(&AlbumSummary) -> Vec<Track> + 'static>(&self, f: F) {
        *self.track_provider.borrow_mut() = Some(Rc::new(f));
    }

    /// Registers the callback invoked when a track inside a drawer is activated.
    /// It receives the album's full track list and the activated track's index.
    pub fn connect_track_activated<F: Fn(Vec<Track>, usize) + 'static>(&self, f: F) {
        *self.on_track_activated.borrow_mut() = Some(Rc::new(f));
    }

    /// Registers the callback invoked with an album's tracks when its cover is
    /// opened, so the caller can enqueue the whole album.
    pub fn connect_album_activated<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        *self.on_album_activated.borrow_mut() = Some(Rc::new(f));
    }

    /// Resizes every cover in place to `size` px, then reflows since a different
    /// cover size changes how many fit per row. No texture re-decode.
    pub fn set_cover_size(&self, size: i32) {
        self.cover_size.set(size);
        for (image, cell) in self.covers.borrow().iter() {
            image.set_pixel_size(size);
            cell.set_width_request(size);
        }
        self.reflow();
    }

    fn cover_cell(&self, index: usize, summary: &AlbumSummary) -> GtkBox {
        let size = self.cover_size.get();

        // GtkImage renders at a fixed pixel size; GtkPicture instead grows to the
        // texture's natural size (the covers-too-big bug), since size_request only
        // sets a minimum.
        let image = Image::new();
        image.set_pixel_size(size);
        image.set_halign(Align::Center);
        image.set_paintable(cover_paintable(summary).as_ref());

        let album_label = Label::new(Some(summary.album.as_str()));
        album_label.set_xalign(0.0);
        album_label.set_max_width_chars(16);
        album_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        album_label.add_css_class("heading");

        let artist_label = Label::new(Some(summary.artist.as_str()));
        artist_label.set_xalign(0.0);
        artist_label.set_max_width_chars(16);
        artist_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        artist_label.add_css_class("dim-label");

        let cell = GtkBox::new(Orientation::Vertical, 4);
        cell.add_css_class("activatable");
        // Pin the cell to the cover width so labels ellipsize instead of widening it.
        cell.set_width_request(size);
        cell.set_halign(Align::Start);
        cell.append(&image);
        cell.append(&album_label);
        cell.append(&artist_label);

        // A single click opens the track drawer; a double click plays the whole
        // album. Using one gesture on the cell (rather than two `clicked` signals
        // from a button) avoids the drawer flickering open then shut on a
        // double-click, since `pressed` fires once per press with a rising count.
        let gesture = GestureClick::new();
        let this = self.clone();
        gesture.connect_pressed(move |_, n_press, _, _| match n_press {
            1 => this.toggle_drawer(index),
            2 => this.activate_album(index),
            _ => {}
        });
        cell.add_controller(gesture);

        self.covers.borrow_mut().push((image, cell.clone()));
        cell
    }

    /// Enqueues the whole album at `index` by firing the album callback.
    fn activate_album(&self, index: usize) {
        let albums = self.albums.borrow();
        let Some(summary) = albums.get(index) else {
            return;
        };
        let tracks = match self.track_provider.borrow().as_ref() {
            Some(provide) => provide(summary),
            None => Vec::new(),
        };
        if let Some(callback) = self.on_album_activated.borrow().as_ref() {
            callback(tracks);
        }
    }

    /// Opens the drawer for `index` beneath its row, or closes it if already open.
    fn toggle_drawer(&self, index: usize) {
        if let Some(open) = self.drawer.borrow_mut().take() {
            self.grid_box.remove(&open);
        }
        if *self.open_album.borrow() == Some(index) {
            *self.open_album.borrow_mut() = None;
            return;
        }

        let drawer = {
            let albums = self.albums.borrow();
            let Some(summary) = albums.get(index) else {
                return;
            };
            let tracks = match self.track_provider.borrow().as_ref() {
                Some(provide) => provide(summary),
                None => Vec::new(),
            };
            build_drawer(summary, tracks, self.on_track_activated.borrow().clone())
        };

        let row_boxes = self.row_boxes.borrow();
        let Some(row_box) = row_boxes.get(index / self.columns.get()) else {
            return;
        };
        self.grid_box.insert_child_after(&drawer, Some(row_box));
        *self.drawer.borrow_mut() = Some(drawer.upcast());
        *self.open_album.borrow_mut() = Some(index);
    }
}

/// Builds the inline drawer: a heading plus the album's full track list.
fn build_drawer(
    summary: &AlbumSummary,
    tracks: Vec<Track>,
    on_track: Option<TrackCallback>,
) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 0);
    container.add_css_class("frame");
    container.set_margin_top(2);
    container.set_margin_bottom(2);

    let heading = Label::new(Some(&drawer_heading(summary)));
    heading.set_xalign(0.0);
    heading.set_margin_start(8);
    heading.set_margin_top(6);
    heading.set_margin_bottom(4);
    heading.add_css_class("heading");
    container.append(&heading);

    let list = ListBox::new();
    list.set_selection_mode(SelectionMode::Single);
    list.set_activate_on_single_click(false);
    for track in &tracks {
        list.append(&track_row(track));
    }
    if let Some(callback) = on_track {
        list.connect_row_activated(move |_, row| {
            let index = row.index() as usize;
            if index < tracks.len() {
                callback(tracks.clone(), index);
            }
        });
    }
    container.append(&list);
    container
}

fn drawer_heading(summary: &AlbumSummary) -> String {
    let title = format!("{} — {}", summary.album.as_str(), summary.artist.as_str());
    if summary.year.is_unknown() {
        title
    } else {
        format!("{title} ({})", summary.year.value())
    }
}

fn track_row(track: &Track) -> ListBoxRow {
    let number = Label::new(Some(&track_number(track)));
    number.set_width_chars(3);
    number.set_xalign(1.0);
    number.add_css_class("dim-label");

    let title = Label::new(Some(track.title.as_str()));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);

    let duration = Label::new(Some(&format_duration(track.duration)));
    duration.add_css_class("dim-label");

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

fn track_number(track: &Track) -> String {
    if track.track_number.is_unknown() {
        String::new()
    } else {
        track.track_number.value().to_string()
    }
}

/// Decodes embedded cover art to a paintable, or `None` when the album has no
/// art or the bytes cannot be decoded.
fn cover_paintable(summary: &AlbumSummary) -> Option<Paintable> {
    let art = summary.art.as_ref()?;
    let bytes = glib::Bytes::from(art.as_bytes());
    Texture::from_bytes(&bytes).ok().map(Paintable::from)
}

fn format_duration(d: TrackDuration) -> String {
    let total_secs = d.as_secs();
    format!("{}:{:02}", total_secs / 60, total_secs % 60)
}
