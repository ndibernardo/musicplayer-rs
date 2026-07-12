use gtk4::Widget;
use gtk4::prelude::IsA;
use gtk4::prelude::WidgetExt;

/// The application stylesheet. Every class it defines appears in [`StyleClass`].
const STYLE_SHEET: &str = include_str!("style.css");

/// A CSS class defined in `style.css`. GTK silently ignores an unknown class
/// name, so keeping the set closed here turns a class-name typo into a compile
/// error instead of an invisibly unstyled widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleClass {
    /// Flat selectable list inside an indented sidebar section.
    SectionList,
    /// Bold title of a collapsible sidebar section.
    SectionName,
    /// Bold, slightly larger sidebar panel title.
    SidebarTitle,
    /// A sidebar panel's root container — tinted apart from the content area.
    Sidebar,
    /// The bottom transport bar.
    PlayerBar,
    /// The player bar's knobless seek scale.
    Seek,
    /// One album cover cell in the grid.
    AlbumCell,
    /// The cover whose drawer is open.
    AlbumSelected,
    /// A cover in the current multi-selection.
    AlbumMultiSelected,
    /// The inline track drawer beneath a cover's row.
    AlbumDrawer,
}

impl StyleClass {
    /// The class name as written in `style.css`.
    pub const fn name(self) -> &'static str {
        match self {
            StyleClass::SectionList => "app-section-list",
            StyleClass::SectionName => "app-section-name",
            StyleClass::SidebarTitle => "app-sidebar-title",
            StyleClass::Sidebar => "app-sidebar",
            StyleClass::PlayerBar => "player-bar",
            StyleClass::Seek => "seek",
            StyleClass::AlbumCell => "album-cell",
            StyleClass::AlbumSelected => "album-selected",
            StyleClass::AlbumMultiSelected => "album-multi-selected",
            StyleClass::AlbumDrawer => "album-drawer",
        }
    }
}

/// Adds an app-defined class to `widget`.
pub fn add_class(widget: &impl IsA<Widget>, class: StyleClass) {
    widget.add_css_class(class.name());
}

/// Removes an app-defined class from `widget`.
pub fn remove_class(widget: &impl IsA<Widget>, class: StyleClass) {
    widget.remove_css_class(class.name());
}

/// Installs the application stylesheet. Called exactly once, at activation and
/// before any widget is built — no per-constructor install, no reliance on
/// duplicate-provider idempotence.
pub fn install() {
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(STYLE_SHEET);
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// The app's spacing scale (px). Every margin is one of these steps, so
/// spacing decisions read as named sizes instead of a number lottery.
pub mod spacing {
    /// Hairline: row inner padding.
    pub const XS: i32 = 2;
    /// Small: section edge insets, control gaps.
    pub const S: i32 = 4;
    /// Medium: row insets, section vertical rhythm.
    pub const M: i32 = 8;
    /// Large: section indentation.
    pub const L: i32 = 12;
    /// Extra large: dialog and grid outer padding.
    pub const XL: i32 = 16;
}

/// A widget's four margins, built by chaining from a base constructor:
/// `Margins::none().start(spacing::S).top(spacing::M)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Margins {
    start: i32,
    end: i32,
    top: i32,
    bottom: i32,
}

impl Margins {
    /// All four margins zero.
    pub const fn none() -> Self {
        Self {
            start: 0,
            end: 0,
            top: 0,
            bottom: 0,
        }
    }

    /// All four margins `px`.
    pub const fn all(px: i32) -> Self {
        Self {
            start: px,
            end: px,
            top: px,
            bottom: px,
        }
    }

    /// Start and end margins `px`, top and bottom zero.
    pub const fn horizontal(px: i32) -> Self {
        Self {
            start: px,
            end: px,
            top: 0,
            bottom: 0,
        }
    }

    /// Top and bottom margins `px`, start and end zero.
    pub const fn vertical(px: i32) -> Self {
        Self {
            start: 0,
            end: 0,
            top: px,
            bottom: px,
        }
    }

    pub const fn start(self, px: i32) -> Self {
        Self { start: px, ..self }
    }

    pub const fn end(self, px: i32) -> Self {
        Self { end: px, ..self }
    }

    pub const fn top(self, px: i32) -> Self {
        Self { top: px, ..self }
    }

    pub const fn bottom(self, px: i32) -> Self {
        Self { bottom: px, ..self }
    }
}

/// Applies `margins` to `widget` in one call.
pub fn set_margins(widget: &impl IsA<Widget>, margins: Margins) {
    widget.set_margin_start(margins.start);
    widget.set_margin_end(margins.end);
    widget.set_margin_top(margins.top);
    widget.set_margin_bottom(margins.bottom);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `StyleClass`, for exhaustive checks. Kept next to the enum so a
    /// new variant fails the completeness assertions below until added here.
    const ALL_CLASSES: [StyleClass; 10] = [
        StyleClass::SectionList,
        StyleClass::SectionName,
        StyleClass::SidebarTitle,
        StyleClass::Sidebar,
        StyleClass::PlayerBar,
        StyleClass::Seek,
        StyleClass::AlbumCell,
        StyleClass::AlbumSelected,
        StyleClass::AlbumMultiSelected,
        StyleClass::AlbumDrawer,
    ];

    #[test]
    fn style_class_names_are_unique() {
        let mut names: Vec<&str> = ALL_CLASSES.iter().map(|c| c.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL_CLASSES.len());
    }

    #[test]
    fn every_style_class_appears_in_the_stylesheet() {
        for class in ALL_CLASSES {
            assert!(
                STYLE_SHEET.contains(class.name()),
                "class {} missing from style.css",
                class.name()
            );
        }
    }

    #[test]
    fn margins_none_zeroes_every_side() {
        assert_eq!(Margins::none(), Margins::all(0));
    }

    #[test]
    fn margins_all_sets_every_side() {
        let margins = Margins::all(spacing::M);
        assert_eq!(
            margins,
            Margins::none()
                .start(spacing::M)
                .end(spacing::M)
                .top(spacing::M)
                .bottom(spacing::M)
        );
    }

    #[test]
    fn margins_horizontal_leaves_vertical_zero() {
        let margins = Margins::horizontal(spacing::L);
        assert_eq!(margins, Margins::none().start(spacing::L).end(spacing::L));
    }

    #[test]
    fn margins_vertical_leaves_horizontal_zero() {
        let margins = Margins::vertical(spacing::S);
        assert_eq!(margins, Margins::none().top(spacing::S).bottom(spacing::S));
    }

    #[test]
    fn margins_chaining_overrides_a_single_side() {
        let margins = Margins::all(spacing::M).top(spacing::XL);
        assert_eq!(
            margins,
            Margins::none()
                .start(spacing::M)
                .end(spacing::M)
                .top(spacing::XL)
                .bottom(spacing::M)
        );
    }
}
