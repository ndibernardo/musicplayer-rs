use gtk4::Expander;
use gtk4::Widget;
use gtk4::prelude::*;

use crate::ui::style;
use crate::ui::style::Margins;
use crate::ui::style::StyleClass;
use crate::ui::style::spacing;

/// A titled, collapsible sidebar section with the app's standard title
/// styling and margins — the one place the "bold name over an indented body"
/// look is built. Top spacing is the surrounding panel's job, so stacked
/// sections keep a single-`M` rhythm instead of doubling up.
#[derive(Clone)]
pub struct CollapsibleSection {
    expander: Expander,
}

impl CollapsibleSection {
    pub fn new(title: &str, body: &impl IsA<Widget>) -> Self {
        let expander = Expander::new(Some(title));
        expander.set_expanded(true);
        style::set_margins(
            &expander,
            Margins::none().start(spacing::S).bottom(spacing::M),
        );
        expander.set_child(Some(body));
        if let Some(label_widget) = expander.label_widget() {
            style::add_class(&label_widget, StyleClass::SectionName);
        }
        Self { expander }
    }

    /// Collapses the section when it empties — there's nothing left inside
    /// to expand into. Never re-expands on its own, so a manual collapse of
    /// a non-empty section is left alone.
    pub fn set_empty(&self, empty: bool) {
        if empty {
            self.expander.set_expanded(false);
        }
    }

    pub fn set_visible(&self, visible: bool) {
        self.expander.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.expander.is_visible()
    }

    pub fn widget(&self) -> &Expander {
        &self.expander
    }
}
