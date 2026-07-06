use std::cell::Cell;
use std::rc::Rc;

use gtk4::Align;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::Entry;
use gtk4::FileDialog;
use gtk4::FileFilter;
use gtk4::Grid;
use gtk4::Image;
use gtk4::Orientation;
use gtk4::Window;
use gtk4::prelude::*;

use crate::library::metadata_edit::Shared;
use crate::library::metadata_edit::TrackEdit;
use crate::library::track::AlbumArtData;
use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Composer;
use crate::library::track::DiscNumber;
use crate::library::track::Genre;
use crate::library::track::Title;
use crate::library::track::Track;
use crate::library::track::TrackNumber;
use crate::library::track::Year;
use crate::ui::album_grid::decode_and_scale;

/// Side length (px) the preview cover is decoded and shown at. A one-off
/// main-thread decode is fine here — a single image, on explicit user action.
const PREVIEW_COVER_SIZE: i32 = 120;

/// Opens a modal editor for a single track. Every field's initial text is the
/// track's current value (possibly empty) — there is no "Mixed" state. `art`
/// is the track's currently embedded cover, if any. `on_save` receives the
/// edit and, only if the user picked a new cover, the replacement art bytes.
pub fn open_track_editor(
    parent: &impl IsA<Window>,
    track: Track,
    art: Option<AlbumArtData>,
    on_save: impl Fn(TrackEdit, Option<AlbumArtData>) + 'static,
) {
    open_editor(parent.upcast_ref(), "Edit Track", &[track], art, on_save);
}

/// Opens a modal editor for a batch of tracks — a single album's tracks, a
/// multi-album selection, or an arbitrary multi-track selection; all three
/// are just a `Vec<Track>`. A field common to every track prefills its
/// value; a field that differs shows a "Multiple Values" placeholder
/// instead. Only fields the user actually edits are set in the resulting
/// `TrackEdit` — untouched fields (including ones already showing "Multiple
/// Values") stay `None`, so every track keeps its own value for those. `art`
/// is the batch's current representative cover, if any.
pub fn open_album_editor(
    parent: &impl IsA<Window>,
    tracks: Vec<Track>,
    art: Option<AlbumArtData>,
    on_save: impl Fn(TrackEdit, Option<AlbumArtData>) + 'static,
) {
    let title = format!("Edit {} track(s)", tracks.len());
    open_editor(parent.upcast_ref(), &title, &tracks, art, on_save);
}

/// The shared builder behind both public entry points. A single-track slice
/// makes every field trivially `Shared::Common` (it agrees with itself), so
/// the same touched-field logic that protects an album's "Multiple Values"
/// fields also handles the single-track case correctly: an untouched field's
/// entry already holds the track's own value, so leaving it as `None` in the
/// resulting `TrackEdit` and re-applying it are equivalent.
fn open_editor(
    parent: &Window,
    title: &str,
    tracks: &[Track],
    art: Option<AlbumArtData>,
    on_save: impl Fn(TrackEdit, Option<AlbumArtData>) + 'static,
) {
    let window = Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .default_width(480)
        .default_height(560)
        .resizable(true)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 12);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);

    let cover = Image::new();
    cover.set_pixel_size(PREVIEW_COVER_SIZE);
    cover.set_halign(Align::Center);
    if let Some(art) = &art
        && let Some(texture) = decode_and_scale(art, PREVIEW_COVER_SIZE)
    {
        cover.set_paintable(Some(&texture));
    }
    root.append(&cover);

    let new_art: Rc<Cell<Option<Vec<u8>>>> = Rc::new(Cell::new(None));
    let cover_buttons = GtkBox::new(Orientation::Horizontal, 8);
    cover_buttons.set_halign(Align::Center);
    let choose_btn = Button::with_label("Choose Cover…");
    wire_choose_cover(&choose_btn, &window, &cover, Rc::clone(&new_art));
    cover_buttons.append(&choose_btn);
    let paste_btn = Button::with_label("Paste Cover");
    wire_paste_cover(&paste_btn, &window, &cover, Rc::clone(&new_art));
    cover_buttons.append(&paste_btn);
    root.append(&cover_buttons);

    let grid = Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(12);
    root.append(&grid);

    let title_field = field_row(&grid, 0, "Title", text_shared(tracks, |t| t.title.clone()));
    let artist_field = field_row(
        &grid,
        1,
        "Artist",
        text_shared(tracks, |t| t.artist.clone()),
    );
    let album_artist_field = field_row(
        &grid,
        2,
        "Album Artist",
        text_shared(tracks, |t| t.album_artist.clone()),
    );
    let album_field = field_row(&grid, 3, "Album", text_shared(tracks, |t| t.album.clone()));
    let genre_field = field_row(&grid, 4, "Genre", text_shared(tracks, |t| t.genre.clone()));
    let composer_field = field_row(
        &grid,
        5,
        "Composer",
        text_shared(tracks, |t| t.composer.clone()),
    );
    let track_number_field = field_row(
        &grid,
        6,
        "Track Number",
        number_shared(tracks, |t| t.track_number.value(), |n| n == 0),
    );
    let disc_number_field = field_row(
        &grid,
        7,
        "Disc Number",
        number_shared(tracks, |t| t.disc_number.value(), |n| n == 0),
    );
    let year_field = field_row(
        &grid,
        8,
        "Year",
        number_shared(tracks, |t| u32::from(t.year.value()), |y| y == 0),
    );

    let buttons = GtkBox::new(Orientation::Horizontal, 8);
    buttons.set_halign(Align::End);
    let cancel_btn = Button::with_label("Cancel");
    let save_btn = Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    buttons.append(&cancel_btn);
    buttons.append(&save_btn);
    root.append(&buttons);

    window.set_child(Some(&root));

    let for_cancel = window.clone();
    cancel_btn.connect_clicked(move |_| for_cancel.close());

    let for_save = window.clone();
    save_btn.connect_clicked(move |_| {
        let edit = TrackEdit {
            title: text_edit(&title_field, Title::new),
            artist: text_edit(&artist_field, Artist::new),
            album_artist: text_edit(&album_artist_field, Artist::new),
            album: text_edit(&album_field, AlbumTitle::new),
            genre: text_edit(&genre_field, Genre::new),
            composer: text_edit(&composer_field, Composer::new),
            track_number: number_edit(&track_number_field).map(TrackNumber::new),
            disc_number: number_edit(&disc_number_field).map(DiscNumber::new),
            year: number_edit(&year_field).map(|y: u32| Year::new(y as u16)),
        };
        on_save(edit, new_art.take().map(AlbumArtData::new));
        for_save.close();
    });

    window.present();
}

/// One field's `Entry` widget together with the flag that flips to `true` the
/// first time the user edits it — connected *after* the initial `set_text`,
/// so the programmatic prefill itself never marks the field touched.
struct Field {
    entry: Entry,
    touched: Rc<Cell<bool>>,
}

fn field_row(grid: &Grid, row: i32, label: &str, initial: Shared<String>) -> Field {
    let name = gtk4::Label::new(Some(label));
    name.set_xalign(0.0);
    name.set_margin_end(12);
    grid.attach(&name, 0, row, 1, 1);

    let entry = Entry::new();
    entry.set_hexpand(true);
    match &initial {
        Shared::Common(value) => entry.set_text(value),
        Shared::Mixed => entry.set_placeholder_text(Some("Multiple Values")),
    }
    grid.attach(&entry, 1, row, 1, 1);

    let touched = Rc::new(Cell::new(false));
    let touched_for_signal = Rc::clone(&touched);
    entry.connect_changed(move |_| touched_for_signal.set(true));

    Field { entry, touched }
}

/// `Shared::of` over `tracks`, converted to its display text via `show`.
fn text_shared<T: Clone + PartialEq + AsDisplayText>(
    tracks: &[Track],
    extract: impl Fn(&Track) -> T,
) -> Shared<String> {
    match Shared::of(tracks, extract) {
        Shared::Common(value) => Shared::Common(value.display_text()),
        Shared::Mixed => Shared::Mixed,
    }
}

/// Same as `text_shared`, but for the three numeric fields: `is_unknown`
/// decides whether a common value displays as blank (matching every other
/// "0/absent means unknown" field in the domain) rather than literal `0`.
fn number_shared(
    tracks: &[Track],
    extract: impl Fn(&Track) -> u32,
    is_unknown: impl Fn(u32) -> bool,
) -> Shared<String> {
    match Shared::of(tracks, extract) {
        Shared::Common(value) if is_unknown(value) => Shared::Common(String::new()),
        Shared::Common(value) => Shared::Common(value.to_string()),
        Shared::Mixed => Shared::Mixed,
    }
}

trait AsDisplayText {
    fn display_text(&self) -> String;
}

impl AsDisplayText for Title {
    fn display_text(&self) -> String {
        self.as_str().to_owned()
    }
}

impl AsDisplayText for Artist {
    fn display_text(&self) -> String {
        self.as_str().to_owned()
    }
}

impl AsDisplayText for AlbumTitle {
    fn display_text(&self) -> String {
        self.as_str().to_owned()
    }
}

impl AsDisplayText for Genre {
    fn display_text(&self) -> String {
        self.as_str().to_owned()
    }
}

impl AsDisplayText for Composer {
    fn display_text(&self) -> String {
        self.as_str().to_owned()
    }
}

/// `Some(new(text))` only if the user touched the field; `None` otherwise, so
/// an untouched "Multiple Values" field leaves every track's own value alone.
fn text_edit<T>(field: &Field, new: impl Fn(String) -> T) -> Option<T> {
    field
        .touched
        .get()
        .then(|| new(field.entry.text().to_string()))
}

/// The three-way rule for a numeric field: untouched stays `None`; touched
/// and blank means the domain's "unknown" (`0`), an explicit clear; touched
/// and unparsable (a typo) is treated as untouched rather than silently
/// blanking the field across a whole album — so it also stays `None`.
fn number_edit(field: &Field) -> Option<u32> {
    if !field.touched.get() {
        return None;
    }
    let text = field.entry.text();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Some(0)
    } else {
        trimmed.parse().ok()
    }
}

fn wire_choose_cover(
    button: &Button,
    window: &Window,
    cover: &Image,
    new_art: Rc<Cell<Option<Vec<u8>>>>,
) {
    let window = window.clone();
    let cover = cover.clone();
    button.connect_clicked(move |_| {
        let window = window.clone();
        let cover = cover.clone();
        let new_art = Rc::clone(&new_art);
        glib::spawn_future_local(async move {
            let dialog = FileDialog::new();
            dialog.set_title("Choose Cover Image");
            let filter = FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_mime_type("image/jpeg");
            filter.add_mime_type("image/png");
            dialog.set_default_filter(Some(&filter));

            let Ok(file) = dialog.open_future(Some(&window)).await else {
                return;
            };
            let Some(path) = file.path() else { return };
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            if let Some(texture) =
                decode_and_scale(&AlbumArtData::new(bytes.clone()), PREVIEW_COVER_SIZE)
            {
                cover.set_paintable(Some(&texture));
            }
            new_art.set(Some(bytes));
        });
    });
}

/// Wires "Paste Cover" to replace the preview with whatever image, if any,
/// is currently on the clipboard (e.g. copied from a browser or file
/// manager). The clipboard hands back decoded pixels, not file bytes, so
/// unlike `wire_choose_cover` this re-encodes the texture to PNG before
/// storing it — everything downstream (`AlbumArtData`, `metadata::write`)
/// only ever deals in encoded image bytes.
fn wire_paste_cover(
    button: &Button,
    window: &Window,
    cover: &Image,
    new_art: Rc<Cell<Option<Vec<u8>>>>,
) {
    let window = window.clone();
    let cover = cover.clone();
    button.connect_clicked(move |_| {
        let clipboard = window.clipboard();
        let cover = cover.clone();
        let new_art = Rc::clone(&new_art);
        glib::spawn_future_local(async move {
            let Ok(Some(texture)) = clipboard.read_texture_future().await else {
                return;
            };
            let bytes = texture.save_to_png_bytes().to_vec();
            if let Some(preview) =
                decode_and_scale(&AlbumArtData::new(bytes.clone()), PREVIEW_COVER_SIZE)
            {
                cover.set_paintable(Some(&preview));
            }
            new_art.set(Some(bytes));
        });
    });
}
