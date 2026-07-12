use std::rc::Rc;

use async_channel::Sender;
use gtk4::Box as GtkBox;
use gtk4::Label;
use gtk4::Orientation;
use gtk4::Paned;
use gtk4::Separator;
use gtk4::ToggleButton;
use gtk4::prelude::*;

use crate::library::db::Db;
use crate::library::settings::Settings;
use crate::library::settings::SidebarEdge;
use crate::library::window_state::WindowMessage;
use crate::ui::folder_list::FolderList;
use crate::ui::queue_view::QueueView;
use crate::ui::sidebar::Sidebar;
use crate::ui::widgets::SidebarPanel;

/// Assembles the two app sidebars as `SidebarPanel` instances, so they cannot
/// drift apart visually: each panel's title ("Library", "Queue") sits in the
/// shared header slot — flush with the panel top, no extra inset — and its
/// scrollable area(s) sit in the content slot below. The right panel stacks
/// the queue list above the folder tree, set off by a separator, with the
/// status line as its footer.
pub(super) fn build_sidebar_panels(
    filter_sidebar: &Sidebar,
    queue_view: &QueueView,
    folder_list: &FolderList,
    status_label: &Label,
) -> (SidebarPanel, SidebarPanel) {
    let left = SidebarPanel::builder(filter_sidebar.content())
        .header(filter_sidebar.header())
        .footer(filter_sidebar.footer())
        .build();

    let right_stack = GtkBox::new(Orientation::Vertical, 0);
    right_stack.append(queue_view.content());
    right_stack.append(&Separator::new(Orientation::Horizontal));
    right_stack.append(folder_list.widget());
    let right = SidebarPanel::builder(&right_stack)
        .header(queue_view.header())
        .footer(status_label)
        .build();

    (left, right)
}

pub(super) fn wire_folder_list(folder_list: &FolderList, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    folder_list.connect_remove_requested(move |folder| {
        let _ = tx.send_blocking(WindowMessage::FolderRemoved(folder));
    });
}

pub(super) fn wire_sidebar(filter_sidebar: &Sidebar, tx: &Sender<WindowMessage>) {
    let tx_filter = tx.clone();
    filter_sidebar.connect_filter_selected(move |filter| {
        let _ = tx_filter.send_blocking(WindowMessage::FilterSelected(filter));
    });
    let tx_fields = tx.clone();
    filter_sidebar.connect_fields_changed(move |fields| {
        let _ = tx_fields.send_blocking(WindowMessage::SidebarFieldsChanged(fields));
    });
}

pub(super) fn wire_queue_view(queue_view: &QueueView, tx: &Sender<WindowMessage>) {
    let tx = tx.clone();
    queue_view.connect_track_selected(move |index| {
        let _ = tx.send_blocking(WindowMessage::QueueTrackSelected(index));
    });
}

/// Links a sidebar toggle to `paned`'s divider and persists both the open
/// state and, while open, the dragged width — so the sidebar reopens (this
/// session or a future one) at the size it was last left at. A `Revealer`
/// here would leave the paned's fixed `position` allocating full width to an
/// empty child, so the divider itself is the collapse mechanism.
///
/// `edge` decides the divider math: a `Start`-edge sidebar's width *is* the
/// divider position, while an `End`-edge sidebar's width is `paned`'s total
/// allocated width minus the divider position — read at toggle/drag time,
/// once the widget is realized and that width is accurate (see
/// `estimated_content_width` at construction for the one case where it isn't
/// yet).
pub(super) fn wire_sidebar_toggle(
    toggle: &ToggleButton,
    paned: &Paned,
    edge: SidebarEdge,
    db: Rc<Db>,
) {
    let paned_for_toggle = paned.clone();
    let db_for_toggle = Rc::clone(&db);
    toggle.connect_toggled(move |btn| {
        let settings = Settings::new(&db_for_toggle);
        let open = btn.is_active();
        settings.set_sidebar_open(edge, open);
        let width = settings.sidebar_width(edge);
        paned_for_toggle.set_position(match edge {
            SidebarEdge::Start => {
                if open {
                    width
                } else {
                    0
                }
            }
            SidebarEdge::End => {
                let total_width = paned_for_toggle.width();
                if open {
                    (total_width - width).max(0)
                } else {
                    total_width
                }
            }
        });
    });

    let toggle_for_drag = toggle.clone();
    paned.connect_position_notify(move |paned| {
        if !toggle_for_drag.is_active() {
            return;
        }
        let width = match edge {
            SidebarEdge::Start => paned.position(),
            SidebarEdge::End => paned.width() - paned.position(),
        };
        if width > 0 {
            Settings::new(&db).set_sidebar_width(edge, width);
        }
    });
}
