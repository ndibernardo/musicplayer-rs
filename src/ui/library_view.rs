use std::cell::OnceCell;
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

use crate::library::track::Track;
use crate::ui::context_menu::ContextAction;
use crate::ui::context_menu::show_context_menu;
use crate::ui::format::format_duration;

/// Invoked with a single track chosen from a row's "add to queue" context menu.
type SingleTrackCallback = Rc<dyn Fn(Track)>;
/// Invoked with every selected track when a batch action is chosen from a
/// multi-selected row's context menu.
type MultiTrackCallback = Rc<dyn Fn(Vec<Track>)>;

#[derive(Clone)]
pub struct LibraryView {
    pub widget: ScrolledWindow,
    column_view: ColumnView,
    store: ListStore,
    selection: MultiSelection,
    on_track_enqueue: Rc<OnceCell<SingleTrackCallback>>,
    on_track_edit: Rc<OnceCell<SingleTrackCallback>>,
    on_tracks_enqueue: Rc<OnceCell<MultiTrackCallback>>,
    on_tracks_edit: Rc<OnceCell<MultiTrackCallback>>,
}

impl LibraryView {
    pub fn new() -> Self {
        let store = ListStore::new::<BoxedAnyObject>();
        // A real selection model (rather than `NoSelection`) is what gives
        // ctrl/shift-click multi-select for free — GTK's ColumnView handles
        // the click/modifier logic internally once it has one to update.
        let selection = MultiSelection::new(Some(store.clone()));
        let column_view = ColumnView::new(Some(selection.clone()));
        column_view.set_show_row_separators(true);
        column_view.set_hexpand(true);
        column_view.set_vexpand(true);

        let on_track_enqueue: Rc<OnceCell<SingleTrackCallback>> = Rc::new(OnceCell::new());
        let on_track_edit: Rc<OnceCell<SingleTrackCallback>> = Rc::new(OnceCell::new());
        let on_tracks_enqueue: Rc<OnceCell<MultiTrackCallback>> = Rc::new(OnceCell::new());
        let on_tracks_edit: Rc<OnceCell<MultiTrackCallback>> = Rc::new(OnceCell::new());

        let title_col = text_column(
            "Title",
            |t| t.title.as_str().to_owned(),
            &selection,
            Rc::clone(&on_track_enqueue),
            Rc::clone(&on_track_edit),
            Rc::clone(&on_tracks_enqueue),
            Rc::clone(&on_tracks_edit),
        );
        title_col.set_expand(true);
        column_view.append_column(&title_col);

        let artist_col = text_column(
            "Artist",
            |t| t.artist.as_str().to_owned(),
            &selection,
            Rc::clone(&on_track_enqueue),
            Rc::clone(&on_track_edit),
            Rc::clone(&on_tracks_enqueue),
            Rc::clone(&on_tracks_edit),
        );
        artist_col.set_expand(true);
        column_view.append_column(&artist_col);

        let album_col = text_column(
            "Album",
            |t| t.album.as_str().to_owned(),
            &selection,
            Rc::clone(&on_track_enqueue),
            Rc::clone(&on_track_edit),
            Rc::clone(&on_tracks_enqueue),
            Rc::clone(&on_tracks_edit),
        );
        album_col.set_expand(true);
        column_view.append_column(&album_col);

        column_view.append_column(&text_column(
            "Genre",
            |t| t.genre.as_str().to_owned(),
            &selection,
            Rc::clone(&on_track_enqueue),
            Rc::clone(&on_track_edit),
            Rc::clone(&on_tracks_enqueue),
            Rc::clone(&on_tracks_edit),
        ));
        column_view.append_column(&text_column(
            "Year",
            |t| {
                if t.year.is_unknown() {
                    String::new()
                } else {
                    t.year.value().to_string()
                }
            },
            &selection,
            Rc::clone(&on_track_enqueue),
            Rc::clone(&on_track_edit),
            Rc::clone(&on_tracks_enqueue),
            Rc::clone(&on_tracks_edit),
        ));
        column_view.append_column(&text_column(
            "Duration",
            |t| format_duration(t.duration),
            &selection,
            Rc::clone(&on_track_enqueue),
            Rc::clone(&on_track_edit),
            Rc::clone(&on_tracks_enqueue),
            Rc::clone(&on_tracks_edit),
        ));

        let scrolled = ScrolledWindow::new();
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&column_view));

        Self {
            widget: scrolled,
            column_view,
            store,
            selection,
            on_track_enqueue,
            on_track_edit,
            on_tracks_enqueue,
            on_tracks_edit,
        }
    }

    pub fn set_tracks(&self, tracks: Vec<Track>) {
        self.store.remove_all();
        for track in tracks {
            self.store.append(&BoxedAnyObject::new(track));
        }
    }

    /// Clears the current selection. Call when the user clicks elsewhere in
    /// the application, so a multi-selection doesn't linger indefinitely.
    pub fn clear_selection(&self) {
        self.selection.unselect_all();
    }

    /// Calls `f` with the full visible track list and the index of the
    /// double-clicked row, so the caller can enqueue the list from that track.
    pub fn connect_track_activated<F: Fn(Vec<Track>, usize) + 'static>(&self, f: F) {
        let store = self.store.clone();
        self.column_view.connect_activate(move |_, position| {
            f(collect_tracks(&store), position as usize);
        });
    }

    /// Registers the callback invoked with a single track when "Add to Queue"
    /// is chosen from a row's right-click menu.
    pub fn connect_track_enqueue<F: Fn(Track) + 'static>(&self, f: F) {
        let _ = self.on_track_enqueue.set(Rc::new(f));
    }

    /// Registers the callback invoked with a single track when "Edit Track…"
    /// is chosen from a row's right-click menu.
    pub fn connect_track_edit_requested<F: Fn(Track) + 'static>(&self, f: F) {
        let _ = self.on_track_edit.set(Rc::new(f));
    }

    /// Registers the callback invoked with every selected track when "Add N
    /// to Queue" is chosen from a multi-selected row's right-click menu.
    pub fn connect_tracks_enqueue<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        let _ = self.on_tracks_enqueue.set(Rc::new(f));
    }

    /// Registers the callback invoked with every selected track when "Edit N
    /// Tracks…" is chosen from a multi-selected row's right-click menu.
    pub fn connect_tracks_edit_requested<F: Fn(Vec<Track>) + 'static>(&self, f: F) {
        let _ = self.on_tracks_edit.set(Rc::new(f));
    }
}

/// Snapshots the store's tracks in display order.
fn collect_tracks(store: &ListStore) -> Vec<Track> {
    (0..store.n_items())
        .filter_map(|i| store.item(i).and_downcast::<BoxedAnyObject>())
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
    on_enqueue: Rc<OnceCell<SingleTrackCallback>>,
    on_edit: Rc<OnceCell<SingleTrackCallback>>,
    on_tracks_enqueue: Rc<OnceCell<MultiTrackCallback>>,
    on_tracks_edit: Rc<OnceCell<MultiTrackCallback>>,
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
        let on_tracks_enqueue = Rc::clone(&on_tracks_enqueue);
        let on_tracks_edit = Rc::clone(&on_tracks_edit);
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
            let mut actions: Vec<ContextAction> = Vec::new();
            if selection.is_selected(position) && selection.selection().size() > 1 {
                let tracks = selected_tracks(&selection);
                if let Some(callback) = on_tracks_enqueue.get().cloned() {
                    let tracks = tracks.clone();
                    let label = format!("Add {} to Queue", tracks.len());
                    actions.push((label, Box::new(move || callback(tracks.clone()))));
                }
                if let Some(callback) = on_tracks_edit.get().cloned() {
                    let tracks = tracks.clone();
                    let label = format!("Edit {} Tracks…", tracks.len());
                    actions.push((label, Box::new(move || callback(tracks.clone()))));
                }
            } else {
                // Right-clicking outside the current selection collapses it
                // to just this row — standard file-manager convention.
                selection.select_item(position, true);
                if let Some(callback) = on_enqueue.get().cloned() {
                    let track = track.clone();
                    actions.push((
                        "Add to Queue".to_string(),
                        Box::new(move || callback(track.clone())),
                    ));
                }
                if let Some(callback) = on_edit.get().cloned() {
                    let track = track.clone();
                    actions.push((
                        "Edit Track…".to_string(),
                        Box::new(move || callback(track.clone())),
                    ));
                }
            }
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
