use std::cell::RefCell;
use std::rc::Rc;

use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::SelectionMode;
use gtk4::prelude::*;

use crate::ui::style;
use crate::ui::style::StyleClass;
use crate::ui::widgets::callback::Callback;
use crate::ui::widgets::containers::remove_all_rows;

/// A flat, sidebar-styled `ListBox` over domain values. Rows mirror `items`
/// index-for-index — the invariant every hand-rolled section list maintained
/// separately — so an activation hands back the value, not a bare row index
/// the caller has to resolve.
pub struct ValueList<T> {
    list: ListBox,
    items: Rc<RefCell<Vec<T>>>,
    render: Rc<dyn Fn(&T) -> ListBoxRow>,
    on_activate: Rc<Callback<(usize, T)>>,
}

impl<T: Clone + 'static> ValueList<T> {
    pub fn new(mode: SelectionMode, render: impl Fn(&T) -> ListBoxRow + 'static) -> Self {
        let list = ListBox::new();
        list.set_selection_mode(mode);
        list.set_activate_on_single_click(true);
        style::add_class(&list, StyleClass::SectionList);

        let items: Rc<RefCell<Vec<T>>> = Rc::new(RefCell::new(Vec::new()));
        let on_activate: Rc<Callback<(usize, T)>> = Rc::new(Callback::new());
        {
            let items = Rc::clone(&items);
            let on_activate = Rc::clone(&on_activate);
            list.connect_row_activated(move |_, row| {
                let index = row.index() as usize;
                let item = items.borrow().get(index).cloned();
                if let Some(item) = item {
                    on_activate.emit((index, item));
                }
            });
        }

        Self {
            list,
            items,
            render: Rc::new(render),
            on_activate,
        }
    }

    pub fn widget(&self) -> &ListBox {
        &self.list
    }

    /// Replaces the visible items, rebuilding every row. Returns `true` when
    /// the list is now empty, so the caller can react (e.g. clear a
    /// highlight).
    pub fn set_items(&self, new_items: Vec<T>) -> bool {
        remove_all_rows(&self.list);
        let empty = new_items.is_empty();
        for item in &new_items {
            self.list.append(&(self.render)(item));
        }
        *self.items.borrow_mut() = new_items;
        empty
    }

    /// Registers the callback invoked with an activated row's index and value.
    pub fn connect_activated(&self, f: impl Fn(usize, T) + 'static) {
        self.on_activate.set(move |(index, item)| f(index, item));
    }

    /// Selects the first row whose value satisfies `pred`, or clears the
    /// selection when none does.
    pub fn select_where(&self, pred: impl Fn(&T) -> bool) {
        let index = self.items.borrow().iter().position(pred);
        match index.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
    }

    pub fn clear_selection(&self) {
        self.list.unselect_all();
    }
}

// Manual impl: `derive(Clone)` would demand `T: Clone` even though every
// field is already reference-counted.
impl<T> Clone for ValueList<T> {
    fn clone(&self) -> Self {
        Self {
            list: self.list.clone(),
            items: Rc::clone(&self.items),
            render: Rc::clone(&self.render),
            on_activate: Rc::clone(&self.on_activate),
        }
    }
}
