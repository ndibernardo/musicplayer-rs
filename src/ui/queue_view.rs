use std::cell::OnceCell;
use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::Expander;
use gtk4::Label;
use gtk4::ListBox;
use gtk4::ListBoxRow;
use gtk4::Orientation;
use gtk4::ScrolledWindow;
use gtk4::prelude::*;

use crate::library::track::Track;
use crate::library::track::TrackId;
use crate::ui::format::display_title;

type SelectCallback = Rc<dyn Fn(usize)>;

/// The play queue as a selectable list. The currently playing track is
/// highlighted; activating a row jumps playback to it. Wrapped in its own
/// `Expander`, which auto-collapses whenever the queue empties out, matching
/// the sidebar's genre/artist sections.
#[derive(Clone)]
pub struct QueueView {
    pub widget: Expander,
    inner: Rc<QueueViewInner>,
}

struct QueueViewInner {
    list: ListBox,
    tracks: RefCell<Vec<Track>>,
    on_select: OnceCell<SelectCallback>,
}

impl QueueView {
    pub fn new() -> Self {
        let list = ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::Single);
        list.set_activate_on_single_click(true);

        let scrolled = ScrolledWindow::new();
        scrolled.set_min_content_height(140);
        scrolled.set_child(Some(&list));

        let widget = Expander::new(Some("Queue"));
        widget.set_expanded(true);
        widget.set_margin_start(4);
        widget.set_child(Some(&scrolled));

        let inner = Rc::new(QueueViewInner {
            list: list.clone(),
            tracks: RefCell::new(Vec::new()),
            on_select: OnceCell::new(),
        });
        {
            let inner = Rc::clone(&inner);
            list.connect_row_activated(move |_, row| {
                if let Some(callback) = inner.on_select.get() {
                    callback(row.index() as usize);
                }
            });
        }

        Self { widget, inner }
    }

    /// Replaces the visible queue with `tracks`. Collapses the section when
    /// the queue empties out; never re-expands it on its own, so a manual
    /// collapse of a non-empty queue is left alone.
    pub fn set_tracks(&self, tracks: Vec<Track>) {
        while let Some(child) = self.inner.list.first_child() {
            self.inner.list.remove(&child);
        }
        for track in &tracks {
            self.inner.list.append(&track_row(track));
        }
        if tracks.is_empty() {
            self.widget.set_expanded(false);
        }
        *self.inner.tracks.borrow_mut() = tracks;
    }

    /// Highlights the row whose track matches `current`, or clears the highlight
    /// when `current` is `None` or absent from the queue.
    pub fn set_current(&self, current: Option<TrackId>) {
        let index = current.and_then(|id| {
            self.inner
                .tracks
                .borrow()
                .iter()
                .position(|track| track.id == id)
        });
        match index.and_then(|i| self.inner.list.row_at_index(i as i32)) {
            Some(row) => self.inner.list.select_row(Some(&row)),
            None => self.inner.list.unselect_all(),
        }
    }

    /// Registers the callback invoked with the row index when a queue entry is
    /// activated.
    pub fn connect_track_selected<F: Fn(usize) + 'static>(&self, f: F) {
        let _ = self.inner.on_select.set(Rc::new(f));
    }
}

fn track_row(track: &Track) -> ListBoxRow {
    let title = display_title(track);
    let title_label = Label::new(Some(&title));
    title_label.set_xalign(0.0);
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

    let artist_label = Label::new(Some(track.artist.as_str()));
    artist_label.set_xalign(0.0);
    artist_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    artist_label.add_css_class("dim-label");
    artist_label.add_css_class("caption");

    let row_box = GtkBox::new(Orientation::Vertical, 0);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);
    row_box.set_margin_top(2);
    row_box.set_margin_bottom(2);
    row_box.append(&title_label);
    row_box.append(&artist_label);

    let row = ListBoxRow::new();
    row.set_child(Some(&row_box));
    row
}
