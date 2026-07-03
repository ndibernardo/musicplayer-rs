use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::ContentFit;
use gtk4::Label;
use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::Picture;
use gtk4::ScrolledWindow;
use gtk4::SelectionMode;
use gtk4::Widget;
use gtk4::gdk::Paintable;
use gtk4::gdk::Texture;
use gtk4::prelude::*;

use crate::library::album::AlbumSummary;
use crate::library::track::Track;
use crate::library::track::TrackDuration;

/// Fixed number of album covers per row. The grid is hand-built (not a
/// `GridView`) so a full-width track drawer can be inserted between rows.
const COLUMNS: usize = 4;

type TrackProvider = Rc<dyn Fn(&AlbumSummary) -> Vec<Track>>;
type TrackCallback = Rc<dyn Fn(Track)>;

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
    track_provider: Rc<RefCell<Option<TrackProvider>>>,
    on_track_activated: Rc<RefCell<Option<TrackCallback>>>,
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

        Self {
            widget: scrolled,
            grid_box,
            row_boxes: Rc::new(RefCell::new(Vec::new())),
            albums: Rc::new(RefCell::new(Vec::new())),
            open_album: Rc::new(RefCell::new(None)),
            drawer: Rc::new(RefCell::new(None)),
            track_provider: Rc::new(RefCell::new(None)),
            on_track_activated: Rc::new(RefCell::new(None)),
        }
    }

    /// Rebuilds the cover grid and closes any open drawer.
    pub fn set_albums(&self, albums: Vec<AlbumSummary>) {
        while let Some(child) = self.grid_box.first_child() {
            self.grid_box.remove(&child);
        }
        *self.open_album.borrow_mut() = None;
        *self.drawer.borrow_mut() = None;

        let mut row_boxes = Vec::new();
        for (row_index, chunk) in albums.chunks(COLUMNS).enumerate() {
            let row_box = GtkBox::new(Orientation::Horizontal, 6);
            for (col, summary) in chunk.iter().enumerate() {
                let index = row_index * COLUMNS + col;
                row_box.append(&self.cover_button(index, summary));
            }
            self.grid_box.append(&row_box);
            row_boxes.push(row_box);
        }
        *self.row_boxes.borrow_mut() = row_boxes;
        *self.albums.borrow_mut() = albums;
    }

    /// Supplies the tracks of an album, fetched when its drawer opens.
    pub fn set_track_provider<F: Fn(&AlbumSummary) -> Vec<Track> + 'static>(&self, f: F) {
        *self.track_provider.borrow_mut() = Some(Rc::new(f));
    }

    /// Registers the callback invoked when a track inside a drawer is activated.
    pub fn connect_track_activated<F: Fn(Track) + 'static>(&self, f: F) {
        *self.on_track_activated.borrow_mut() = Some(Rc::new(f));
    }

    fn cover_button(&self, index: usize, summary: &AlbumSummary) -> Button {
        let picture = Picture::new();
        picture.set_size_request(160, 160);
        picture.set_content_fit(ContentFit::Cover);
        picture.set_paintable(cover_paintable(summary).as_ref());

        let album_label = Label::new(Some(summary.album.as_str()));
        album_label.set_xalign(0.0);
        album_label.set_max_width_chars(20);
        album_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        album_label.add_css_class("heading");

        let artist_label = Label::new(Some(summary.artist.as_str()));
        artist_label.set_xalign(0.0);
        artist_label.set_max_width_chars(20);
        artist_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        artist_label.add_css_class("dim-label");

        let cell = GtkBox::new(Orientation::Vertical, 4);
        cell.append(&picture);
        cell.append(&album_label);
        cell.append(&artist_label);

        let button = Button::new();
        button.add_css_class("flat");
        button.set_child(Some(&cell));

        let this = self.clone();
        button.connect_clicked(move |_| this.toggle_drawer(index));
        button
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
        let Some(row_box) = row_boxes.get(index / COLUMNS) else {
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
            if let Some(track) = tracks.get(row.index() as usize) {
                callback(track.clone());
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
