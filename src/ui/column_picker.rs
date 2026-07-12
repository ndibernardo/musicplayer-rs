use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::CheckButton;
use gtk4::DragSource;
use gtk4::DropTarget;
use gtk4::Image;
use gtk4::MenuButton;
use gtk4::Orientation;
use gtk4::Popover;
use gtk4::prelude::*;

use crate::library::column::ColumnConfig;
use crate::library::column::ColumnPrefs;
use crate::library::format::TrackField;
use crate::ui::widgets::AppIcon;
use crate::ui::widgets::Callback;
use crate::ui::widgets::remove_all_children;

struct Inner {
    list_box: GtkBox,
    state: RefCell<ColumnPrefs>,
    on_change: Callback<ColumnPrefs>,
}

/// A header-bar button whose popover lists every [`TrackField`] with a
/// visibility checkbox; visible fields can also be drag-and-dropped onto one
/// another to reorder them. Every toggle or reorder rebuilds the row list
/// (cheap at ten rows) and calls the registered callback with the new
/// [`ColumnPrefs`].
#[derive(Clone)]
pub struct ColumnPicker {
    pub widget: MenuButton,
    inner: Rc<Inner>,
}

impl ColumnPicker {
    pub fn new(initial: ColumnPrefs) -> Self {
        let list_box = GtkBox::new(Orientation::Vertical, 4);
        list_box.set_margin_top(8);
        list_box.set_margin_bottom(8);
        list_box.set_margin_start(8);
        list_box.set_margin_end(8);

        let popover = Popover::new();
        popover.set_child(Some(&list_box));

        let widget = MenuButton::new();
        widget.set_icon_name(AppIcon::OpenMenu.name());
        widget.set_tooltip_text(Some("Choose columns"));
        widget.set_popover(Some(&popover));

        let picker = Self {
            widget,
            inner: Rc::new(Inner {
                list_box,
                state: RefCell::new(initial),
                on_change: Callback::new(),
            }),
        };
        picker.rebuild_rows();
        picker
    }

    /// Registers the callback invoked with the new prefs after every change.
    pub fn connect_changed<F: Fn(ColumnPrefs) + 'static>(&self, f: F) {
        self.inner.on_change.set(f);
    }

    fn rebuild_rows(&self) {
        remove_all_children(&self.inner.list_box);
        let visible: Vec<TrackField> = self
            .inner
            .state
            .borrow()
            .columns()
            .iter()
            .map(|c| c.field)
            .collect();
        let hidden: Vec<TrackField> = TrackField::all()
            .into_iter()
            .filter(|f| !visible.contains(f))
            .collect();

        for field in visible {
            self.inner.list_box.append(&self.build_row(field, true));
        }
        for field in hidden {
            self.inner.list_box.append(&self.build_row(field, false));
        }
    }

    /// Builds one picker row. Only visible rows are drag sources/drop
    /// targets — reordering only makes sense among fields already shown.
    fn build_row(&self, field: TrackField, visible: bool) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 6);

        let check = CheckButton::with_label(field.label());
        check.set_active(visible);
        check.set_hexpand(true);
        let picker = self.clone();
        check.connect_toggled(move |cb| picker.set_field_visible(field, cb.is_active()));
        row.append(&check);

        if visible {
            let handle = Image::from_icon_name(AppIcon::ListDragHandle.name());
            handle.set_tooltip_text(Some("Drag to reorder"));
            row.append(&handle);

            let drag_source = DragSource::new();
            drag_source.set_actions(gtk4::gdk::DragAction::MOVE);
            drag_source.connect_prepare(move |_, _, _| {
                Some(gtk4::gdk::ContentProvider::for_value(
                    &field.as_key().to_value(),
                ))
            });
            row.add_controller(drag_source);

            let drop_target = DropTarget::new(String::static_type(), gtk4::gdk::DragAction::MOVE);
            let picker = self.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                let Ok(key) = value.get::<String>() else {
                    return false;
                };
                let Some(dragged) = TrackField::from_key(&key) else {
                    return false;
                };
                picker.move_field_before(dragged, field);
                true
            });
            row.add_controller(drop_target);
        }

        row
    }

    /// Shows or hides `field`. Showing appends it (with its default format)
    /// to the end of the visible order; hiding removes it — any custom
    /// format it had is not preserved, since this pass doesn't expose format
    /// editing in the UI.
    fn set_field_visible(&self, field: TrackField, visible: bool) {
        let mut columns: Vec<ColumnConfig> = self.inner.state.borrow().columns().to_vec();
        if visible {
            if !columns.iter().any(|c| c.field == field) {
                columns.push(ColumnConfig::default_for(field));
            }
        } else {
            columns.retain(|c| c.field != field);
        }
        self.replace_state(ColumnPrefs::new(columns));
    }

    /// Moves `dragged` to sit immediately before `target` in the visible
    /// order — the drop-onto-a-row reorder gesture. A no-op when `dragged`
    /// and `target` are the same field, or when `dragged` isn't visible.
    fn move_field_before(&self, dragged: TrackField, target: TrackField) {
        if dragged == target {
            return;
        }
        let mut columns: Vec<ColumnConfig> = self.inner.state.borrow().columns().to_vec();
        let Some(from) = columns.iter().position(|c| c.field == dragged) else {
            return;
        };
        let moved = columns.remove(from);
        let to = columns
            .iter()
            .position(|c| c.field == target)
            .unwrap_or(columns.len());
        columns.insert(to, moved);
        self.replace_state(ColumnPrefs::new(columns));
    }

    fn replace_state(&self, prefs: ColumnPrefs) {
        *self.inner.state.borrow_mut() = prefs;
        self.rebuild_rows();
        self.inner.on_change.emit(self.inner.state.borrow().clone());
    }
}
