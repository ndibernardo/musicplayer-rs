use std::rc::Rc;

use gtk4::Expander;
use gtk4::ListBoxRow;
use gtk4::ScrolledWindow;
use gtk4::prelude::*;

use crate::library::track::Track;
use crate::library::track::TrackId;
use crate::ui::format::display_title;
use crate::ui::widgets::Callback;
use crate::ui::widgets::CollapsibleSection;
use crate::ui::widgets::ValueList;
use crate::ui::widgets::two_line_row;

/// The play queue as a selectable list. The currently playing track is
/// highlighted; activating a row jumps playback to it. Wrapped in its own
/// [`CollapsibleSection`], which auto-collapses whenever the queue empties
/// out, matching the sidebar's genre/artist sections.
#[derive(Clone)]
pub struct QueueView {
    pub widget: Expander,
    section: CollapsibleSection,
    list: ValueList<Track>,
    on_select: Rc<Callback<usize>>,
}

impl QueueView {
    pub fn new() -> Self {
        let list = ValueList::new(gtk4::SelectionMode::Single, track_row);

        let scrolled = ScrolledWindow::new();
        scrolled.set_min_content_height(140);
        // Fills all the way to the bottom of the section's allocated space
        // while open, instead of stopping at its minimum content height and
        // leaving a gap above the sections below it.
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(list.widget()));

        let section = CollapsibleSection::new("Queue", &scrolled);
        // The queue expands to fill its sidebar's leftover height, pinning
        // whatever sits below it (watched folders, status) to the bottom —
        // its own layout preference, not the panel's.
        section.widget().set_vexpand(true);

        let on_select: Rc<Callback<usize>> = Rc::new(Callback::new());
        {
            let on_select = Rc::clone(&on_select);
            list.connect_activated(move |index, _| on_select.emit(index));
        }

        Self {
            widget: section.widget().clone(),
            section,
            list,
            on_select,
        }
    }

    /// Replaces the visible queue with `tracks`, collapsing the section when
    /// the queue empties out.
    pub fn set_tracks(&self, tracks: Vec<Track>) {
        let empty = self.list.set_items(tracks);
        self.section.set_empty(empty);
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
