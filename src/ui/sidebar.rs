use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::CheckButton;
use gtk4::Label;
use gtk4::Orientation;
use gtk4::Popover;
use gtk4::ScrolledWindow;
use gtk4::SelectionMode;
use gtk4::pango;
use gtk4::prelude::*;

use crate::library::filter::FilterField;
use crate::library::filter::LibraryFilter;
use crate::ui::style;
use crate::ui::style::Margins;
use crate::ui::style::StyleClass;
use crate::ui::style::spacing;
use crate::ui::widgets::AppIcon;
use crate::ui::widgets::Callback;
use crate::ui::widgets::NavTree;
use crate::ui::widgets::flat_icon_button;
use crate::ui::widgets::flat_menu_button;
use crate::ui::widgets::remove_all_children;

/// Genre / album artist / artist / year / composer browser: a single nav
/// tree with one top-level row per [`FilterField`] the user has enabled via
/// the filter picker, and one leaf row per distinct value under it. Emits a
/// `LibraryFilter` when the user selects a leaf, and the new active field
/// list whenever the picker's selection changes.
///
/// Exposes its three parts — title label, scrollable tree, actions strip —
/// separately, so the caller can assemble them into the shared `SidebarPanel`
/// skeleton alongside the right sidebar's parts.
#[derive(Clone)]
pub struct Sidebar {
    header: Label,
    content: ScrolledWindow,
    footer: GtkBox,
    inner: Rc<SidebarInner>,
}

struct SidebarInner {
    tree: NavTree<String>,
    picker_list: GtkBox,
    /// The field/value pair behind the currently selected row, if any —
    /// tracked separately from GTK's own selection state so a second click on
    /// the same row can be recognised as a request to deselect it.
    current_selection: RefCell<Option<(FilterField, String)>>,
    on_select: Callback<LibraryFilter>,
    on_fields_changed: Callback<Vec<FilterField>>,
}

impl SidebarInner {
    fn active_fields(&self) -> Vec<FilterField> {
        FilterField::all()
            .into_iter()
            .filter(|f| self.tree.is_category_visible(f.index()))
            .collect()
    }
}

impl Sidebar {
    pub fn new(active_fields: &[FilterField]) -> Self {
        let title = build_library_label();
        let (picker_btn, picker_list) = build_filter_picker();
        let clear_btn = flat_icon_button(AppIcon::EditClear, "Clear filter");
        let actions_row = build_actions_row(&picker_btn, &clear_btn);

        let render: Rc<dyn Fn(&String) -> String> = Rc::new(String::clone);
        let categories = FilterField::all()
            .map(|field| (field.label(), Rc::clone(&render)))
            .to_vec();
        let tree = NavTree::new(SelectionMode::Single, pango::EllipsizeMode::End, categories);
        for field in active_fields {
            tree.set_category_visible(field.index(), true);
        }

        let inner = Rc::new(SidebarInner {
            tree,
            picker_list,
            current_selection: RefCell::new(None),
            on_select: Callback::new(),
            on_fields_changed: Callback::new(),
        });

        {
            let inner = Rc::clone(&inner);
            clear_btn.connect_clicked(move |_| clear_filter(&inner));
        }
        wire_tree_activation(&inner);

        let content = build_tree_scroller(&inner.tree);

        let sidebar = Self {
            header: title,
            content,
            footer: actions_row,
            inner,
        };
        sidebar.rebuild_picker_rows(active_fields);
        sidebar
    }

    /// The "Library" title label — the panel's header slot.
    pub fn header(&self) -> &Label {
        &self.header
    }

    /// The scrollable nav tree — the panel's content slot.
    pub fn content(&self) -> &ScrolledWindow {
        &self.content
    }

    /// The picker/clear actions strip — the panel's footer slot.
    pub fn footer(&self) -> &GtkBox {
        &self.footer
    }

    /// Registers the callback invoked whenever the user picks a filter.
    pub fn connect_filter_selected<F: Fn(LibraryFilter) + 'static>(&self, f: F) {
        self.inner.on_select.set(f);
    }

    /// Registers the callback invoked with the new active field list whenever
    /// the user enables or disables a filter category in the picker.
    pub fn connect_fields_changed<F: Fn(Vec<FilterField>) + 'static>(&self, f: F) {
        self.inner.on_fields_changed.set(f);
    }

    /// Replaces `field`'s tree rows with the given distinct values.
    pub fn populate(&self, field: FilterField, values: Vec<String>) {
        self.inner.tree.set_items(field.index(), values);
    }

    fn rebuild_picker_rows(&self, active_fields: &[FilterField]) {
        remove_all_children(&self.inner.picker_list);
        for field in FilterField::all() {
            let check = CheckButton::with_label(field.label());
            check.set_active(active_fields.contains(&field));
            let sidebar = self.clone();
            check.connect_toggled(move |cb| sidebar.set_field_active(field, cb.is_active()));
            self.inner.picker_list.append(&check);
        }
    }

    /// Shows or hides `field`'s tree row and notifies the change. Hiding
    /// removes the row (and, with it, any GTK-level selection inside it), so
    /// a filter the user can no longer see doesn't stay silently active in
    /// the background.
    fn set_field_active(&self, field: FilterField, active: bool) {
        self.inner.tree.set_category_visible(field.index(), active);
        if !active {
            let mut current = self.inner.current_selection.borrow_mut();
            if current.as_ref().is_some_and(|(f, _)| *f == field) {
                *current = None;
            }
        }
        self.inner
            .on_fields_changed
            .emit(self.inner.active_fields());
    }
}

/// The "Library" title label — clearing the filter is the footer's
/// "Clear filter" button's job now, not the title's.
fn build_library_label() -> Label {
    let title = Label::new(Some("Library"));
    title.set_xalign(0.0);
    style::add_class(&title, StyleClass::SectionName);
    title
}

/// The filter-picker menu button and the (initially empty) row container its
/// popover shows; `rebuild_picker_rows` fills the container.
fn build_filter_picker() -> (gtk4::MenuButton, GtkBox) {
    let picker_list = GtkBox::new(Orientation::Vertical, 4);
    style::set_margins(&picker_list, Margins::all(spacing::M));

    let picker_popover = Popover::new();
    picker_popover.set_child(Some(&picker_list));

    let picker_btn = flat_menu_button(AppIcon::ListAdd, "Choose filters");
    picker_btn.set_popover(Some(&picker_popover));
    (picker_btn, picker_list)
}

/// The actions strip at the bottom of the sidebar, below the nav tree, set
/// off by a separator above it.
fn build_actions_row(picker_btn: &gtk4::MenuButton, clear_btn: &Button) -> GtkBox {
    let actions_row = GtkBox::new(Orientation::Horizontal, 4);
    style::set_margins(&actions_row, Margins::all(spacing::S));
    actions_row.append(picker_btn);
    actions_row.append(clear_btn);
    actions_row
}

/// Routes a leaf activation to the filter logic: re-activating the current
/// selection clears it; anything else becomes the new filter. Selecting a
/// different leaf anywhere in the tree already deselects the previous one —
/// `GtkTreeSelection`'s native single-selection mode spans every category.
fn wire_tree_activation(inner: &Rc<SidebarInner>) {
    let inner_for_row = Rc::clone(inner);
    inner.tree.connect_activated(move |category_id, value| {
        let Some(field) = FilterField::all().get(category_id).copied() else {
            return;
        };
        let reselected_same_value = inner_for_row
            .current_selection
            .borrow()
            .as_ref()
            .is_some_and(|(f, v)| *f == field && *v == value);
        if reselected_same_value {
            clear_filter(&inner_for_row);
        } else {
            *inner_for_row.current_selection.borrow_mut() = Some((field, value.clone()));
            emit_filter(&inner_for_row, field.to_filter(&value));
        }
    });
}

/// The scrollable nav tree — indentation under the title and the top inset
/// are the surrounding `SidebarPanel`'s job.
fn build_tree_scroller(tree: &NavTree<String>) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(tree.widget()));
    scrolled
}

/// Deselects the tree, forgets the tracked selection, and shows the whole
/// library — shared by the clear-filter button and re-clicking an
/// already-selected row.
fn clear_filter(inner: &SidebarInner) {
    inner.tree.clear_selection();
    *inner.current_selection.borrow_mut() = None;
    emit_filter(inner, LibraryFilter::All);
}

fn emit_filter(inner: &SidebarInner, filter: LibraryFilter) {
    inner.on_select.emit(filter);
}
