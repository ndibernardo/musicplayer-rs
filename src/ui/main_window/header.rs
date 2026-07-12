use std::rc::Rc;

use async_channel::Sender;
use gtk4::ApplicationWindow;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::DropDown;
use gtk4::FileDialog;
use gtk4::Image;
use gtk4::Label;
use gtk4::Orientation;
use gtk4::Scale;
use gtk4::Spinner;
use gtk4::Stack;
use gtk4::ToggleButton;
use gtk4::prelude::*;

use crate::library::album::AlbumSort;
use crate::library::album::AlbumSortField;
use crate::library::album::SortDirection;
use crate::library::db::LibraryFolder;
use crate::library::settings::COVER_SIZE_MAX;
use crate::library::settings::COVER_SIZE_MIN;
use crate::library::view_mode::ViewMode;
use crate::library::window_state::WindowMessage;
use crate::ui::album_grid::AlbumGrid;
use crate::ui::column_picker::ColumnPicker;
use crate::ui::player_bar::PlayerBar;
use crate::ui::widgets::AppIcon;

/// The album-grid sort fields in dropdown order.
const SORT_FIELDS: [AlbumSortField; 4] = [
    AlbumSortField::AlbumArtist,
    AlbumSortField::Year,
    AlbumSortField::Genre,
    AlbumSortField::Album,
];
/// Human labels for `SORT_FIELDS`, in the same order.
const SORT_FIELD_LABELS: [&str; 4] = ["Album Artist", "Year", "Genre", "Album"];

/// Restores the sort controls and wires further changes to send `WindowMessage`s.
pub(super) fn wire_sort_controls(
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
pub(super) fn wire_cover_size(
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
pub(super) fn wire_column_picker(column_picker: &ColumnPicker, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    column_picker.connect_changed(move |prefs| {
        let _ = tx.send_blocking(WindowMessage::ColumnPrefsChanged(prefs));
    });
}

/// Restores the active view and wires the list/grid toggle buttons.
pub(super) fn wire_view_toggles(
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

pub(super) fn wire_volume(player_bar: &PlayerBar, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    player_bar.connect_volume_changed(move |percent| {
        let _ = tx.send_blocking(WindowMessage::VolumeChanged(percent));
    });
}

pub(super) fn wire_scan_button(scan_btn: &Button, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    scan_btn.connect_clicked(move |_| {
        let _ = tx.send_blocking(WindowMessage::ScanRequested);
    });
}

/// Add-folder button: pick a folder, then dispatch `WindowMessage::FolderAdded`. The DB
/// write itself happens in `Context::apply` — this only handles the async
/// file-dialog interaction, which doesn't belong in a state transition.
pub(super) fn wire_add_folder_button(
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

pub(super) fn build_sort_controls() -> (GtkBox, DropDown, ToggleButton) {
    let sort_icon = Image::from_icon_name(AppIcon::ViewSortDescending.name());
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

pub(super) fn build_size_scale() -> Scale {
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

pub(super) fn build_view_toggles() -> (ToggleButton, ToggleButton) {
    let list_toggle = ToggleButton::new();
    list_toggle.set_icon_name(AppIcon::ViewList.name());
    list_toggle.set_tooltip_text(Some("Track list"));
    list_toggle.set_active(true);
    let grid_toggle = ToggleButton::new();
    grid_toggle.set_icon_name(AppIcon::ViewGrid.name());
    grid_toggle.set_tooltip_text(Some("Album grid"));
    grid_toggle.set_group(Some(&list_toggle));
    (list_toggle, grid_toggle)
}

pub(super) fn build_scan_indicator() -> (Spinner, Label, GtkBox) {
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
        SortDirection::Ascending => AppIcon::ViewSortAscending.name(),
        SortDirection::Descending => AppIcon::ViewSortDescending.name(),
    }
}
