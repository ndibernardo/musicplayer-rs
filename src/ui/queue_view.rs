use std::rc::Rc;

use gtk4::Label;
use gtk4::ListBoxRow;
use gtk4::ScrolledWindow;
use gtk4::prelude::*;

use crate::library::track::Track;
use crate::library::track::TrackId;
use crate::ui::format::display_title;
use crate::ui::style;
use crate::ui::style::StyleClass;
use crate::ui::widgets::Callback;
use crate::ui::widgets::ValueList;
use crate::ui::widgets::two_line_row;

/// The play queue as a selectable list, under a plain "Queue" section title —
/// always visible, unlike the sidebar's other sections, so an empty queue
/// still reads as "nothing queued" rather than disappearing.
///
/// Exposes its title and its scrollable list separately, so the caller can
/// place the title in a `SidebarPanel`'s header slot — the same slot
/// `Sidebar`'s "Library" title uses — instead of stacking it inside the
/// content area, where it would pick up the panel's own top inset and end up
/// looking indented differently from "Library".
#[derive(Clone)]
pub struct QueueView {
    header: Label,
    content: ScrolledWindow,
    list: ValueList<Track>,
    on_select: Rc<Callback<usize>>,
}

impl QueueView {
    pub fn new() -> Self {
        let list = ValueList::new(gtk4::SelectionMode::Single, track_row);

        let scrolled = ScrolledWindow::new();
        scrolled.set_min_content_height(140);
        // Fills all the way to the bottom of the sidebar's leftover height,
        // pinning whatever sits below it (watched folders, status) to the
        // bottom — its own layout preference, not the panel's.
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(list.widget()));

        let title = Label::new(Some("Queue"));
        title.set_xalign(0.0);
        style::add_class(&title, StyleClass::SectionName);

        let on_select: Rc<Callback<usize>> = Rc::new(Callback::new());
        {
            let on_select = Rc::clone(&on_select);
            list.connect_activated(move |index, _| on_select.emit(index));
        }

        Self {
            header: title,
            content: scrolled,
            list,
            on_select,
        }
    }

    /// The "Queue" title label — the panel's header slot.
    pub fn header(&self) -> &Label {
        &self.header
    }

    /// The scrollable queue list — stacked into the panel's content slot
    /// above the watched-folders tree.
    pub fn content(&self) -> &ScrolledWindow {
        &self.content
    }

    /// Replaces the visible queue with `tracks`.
    pub fn set_tracks(&self, tracks: Vec<Track>) {
        self.list.set_items(tracks);
    }

    /// Highlights the row whose track matches `current`, or clears the highlight
    /// when `current` is `None` or absent from the queue.
    pub fn set_current(&self, current: Option<TrackId>) {
        match current {
            Some(id) => self.list.select_where(|track| track.id == id),
            None => self.list.clear_selection(),
        }
    }

    /// Registers the callback invoked with the row index when a queue entry is
    /// activated.
    pub fn connect_track_selected<F: Fn(usize) + 'static>(&self, f: F) {
        self.on_select.set(f);
    }
}

fn track_row(track: &Track) -> ListBoxRow {
    two_line_row(&display_title(track), track.artist.as_str())
}
