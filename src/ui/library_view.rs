use std::cell::RefCell;
use std::rc::Rc;

use glib::BoxedAnyObject;
use gtk4::ColumnView;
use gtk4::ColumnViewColumn;
use gtk4::GestureClick;
use gtk4::Label;
use gtk4::ListItem;
use gtk4::MultiSelection;
use gtk4::ScrolledWindow;
use gtk4::SignalListItemFactory;
use gtk4::Widget;
use gtk4::gio::ListStore;
use gtk4::prelude::*;

use crate::library::column::ColumnPrefs;
use crate::library::format;
use crate::library::format::TrackField;
use crate::library::track::Track;
use crate::ui::context_menu;
use crate::ui::context_menu::show_context_menu;
use crate::ui::widgets::Callback;

#[derive(Clone)]
pub struct LibraryView {
    pub widget: ScrolledWindow,
    column_view: ColumnView,
    store: ListStore,
    selection: MultiSelection,
    on_enqueue: Rc<Callback<Vec<Track>>>,
    on_edit_requested: Rc<Callback<Vec<Track>>>,
    /// Fired with a field and its new fixed width (px) after a header drag.
    on_column_resized: Rc<Callback<(TrackField, i32)>>,
    /// The columns currently appended to `column_view`, so a later
    /// `set_column_prefs` call knows what to remove before rebuilding.
    columns: Rc<RefCell<Vec<ColumnViewColumn>>>,
}

impl LibraryView {
    pub fn new(prefs: &ColumnPrefs) -> Self {
        let store = ListStore::new::<BoxedAnyObject>();
        // Wrapping the store in a SortListModel is what makes column headers
        // clickable sort controls, once each column has a sorter and this
        // model's sorter is bound to the view's aggregate one below.
        let sort_model = gtk4::SortListModel::new(Some(store.clone()), None::<gtk4::Sorter>);
        // A real selection model (rather than `NoSelection`) is what gives
        // ctrl/shift-click multi-select for free — GTK's ColumnView handles
        // the click/modifier logic internally once it has one to update.
        let selection = MultiSelection::new(Some(sort_model.clone()));
        let column_view = ColumnView::new(Some(selection.clone()));
        column_view.set_show_row_separators(true);
        column_view.set_hexpand(true);
        column_view.set_vexpand(true);

        if let Some(sorter) = column_view.sorter() {
            sort_model.set_sorter(Some(&sorter));
        }
        // GTK's ColumnView natively toggles a clicked header between
        // ascending and descending forever; this adds the one behaviour it
        // doesn't have — a third click clears the sort entirely.
        wire_tri_state_sort(&column_view);

        let scrolled = ScrolledWindow::new();
        scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&column_view));

        let view = Self {
            widget: scrolled,
            column_view,
            store,
            selection,
            on_enqueue: Rc::new(Callback::new()),
            on_edit_requested: Rc::new(Callback::new()),
            on_column_resized: Rc::new(Callback::new()),
            columns: Rc::new(RefCell::new(Vec::new())),
        };
        view.set_column_prefs(prefs);
        view
    }

    /// Rebuilds the column list from `prefs`: removes every column currently
    /// shown and appends fresh ones in the given order, each rendering its
    /// configured format string.
    pub fn set_column_prefs(&self, prefs: &ColumnPrefs) {
        let mut columns = self.columns.borrow_mut();
        for column in columns.drain(..) {
            self.column_view.remove_column(&column);
        }
        let last_index = prefs.columns().len().saturating_sub(1);
        for (index, config) in prefs.columns().iter().enumerate() {
            let format_expr = config.format.clone();
            let column = text_column(
                config.field.label(),
                move |t| format::render(&format_expr, t),
                &self.selection,
                Rc::clone(&self.on_enqueue),
                Rc::clone(&self.on_edit_requested),
            );
            // Exactly one column — the *last* in the user's order — expands
            // to absorb whatever width the others don't use, so the table
            // always fills the window's width, the way music-player track
            // lists conventionally behave. It must be the last column, not
            // the first: every resizable column sits to the expand column's
            // left, so growing/shrinking one only ever pushes into the
            // filler's space to its right — nothing to its own left ever
            // has to shift. A leading filler instead has every dragged
            // column's compensation happen far to the left of it, which
            // visually reads as the *wrong* edge of the dragged column
            // moving. Giving more than one column `expand` has the same
            // problem in miniature: a dragged fixed-width fights the
            // expand allocation on every relayout and snaps back to nothing.
            column.set_expand(index == last_index);
            column.set_sorter(Some(&field_sorter(config.field)));
            column.set_resizable(true);
            // Restore a persisted width *before* wiring the notify handler
            // below, so applying it here doesn't get mistaken for a user
            // drag and re-persisted right back.
            if let Some(width) = config.width {
                column.set_fixed_width(width);
            }
            let field = config.field;
            let on_column_resized = Rc::clone(&self.on_column_resized);
            column.connect_fixed_width_notify(move |column| {
                on_column_resized.emit((field, column.fixed_width()));
            });
            self.column_view.append_column(&column);
            columns.push(column);
        }
    }

    /// Replaces every row in one `splice` call rather than an `append` loop —
    /// one `items-changed` emission instead of one per track, which matters
    /// once the library runs into the tens of thousands.
    pub fn set_tracks(&self, tracks: Vec<Track>) {
        let removed = self.store.n_items();
        let added: Vec<BoxedAnyObject> = tracks.into_iter().map(BoxedAnyObject::new).collect();
        self.store.splice(0, removed, &added);
    }

    /// Clears the current selection. Call when the user clicks elsewhere in
    /// the application, so a multi-selection doesn't linger indefinitely.
    pub fn clear_selection(&self) {
        self.selection.unselect_all();
    }

    /// Calls `f` with the full visible track list and the index of the
    /// double-clicked row, so the caller can enqueue the list from that track.
    /// Reads from `selection` (the sorted model), not the raw store —
    /// `position` refers to the sorted order the user actually clicked on.
    pub fn connect_track_activated<F: Fn(Vec<Track>, usize) + 'static>(&self, f: F) {
        let selection = self.selection.clone();
        self.column_view.connect_activate(move |_, position| {
            f(collect_tracks(&selection), position as usize);
        });
    }

    /// Registers the callback invoked when "Add to Queue" (or, for a
    /// multi-selected row, "Add N to Queue") is chosen from a row's
    /// right-click menu — the selected tracks, one element for the singular
    /// case.
    pub fn connect_track_enqueue<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        self.on_enqueue.set(f);
    }

    /// Registers the callback invoked when "Edit Track…" (or, for a
    /// multi-selected row, "Edit N Tracks…") is chosen from a row's
    /// right-click menu — the selected tracks, one element for the singular
    /// case.
    pub fn connect_track_edit_requested<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        self.on_edit_requested.set(f);
    }

    /// Registers the callback invoked with a field and its new fixed width
    /// after the user drags that column's header to resize it.
    pub fn connect_column_resized<F: Fn(TrackField, i32) + 'static>(&self, f: F) {
        self.on_column_resized
            .set(move |(field, width)| f(field, width));
    }
}

/// Snapshots a list model's tracks in its current order.
fn collect_tracks(model: &impl IsA<gtk4::gio::ListModel>) -> Vec<Track> {
    (0..model.n_items())
        .filter_map(|i| model.item(i).and_downcast::<BoxedAnyObject>())
        .map(|obj| obj.borrow::<Track>().clone())
        .collect()
}

/// Every currently selected track, in display order. `MultiSelection` is
/// itself a `ListModel` over the same items as the store, so no separate
/// store reference is needed here.
fn selected_tracks(selection: &MultiSelection) -> Vec<Track> {
    (0..selection.n_items())
        .filter(|&i| selection.is_selected(i))
        .filter_map(|i| selection.item(i).and_downcast::<BoxedAnyObject>())
        .map(|obj| obj.borrow::<Track>().clone())
        .collect()
}

fn text_column<F>(
    title: &str,
    extract: F,
    selection: &MultiSelection,
    on_enqueue: Rc<Callback<Vec<Track>>>,
    on_edit: Rc<Callback<Vec<Track>>>,
) -> ColumnViewColumn
where
    F: Fn(&Track) -> String + 'static,
{
    let selection = selection.clone();
    let factory = SignalListItemFactory::new();
    factory.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<ListItem>() else {
            return;
        };
        let label = Label::new(None);
        label.set_xalign(0.0);
        label.set_margin_start(6);
        label.set_margin_end(6);

        // Right-click offers "Add to Queue"/"Edit Track…" for whichever track
        // is currently bound to this row, or — if the row is part of a wider
        // multi-selection — batch actions for every selected track instead.
        // `item` is weakly held since it strongly owns `label` as its child,
        // which would otherwise cycle.
        let gesture = GestureClick::new();
        gesture.set_button(gtk4::gdk::BUTTON_SECONDARY);
        let item_weak = item.downgrade();
        let selection = selection.clone();
        let on_enqueue = Rc::clone(&on_enqueue);
        let on_edit = Rc::clone(&on_edit);
        let label_widget = label.clone().upcast::<Widget>();
        gesture.connect_pressed(move |_, _, x, y| {
            let Some(item) = item_weak.upgrade() else {
                return;
            };
            let Some(data) = item.item().and_downcast::<BoxedAnyObject>() else {
                return;
            };
            let track = data.borrow::<Track>().clone();
            let position = item.position();
            let batch = if selection.is_selected(position) && selection.selection().size() > 1 {
                Some(selected_tracks(&selection))
            } else {
                // Right-clicking outside the current selection collapses it
                // to just this row — standard file-manager convention.
                selection.select_item(position, true);
                None
            };
            let actions =
                context_menu::track_actions(&track, batch, on_enqueue.handler(), on_edit.handler());
            show_context_menu(&label_widget, x, y, actions);
        });
        label.add_controller(gesture);

        item.set_child(Some(&label));
    });
    factory.connect_bind(move |_, obj| {
        let Some(item) = obj.downcast_ref::<ListItem>() else {
            return;
        };
        let Some(data) = item.item().and_downcast::<BoxedAnyObject>() else {
            return;
        };
        let track = data.borrow::<Track>();
        let Some(label) = item.child().and_downcast::<Label>() else {
            return;
        };
        label.set_text(&extract(&track));
    });
    ColumnViewColumn::new(Some(title), Some(factory))
}

/// A `CustomSorter` comparing rows by `field`'s typed value (see
/// `TrackField::compare`), not by the rendered text.
fn field_sorter(field: TrackField) -> gtk4::CustomSorter {
    gtk4::CustomSorter::new(move |a, b| {
        let (Some(a), Some(b)) = (
            a.downcast_ref::<BoxedAnyObject>(),
            b.downcast_ref::<BoxedAnyObject>(),
        ) else {
            return gtk4::Ordering::Equal;
        };
        field
            .compare(&a.borrow::<Track>(), &b.borrow::<Track>())
            .into()
    })
}

/// Watches `column_view`'s aggregate sorter for the state GTK's own header
/// click handling would otherwise cycle forever (ascending ↔ descending) and
/// clears it instead the moment a click would wrap a descending column back
/// to ascending — turning the native two-state toggle into three states.
fn wire_tri_state_sort(column_view: &ColumnView) {
    let Some(sorter) = column_view.sorter() else {
        return;
    };
    let Ok(sorter) = sorter.downcast::<gtk4::ColumnViewSorter>() else {
        return;
    };
    let last: Rc<RefCell<Option<(ColumnViewColumn, gtk4::SortType)>>> = Rc::new(RefCell::new(None));

    let cv = column_view.clone();
    let last_clone = Rc::clone(&last);
    sorter.connect_primary_sort_order_notify(move |sorter| {
        on_sort_change(sorter, &cv, &last_clone);
    });

    let cv = column_view.clone();
    sorter.connect_primary_sort_column_notify(move |sorter| {
        on_sort_change(sorter, &cv, &last);
    });
}

fn on_sort_change(
    sorter: &gtk4::ColumnViewSorter,
    column_view: &ColumnView,
    last: &Rc<RefCell<Option<(ColumnViewColumn, gtk4::SortType)>>>,
) {
    let new_column = sorter.primary_sort_column();
    let new_order = sorter.primary_sort_order();

    let wraps_to_none = matches!(
        (&*last.borrow(), &new_column),
        (Some((prev_column, gtk4::SortType::Descending)), Some(current))
            if prev_column == current
    ) && new_order == gtk4::SortType::Ascending;

    if wraps_to_none {
        *last.borrow_mut() = None;
        column_view.sort_by_column(None::<&ColumnViewColumn>, gtk4::SortType::Ascending);
    } else {
        *last.borrow_mut() = new_column.map(|c| (c, new_order));
    }
}
