use gtk4::Box as GtkBox;
use gtk4::ListBox;
use gtk4::prelude::*;

/// Removes every child of `container`. Monomorphic per container type since
/// GTK4 exposes `remove` as an inherent method, not through a shared trait.
pub fn remove_all_children(container: &GtkBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// Removes every row of `list` — see [`remove_all_children`].
pub fn remove_all_rows(list: &ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}
