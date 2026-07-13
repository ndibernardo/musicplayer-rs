// GTK deprecated `TreeView`/`TreeStore` in 4.10 in favor of `ListView` +
// `TreeListModel`, but the newer stack has no cell-based text rendering
// suited to a compact, two-level nav tree without hand-building a custom
// factory widget per row. Deprecated is accepted here deliberately, isolated
// to this one file.
#![allow(deprecated)]

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Button;
use gtk4::CellRendererText;
use gtk4::GestureClick;
use gtk4::Popover;
use gtk4::SelectionMode;
use gtk4::TreeIter;
use gtk4::TreeStore;
use gtk4::TreeView;
use gtk4::TreeViewColumn;
use gtk4::gdk::Rectangle;
use gtk4::glib::Type;
use gtk4::pango;
use gtk4::prelude::*;

use crate::ui::style;
use crate::ui::style::StyleClass;
use crate::ui::widgets::callback::Callback;

const COL_LABEL: u32 = 0;
const COL_IS_CATEGORY: u32 = 1;
const COL_CATEGORY_ID: u32 = 2;
const COL_ITEM_INDEX: u32 = 3;

/// Renders a leaf value as the text shown in its tree row.
pub type RenderFn<T> = Rc<dyn Fn(&T) -> String>;

/// One top-level row in a [`NavTree`] and the leaf values listed under it.
struct CategorySlot<T> {
    category_id: usize,
    label: &'static str,
    render: RenderFn<T>,
    /// Cached even while hidden, so a later `set_category_visible(id, true)`
    /// replays them without the caller having to re-fetch anything.
    items: Vec<T>,
    /// `None` while this category is hidden — its row isn't in the store.
    iter: Option<TreeIter>,
}

/// A Builder-style navigation tree: a fixed, ordered set of expandable
/// top-level categories, each holding zero or more leaf values. Backs both
/// sidebar panels — the left one (one category per `FilterField`) and the
/// right one (a single "Watched Folders" category) — with one widget
/// vocabulary instead of a hand-rolled `Expander` + `ListBox` per section.
pub struct NavTree<T> {
    view: TreeView,
    store: TreeStore,
    categories: Rc<RefCell<Vec<CategorySlot<T>>>>,
    on_activate: Rc<Callback<(usize, T)>>,
    on_right_click: Rc<Callback<(usize, T, f64, f64)>>,
}

impl<T: Clone + 'static> NavTree<T> {
    /// Builds the tree with one category per `(label, render)` pair, in the
    /// order given — that order is each category's permanent `category_id`
    /// (its index into `categories`), stable no matter which are currently
    /// visible. Every category starts hidden; the caller calls
    /// [`Self::set_category_visible`] for whichever should show up front.
    pub fn new(
        mode: SelectionMode,
        ellipsize: pango::EllipsizeMode,
        categories: Vec<(&'static str, RenderFn<T>)>,
    ) -> Self {
        let store = TreeStore::new(&[Type::STRING, Type::BOOL, Type::U32, Type::I32]);

        let view = TreeView::with_model(&store);
        view.set_headers_visible(false);
        view.set_enable_tree_lines(false);
        view.set_activate_on_single_click(true);
        view.selection().set_mode(mode);
        style::add_class(&view, StyleClass::SidebarTree);

        let renderer = CellRendererText::new();
        renderer.set_property("ellipsize", ellipsize);
        let column = TreeViewColumn::new();
        column.pack_start(&renderer, true);
        column.add_attribute(&renderer, "text", COL_LABEL as i32);
        // Category rows read as section headers; leaf rows stay regular
        // weight, matching the bold `.app-section-name` look this tree
        // replaces.
        column.set_cell_data_func(&renderer, move |_col, cell, model, iter| {
            let Some(cell) = cell.downcast_ref::<CellRendererText>() else {
                return;
            };
            let is_category: bool = model.get(iter, COL_IS_CATEGORY as i32);
            cell.set_property("weight", if is_category { 700 } else { 400 });
        });
        view.append_column(&column);

        let slots = categories
            .into_iter()
            .enumerate()
            .map(|(category_id, (label, render))| CategorySlot {
                category_id,
                label,
                render,
                items: Vec::new(),
                iter: None,
            })
            .collect();
        let categories = Rc::new(RefCell::new(slots));
        let on_activate: Rc<Callback<(usize, T)>> = Rc::new(Callback::new());
        let on_right_click: Rc<Callback<(usize, T, f64, f64)>> = Rc::new(Callback::new());

        wire_activation(&view, &categories, &on_activate);
        wire_right_click(&view, &categories, &on_right_click);

        Self {
            view,
            store,
            categories,
            on_activate,
            on_right_click,
        }
    }

    pub fn widget(&self) -> &TreeView {
        &self.view
    }

    /// Replaces `category_id`'s leaf rows with `items`, re-rendering its
    /// children immediately if the category is currently visible.
    pub fn set_items(&self, category_id: usize, items: Vec<T>) {
        let mut categories = self.categories.borrow_mut();
        categories[category_id].items = items;
        if let Some(parent) = categories[category_id].iter {
            self.rebuild_children(&parent, &categories[category_id]);
        }
    }

    /// Shows or hides `category_id`'s row (and, while shown, its children).
    pub fn set_category_visible(&self, category_id: usize, visible: bool) {
        if visible {
            self.show_category(category_id);
        } else {
            self.hide_category(category_id);
        }
    }

    pub fn is_category_visible(&self, category_id: usize) -> bool {
        self.categories.borrow()[category_id].iter.is_some()
    }

    /// Registers the callback invoked with a leaf row's category and value
    /// when the user activates it. Category rows only expand/collapse.
    pub fn connect_activated(&self, f: impl Fn(usize, T) + 'static) {
        self.on_activate
            .set(move |(category_id, item)| f(category_id, item));
    }

    /// Registers the callback invoked with a leaf row's category, value, and
    /// pointer coordinates when the user right-clicks it.
    pub fn connect_row_right_clicked(&self, f: impl Fn(usize, T, f64, f64) + 'static) {
        self.on_right_click
            .set(move |(category_id, item, x, y)| f(category_id, item, x, y));
    }

    pub fn clear_selection(&self) {
        self.view.selection().unselect_all();
    }

    fn show_category(&self, category_id: usize) {
        let mut categories = self.categories.borrow_mut();
        if categories[category_id].iter.is_some() {
            return;
        }
        let next_visible = categories[category_id + 1..]
            .iter()
            .find_map(|slot| slot.iter);
        let iter = match &next_visible {
            Some(sibling) => self.store.insert_before(None, Some(sibling)),
            None => self.store.append(None),
        };
        let label = categories[category_id].label;
        let category_id_u32 = category_id as u32;
        self.store.set_value(&iter, COL_LABEL, &label.to_value());
        self.store
            .set_value(&iter, COL_IS_CATEGORY, &true.to_value());
        self.store
            .set_value(&iter, COL_CATEGORY_ID, &category_id_u32.to_value());
        self.store
            .set_value(&iter, COL_ITEM_INDEX, &(-1i32).to_value());
        categories[category_id].iter = Some(iter);
        self.rebuild_children(&iter, &categories[category_id]);
    }

    fn hide_category(&self, category_id: usize) {
        let mut categories = self.categories.borrow_mut();
        if let Some(iter) = categories[category_id].iter.take() {
            self.store.remove(&iter);
        }
    }

    fn rebuild_children(&self, parent: &TreeIter, slot: &CategorySlot<T>) {
        while let Some(child) = self.store.iter_children(Some(parent)) {
            self.store.remove(&child);
        }
        for (item_index, item) in slot.items.iter().enumerate() {
            let child = self.store.append(Some(parent));
            let text = (slot.render)(item);
            let category_id = slot.category_id as u32;
            let item_index = item_index as i32;
            self.store.set_value(&child, COL_LABEL, &text.to_value());
            self.store
                .set_value(&child, COL_IS_CATEGORY, &false.to_value());
            self.store
                .set_value(&child, COL_CATEGORY_ID, &category_id.to_value());
            self.store
                .set_value(&child, COL_ITEM_INDEX, &item_index.to_value());
        }
    }
}

/// Reads the leaf value at `iter`, or `None` when `iter` names a category row.
fn leaf_at<T: Clone>(
    model: &impl IsA<gtk4::TreeModel>,
    iter: &TreeIter,
    categories: &Rc<RefCell<Vec<CategorySlot<T>>>>,
) -> Option<(usize, T)> {
    let model = model.as_ref();
    let is_category: bool = model.get(iter, COL_IS_CATEGORY as i32);
    if is_category {
        return None;
    }
    let category_id: u32 = model.get(iter, COL_CATEGORY_ID as i32);
    let item_index: i32 = model.get(iter, COL_ITEM_INDEX as i32);
    let category_id = category_id as usize;
    let item_index = item_index as usize;
    categories
        .borrow()
        .get(category_id)?
        .items
        .get(item_index)
        .cloned()
        .map(|item| (category_id, item))
}

fn wire_activation<T: Clone + 'static>(
    view: &TreeView,
    categories: &Rc<RefCell<Vec<CategorySlot<T>>>>,
    on_activate: &Rc<Callback<(usize, T)>>,
) {
    let categories = Rc::clone(categories);
    let on_activate = Rc::clone(on_activate);
    view.connect_row_activated(move |view, path, _column| {
        let Some(model) = view.model() else {
            return;
        };
        let Some(iter) = model.iter(path) else {
            return;
        };
        let is_category: bool = model.get(&iter, COL_IS_CATEGORY as i32);
        if is_category {
            // Native expander hit target is the small triangle only. Widen
            // it to the whole row so a click anywhere toggles the section.
            if view.row_expanded(path) {
                view.collapse_row(path);
            } else {
                view.expand_row(path, false);
            }
            return;
        }
        if let Some((category_id, item)) = leaf_at(&model, &iter, &categories) {
            on_activate.emit((category_id, item));
        }
    });
}

fn wire_right_click<T: Clone + 'static>(
    view: &TreeView,
    categories: &Rc<RefCell<Vec<CategorySlot<T>>>>,
    on_right_click: &Rc<Callback<(usize, T, f64, f64)>>,
) {
    let view_for_gesture = view.clone();
    let categories = Rc::clone(categories);
    let on_right_click = Rc::clone(on_right_click);
    let gesture = GestureClick::new();
    gesture.set_button(3);
    gesture.connect_pressed(move |_gesture, _n_press, x, y| {
        let Some((Some(path), ..)) = view_for_gesture.path_at_pos(x as i32, y as i32) else {
            return;
        };
        let Some(model) = view_for_gesture.model() else {
            return;
        };
        let Some(iter) = model.iter(&path) else {
            return;
        };
        if let Some((category_id, item)) = leaf_at(&model, &iter, &categories) {
            on_right_click.emit((category_id, item, x, y));
        }
    });
    view.add_controller(gesture);
}

/// A popover with a single "Remove" action, opened at `(x, y)` — the one
/// interaction folder rows need that a `GtkTreeView` cell can't host inline.
pub fn show_remove_popover(
    parent: &impl IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    on_remove: impl Fn() + 'static,
) {
    let popover = Popover::new();
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&Rectangle::new(x as i32, y as i32, 1, 1)));

    let remove_btn = Button::with_label("Remove folder");
    remove_btn.add_css_class("flat");
    popover.set_child(Some(&remove_btn));

    let popover_for_click = popover.clone();
    remove_btn.connect_clicked(move |_| {
        on_remove();
        popover_for_click.popdown();
    });
    let popover_for_close = popover.clone();
    popover.connect_closed(move |_| {
        popover_for_close.unparent();
    });

    popover.popup();
}
