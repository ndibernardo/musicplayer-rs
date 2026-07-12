use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Align;
use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::CheckButton;
use gtk4::Label;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::Popover;
use gtk4::ScrolledWindow;
use gtk4::prelude::*;

use crate::library::filter::FilterField;
use crate::library::filter::LibraryFilter;
use crate::ui::style;
use crate::ui::style::Margins;
use crate::ui::style::StyleClass;
use crate::ui::style::spacing;
use crate::ui::widgets::AppIcon;
use crate::ui::widgets::Callback;
use crate::ui::widgets::CollapsibleSection;
use crate::ui::widgets::ValueList;
use crate::ui::widgets::body_label;
use crate::ui::widgets::flat_icon_button;
use crate::ui::widgets::flat_menu_button;
use crate::ui::widgets::remove_all_children;

/// One browsable filter category: a collapsible list of `field`'s distinct
/// values.
struct Section {
    field: FilterField,
    list: ValueList<String>,
    section: CollapsibleSection,
}

impl Section {
    fn new(field: FilterField) -> Self {
        let list = ValueList::new(gtk4::SelectionMode::Single, |value: &String| {
            value_row(value)
        });
        let section = CollapsibleSection::new(field.label(), list.widget());
        // Hidden until the user enables this field in the filter picker.
        section.set_visible(false);
        Self {
            field,
            list,
            section,
        }
    }

    /// Replaces this section's rows with `values`, collapsing the section
    /// when it empties out.
    fn fill(&self, values: Vec<String>) {
        let empty = self.list.set_items(values);
        self.section.set_empty(empty);
    }
}

fn value_row(value: &str) -> ListBoxRow {
    let label = body_label(value);
    label.set_margin_start(spacing::M);
    let row = ListBoxRow::new();
    row.set_child(Some(&label));
    row
}

/// One [`Section`] per [`FilterField`], stored in [`FilterField::all`] order
/// and routed by [`FilterField::index`] — so adding a variant is caught by
/// that method's exhaustive match, in `library/filter.rs`, instead of four
/// separate spots in this file.
struct SectionMap([Rc<Section>; FilterField::COUNT]);

impl SectionMap {
    fn new() -> Self {
        Self(FilterField::all().map(|field| Rc::new(Section::new(field))))
    }

    fn get(&self, field: FilterField) -> &Rc<Section> {
        &self.0[field.index()]
    }

    fn iter(&self) -> impl Iterator<Item = &Rc<Section>> {
        self.0.iter()
    }
}

/// Genre / album artist / artist / year / composer browser, generalised over
/// every [`FilterField`] the user has enabled via the filter picker. Emits a
/// `LibraryFilter` when the user selects an entry, and the new active field
/// list whenever the picker's selection changes.
///
/// Exposes its three parts — title button, scrollable filter stack, actions
/// strip — separately, so the caller can assemble them into the shared
/// `SidebarPanel` skeleton alongside the right sidebar's parts.
#[derive(Clone)]
pub struct Sidebar {
    header: Button,
    content: ScrolledWindow,
    footer: GtkBox,
    inner: Rc<SidebarInner>,
}

struct SidebarInner {
    sections: SectionMap,
    picker_list: GtkBox,
    /// The field/value pair behind the currently selected row, if any —
    /// tracked separately from GTK's own selection state so a second click on
    /// the same row can be recognised as a request to deselect it.
    current_selection: RefCell<Option<(FilterField, String)>>,
    on_select: Callback<LibraryFilter>,
    on_fields_changed: Callback<Vec<FilterField>>,
}

impl SidebarInner {
    fn section(&self, field: FilterField) -> &Rc<Section> {
        self.sections.get(field)
    }

    fn active_fields(&self) -> Vec<FilterField> {
        FilterField::all()
            .into_iter()
            .filter(|f| self.section(*f).section.is_visible())
            .collect()
    }
}

impl Sidebar {
    pub fn new(active_fields: &[FilterField]) -> Self {
        let all_btn = build_library_button();
        let (picker_btn, picker_list) = build_filter_picker();
        let clear_btn = flat_icon_button(AppIcon::EditClear, "Clear filter");
        let actions_row = build_actions_row(&picker_btn, &clear_btn);

        let inner = Rc::new(SidebarInner {
            sections: SectionMap::new(),
            picker_list,
            current_selection: RefCell::new(None),
            on_select: Callback::new(),
            on_fields_changed: Callback::new(),
        });

        for field in active_fields {
            inner.section(*field).section.set_visible(true);
        }

        {
            let inner = Rc::clone(&inner);
            all_btn.connect_clicked(move |_| clear_filter(&inner));
        }
        {
            let inner = Rc::clone(&inner);
            clear_btn.connect_clicked(move |_| clear_filter(&inner));
        }
        wire_section_activation(&inner);

        let filters_scrolled = build_filters_scroller(&inner.sections);

        let sidebar = Self {
            header: all_btn,
            content: filters_scrolled,
            footer: actions_row,
            inner,
        };
        sidebar.rebuild_picker_rows(active_fields);
        sidebar
    }

    /// The "Library" title button — the panel's header slot.
    pub fn header(&self) -> &Button {
        &self.header
    }

    /// The scrollable filter-section stack — the panel's content slot.
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

    /// Replaces `field`'s section entries with the given distinct values.
    pub fn populate(&self, field: FilterField, values: Vec<String>) {
        self.inner.section(field).fill(values);
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

    /// Shows or hides `field`'s section and notifies the change. Hiding also
    /// clears its selection, since a filter the user can no longer see
    /// shouldn't stay silently active in the background.
    fn set_field_active(&self, field: FilterField, active: bool) {
        let section = self.inner.section(field);
        section.section.set_visible(active);
        if !active {
            section.list.clear_selection();
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

/// The "Library" title button: clears the filter, and doubles as the
/// sidebar's title via `StyleClass::SidebarTitle`.
fn build_library_button() -> Button {
    let all_label = Label::new(Some("Library"));
    all_label.set_xalign(0.0);
    style::add_class(&all_label, StyleClass::SidebarTitle);
    let all_btn = Button::new();
    all_btn.set_child(Some(&all_label));
    all_btn.add_css_class("flat");
    all_btn.set_hexpand(true);
    all_btn.set_halign(Align::Start);
    style::set_margins(&all_btn, Margins::none().start(spacing::S).top(spacing::S));
    all_btn
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

/// The actions strip at the bottom of the sidebar, below the filter list,
/// set off by a separator above it.
fn build_actions_row(picker_btn: &gtk4::MenuButton, clear_btn: &Button) -> GtkBox {
    let actions_row = GtkBox::new(Orientation::Horizontal, 4);
    style::set_margins(&actions_row, Margins::all(spacing::S));
    actions_row.append(picker_btn);
    actions_row.append(clear_btn);
    actions_row
}

/// Routes a row activation in any section to the filter logic: re-activating
/// the current selection clears it; anything else becomes the new filter.
fn wire_section_activation(inner: &Rc<SidebarInner>) {
    for field in FilterField::all() {
        let inner_for_row = Rc::clone(inner);
        inner
            .section(field)
            .list
            .connect_activated(move |_, value| {
                let reselected_same_value = inner_for_row
                    .current_selection
                    .borrow()
                    .as_ref()
                    .is_some_and(|(f, v)| *f == field && *v == value);
                if reselected_same_value {
                    clear_filter(&inner_for_row);
                } else {
                    clear_other_selections(&inner_for_row, field);
                    *inner_for_row.current_selection.borrow_mut() = Some((field, value.clone()));
                    emit_filter(&inner_for_row, field.to_filter(&value));
                }
            });
    }
}

/// The scrollable filter-section stack. Indentation under the title and the
/// top inset are the surrounding `SidebarPanel`'s job; each `Section`'s own
/// bottom margin sets them off from each other.
fn build_filters_scroller(sections: &SectionMap) -> ScrolledWindow {
    let filters_box = GtkBox::new(Orientation::Vertical, 0);
    for section in sections.iter() {
        filters_box.append(section.section.widget());
    }
    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&filters_box));
    scrolled
}

/// Deselects every section's list — keeps at most one selected across the
/// whole sidebar.
fn clear_all_selections(inner: &SidebarInner) {
    for section in inner.sections.iter() {
        section.list.clear_selection();
    }
}

/// Deselects every section's list except `except`'s own — a newly activated
/// row's list handles its own single-selection semantics natively.
fn clear_other_selections(inner: &SidebarInner, except: FilterField) {
    for section in inner.sections.iter() {
        if section.field != except {
            section.list.clear_selection();
        }
    }
}

/// Deselects every section, forgets the tracked selection, and shows the
/// whole library — shared by the Library button, the clear-filter button,
/// and re-clicking an already-selected row.
fn clear_filter(inner: &SidebarInner) {
    clear_all_selections(inner);
    *inner.current_selection.borrow_mut() = None;
    emit_filter(inner, LibraryFilter::All);
}

fn emit_filter(inner: &SidebarInner, filter: LibraryFilter) {
    inner.on_select.emit(filter);
}
