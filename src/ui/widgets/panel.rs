use gtk4::Box as GtkBox;
use gtk4::Orientation;
use gtk4::Separator;
use gtk4::Widget;
use gtk4::prelude::*;

use crate::ui::style;
use crate::ui::style::Margins;
use crate::ui::style::StyleClass;
use crate::ui::style::spacing;

/// Minimum width both sidebars share.
const SIDEBAR_MIN_WIDTH: i32 = 220;

/// The common sidebar skeleton — optional header, expanding content, optional
/// footer set off by a separator — so the two app sidebars are instances of
/// one structure and cannot drift apart visually. The panel owns the shared
/// width, the `.app-sidebar` tint, the content's indentation and top inset,
/// and the content's vertical expansion.
pub struct SidebarPanel {
    root: GtkBox,
}

impl SidebarPanel {
    /// Starts a panel around its one required part. Header and footer attach
    /// via the builder.
    pub fn builder(content: &impl IsA<Widget>) -> SidebarPanelBuilder {
        SidebarPanelBuilder {
            content: content.clone().upcast(),
            header: None,
            footer: None,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.root
    }
}

pub struct SidebarPanelBuilder {
    content: Widget,
    header: Option<Widget>,
    footer: Option<Widget>,
}

impl SidebarPanelBuilder {
    /// Widget pinned above the content — a panel title or title button.
    pub fn header(mut self, header: &impl IsA<Widget>) -> Self {
        self.header = Some(header.clone().upcast());
        self
    }

    /// Widget pinned below the content, set off by a separator — an actions
    /// strip or a status line.
    pub fn footer(mut self, footer: &impl IsA<Widget>) -> Self {
        self.footer = Some(footer.clone().upcast());
        self
    }

    pub fn build(self) -> SidebarPanel {
        let root = GtkBox::new(Orientation::Vertical, 0);
        style::add_class(&root, StyleClass::Sidebar);
        root.set_width_request(SIDEBAR_MIN_WIDTH);
        root.set_vexpand(true);

        if let Some(header) = &self.header {
            style::set_margins(header, Margins::none().top(spacing::S));
            root.append(header);
        }

        // No left margin: a selected row's highlight must reach the panel's
        // own left edge, not stop short of it. Sections supply their own
        // bottom rhythm and text inset (CSS padding); the panel supplies
        // only the top inset, so stacked sections keep a single-`M` gap.
        self.content.set_vexpand(true);
        style::set_margins(&self.content, Margins::none().top(spacing::M));
        root.append(&self.content);

        if let Some(footer) = &self.footer {
            root.append(&Separator::new(Orientation::Horizontal));
            root.append(footer);
        }

        SidebarPanel { root }
    }
}
