use glib::BoxedAnyObject;
use gtk4::ColumnView;
use gtk4::ColumnViewColumn;
use gtk4::Label;
use gtk4::ListItem;
use gtk4::NoSelection;
use gtk4::ScrolledWindow;
use gtk4::SignalListItemFactory;
use gtk4::gio::ListStore;
use gtk4::prelude::*;

use crate::domain::track::Track;
use crate::domain::track::TrackDuration;

#[derive(Clone)]
pub struct LibraryView {
    pub widget: ScrolledWindow,
    store: ListStore,
}

impl LibraryView {
    pub fn new() -> Self {
        let store = ListStore::new::<BoxedAnyObject>();
        let selection = NoSelection::new(Some(store.clone()));
        let column_view = ColumnView::new(Some(selection));
        column_view.set_show_row_separators(true);
        column_view.set_hexpand(true);
        column_view.set_vexpand(true);

        let title_col = text_column("Title", |t| t.title.as_str().to_owned());
        title_col.set_expand(true);
        column_view.append_column(&title_col);

        let artist_col = text_column("Artist", |t| t.artist.as_str().to_owned());
        artist_col.set_expand(true);
        column_view.append_column(&artist_col);

        let album_col = text_column("Album", |t| t.album.as_str().to_owned());
        album_col.set_expand(true);
        column_view.append_column(&album_col);

        column_view.append_column(&text_column("Genre", |t| t.genre.as_str().to_owned()));
        column_view.append_column(&text_column("Year", |t| {
            if t.year.is_unknown() {
                String::new()
            } else {
                t.year.value().to_string()
            }
        }));
        column_view.append_column(&text_column("Duration", |t| format_duration(t.duration)));

        let scrolled = ScrolledWindow::new();
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&column_view));

        Self {
            widget: scrolled,
            store,
        }
    }

    pub fn set_tracks(&self, tracks: Vec<Track>) {
        self.store.remove_all();
        for track in tracks {
            self.store.append(&BoxedAnyObject::new(track));
        }
    }
}

fn text_column<F>(title: &str, extract: F) -> ColumnViewColumn
where
    F: Fn(&Track) -> String + 'static,
{
    let factory = SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
        let Some(item) = obj.downcast_ref::<ListItem>() else { return };
        let label = Label::new(None);
        label.set_xalign(0.0);
        label.set_margin_start(6);
        label.set_margin_end(6);
        item.set_child(Some(&label));
    });
    factory.connect_bind(move |_, obj| {
        let Some(item) = obj.downcast_ref::<ListItem>() else { return };
        let Some(data) = item.item().and_downcast::<BoxedAnyObject>() else { return };
        let track = data.borrow::<Track>();
        let Some(label) = item.child().and_downcast::<Label>() else { return };
        label.set_text(&extract(&*track));
    });
    ColumnViewColumn::new(Some(title), Some(factory))
}

fn format_duration(d: TrackDuration) -> String {
    let total_secs = d.as_secs();
    format!("{}:{:02}", total_secs / 60, total_secs % 60)
}
