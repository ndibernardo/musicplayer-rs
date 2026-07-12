use gtk4::Label;
use gtk4::prelude::*;

/// A left-aligned, end-ellipsized text label — the app's standard body text
/// for list rows and cells.
pub fn body_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label
}

/// A [`body_label`] dimmed and shrunk to caption size — the standard second
/// line under a row's primary text.
pub fn caption_label(text: &str) -> Label {
    let label = body_label(text);
    label.add_css_class("dim-label");
    label.add_css_class("caption");
    label
}

/// A right-aligned, dimmed, tabular-numerals label for track numbers and
/// durations.
pub fn numeric_dim_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(1.0);
    label.add_css_class("dim-label");
    label.add_css_class("numeric");
    label
}
