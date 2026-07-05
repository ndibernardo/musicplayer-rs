use std::cell::OnceCell;
use std::rc::Rc;

use glib::BoxedAnyObject;
use gtk4::ColumnView;
use gtk4::ColumnViewColumn;
use gtk4::GestureClick;
use gtk4::Label;
use gtk4::ListItem;
use gtk4::NoSelection;
use gtk4::ScrolledWindow;
use gtk4::SignalListItemFactory;
use gtk4::Widget;
use gtk4::gio::ListStore;
use gtk4::prelude::*;

use crate::library::track::Track;
use crate::ui::context_menu::show_add_to_queue_menu;
use crate::ui::format::format_duration;

/// Invoked with a single track chosen from a row's "add to queue" context menu.
type SingleTrackCallback = Rc<dyn Fn(Track)>;

#[derive(Clone)]
pub struct LibraryView {
    pub widget: ScrolledWindow,
    column_view: ColumnView,
    store: ListStore,
    on_track_enqueue: Rc<OnceCell<SingleTrackCallback>>,
}

impl LibraryView {
    pub fn new() -> Self {
        let store = ListStore::new::<BoxedAnyObject>();
        let selection = NoSelection::new(Some(store.clone()));
        let column_view = ColumnView::new(Some(selection));
        column_view.set_show_row_separators(true);
        column_view.set_hexpand(true);
        column_view.set_vexpand(true);

        let on_track_enqueue: Rc<OnceCell<SingleTrackCallback>> = Rc::new(OnceCell::new());

        let title_col = text_column(
            "Title",
            |t| t.title.as_str().to_owned(),
            Rc::clone(&on_track_enqueue),
        );
        title_col.set_expand(true);
        column_view.append_column(&title_col);

        let artist_col = text_column(
            "Artist",
            |t| t.artist.as_str().to_owned(),
            Rc::clone(&on_track_enqueue),
        );
        artist_col.set_expand(true);
        column_view.append_column(&artist_col);

        let album_col = text_column(
            "Album",
            |t| t.album.as_str().to_owned(),
            Rc::clone(&on_track_enqueue),
        );
        album_col.set_expand(true);
        column_view.append_column(&album_col);

        column_view.append_column(&text_column(
            "Genre",
            |t| t.genre.as_str().to_owned(),
            Rc::clone(&on_track_enqueue),
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
            Rc::clone(&on_track_enqueue),
        ));
        column_view.append_column(&text_column(
            "Duration",
            |t| format_duration(t.duration),
            Rc::clone(&on_track_enqueue),
        ));

        let scrolled = ScrolledWindow::new();
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&column_view));

        Self {
            widget: scrolled,
            column_view,
            store,
            on_track_enqueue,
        }
    }

    pub fn set_tracks(&self, tracks: Vec<Track>) {
        self.store.remove_all();
        for track in tracks {
            self.store.append(&BoxedAnyObject::new(track));
        }
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
}

/// Snapshots the store's tracks in display order.
fn collect_tracks(store: &ListStore) -> Vec<Track> {
    (0..store.n_items())
        .filter_map(|i| store.item(i).and_downcast::<BoxedAnyObject>())
        .map(|obj| obj.borrow::<Track>().clone())
        .collect()
}

fn text_column<F>(
    title: &str,
    extract: F,
    on_enqueue: Rc<OnceCell<SingleTrackCallback>>,
) -> ColumnViewColumn
where
    F: Fn(&Track) -> String + 'static,
{
    let factory = SignalListItemFactory::new();
    factory.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<ListItem>() else {
            return;
        };
        let label = Label::new(None);
        label.set_xalign(0.0);
        label.set_margin_start(6);
        label.set_margin_end(6);

        // Right-click offers "Add to Queue" for whichever track is currently
        // bound to this row; `item` is weakly held since it strongly owns
        // `label` as its child, which would otherwise cycle.
        let gesture = GestureClick::new();
        gesture.set_button(gtk4::gdk::BUTTON_SECONDARY);
        let item_weak = item.downgrade();
        let on_enqueue = Rc::clone(&on_enqueue);
        let label_widget = label.clone().upcast::<Widget>();
        gesture.connect_pressed(move |_, _, x, y| {
            let Some(item) = item_weak.upgrade() else {
                return;
            };
            let Some(data) = item.item().and_downcast::<BoxedAnyObject>() else {
                return;
            };
            let track = data.borrow::<Track>().clone();
            let Some(callback) = on_enqueue.get().cloned() else {
                return;
            };
            show_add_to_queue_menu(&label_widget, x, y, move || callback(track.clone()));
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
