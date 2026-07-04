use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Box as GtkBox;
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
/// highlighted; activating a row jumps playback to it.
#[derive(Clone)]
pub struct QueueView {
    pub widget: ScrolledWindow,
    list: ListBox,
    tracks: Rc<RefCell<Vec<Track>>>,
    on_select: Rc<RefCell<Option<SelectCallback>>>,
}

impl QueueView {
    pub fn new() -> Self {
        let list = ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::Single);
        list.set_activate_on_single_click(true);

        let scrolled = ScrolledWindow::new();
        scrolled.set_min_content_height(140);
        scrolled.set_child(Some(&list));

        let on_select: Rc<RefCell<Option<SelectCallback>>> = Rc::new(RefCell::new(None));
        {
            let on_select = Rc::clone(&on_select);
            list.connect_row_activated(move |_, row| {
                if let Some(callback) = on_select.borrow().as_ref() {
                    callback(row.index() as usize);
                }
            });
        }

        Self {
            widget: scrolled,
            list,
            tracks: Rc::new(RefCell::new(Vec::new())),
            on_select,
        }
    }

    /// Replaces the visible queue with `tracks`.
    pub fn set_tracks(&self, tracks: Vec<Track>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for track in &tracks {
            self.list.append(&track_row(track));
        }
        *self.tracks.borrow_mut() = tracks;
    }

    /// Highlights the row whose track matches `current`, or clears the highlight
    /// when `current` is `None` or absent from the queue.
    pub fn set_current(&self, current: Option<TrackId>) {
        let index =
            current.and_then(|id| self.tracks.borrow().iter().position(|track| track.id == id));
        match index.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
    }

    /// Registers the callback invoked with the row index when a queue entry is
    /// activated.
    pub fn connect_track_selected<F: Fn(usize) + 'static>(&self, f: F) {
        *self.on_select.borrow_mut() = Some(Rc::new(f));
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
