use gtk4::Box as GtkBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::prelude::*;

use crate::ui::style;
use crate::ui::style::Margins;
use crate::ui::style::spacing;
use crate::ui::widgets::body_label;
use crate::ui::widgets::caption_label;

/// A `ListBoxRow` with a primary line over a dimmed caption line — the app's
/// standard two-line list row (queue entries today; any future track-with-
/// subtitle list reuses it).
pub fn two_line_row(primary: &str, secondary: &str) -> ListBoxRow {
    let row_box = GtkBox::new(Orientation::Vertical, 0);
    style::set_margins(
        &row_box,
        Margins::horizontal(spacing::M)
            .top(spacing::XS)
            .bottom(spacing::XS),
    );
    row_box.append(&body_label(primary));
    row_box.append(&caption_label(secondary));

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
