use gtk4::Box as GtkBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::prelude::*;

use crate::ui::widgets::body_label;
use crate::ui::widgets::caption_label;

/// A `ListBoxRow` with a primary line over a dimmed caption line — the app's
/// standard two-line list row (queue entries today; any future track-with-
/// subtitle list reuses it). No margin: the inset is `list.app-section-list
/// row`'s own CSS padding, so a selected row's highlight fills the row
/// edge-to-edge instead of leaving a gap before it.
pub fn two_line_row(primary: &str, secondary: &str) -> ListBoxRow {
    let row_box = GtkBox::new(Orientation::Vertical, 0);
    row_box.append(&body_label(primary));
    row_box.append(&caption_label(secondary));

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
