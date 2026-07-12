use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::Button;
use gtk4::Orientation;
use gtk4::Popover;
use gtk4::Widget;
use gtk4::gdk::Rectangle;
use gtk4::prelude::*;

use crate::library::track::Track;

/// One labeled action in a `show_context_menu` popover. The label is an
/// owned `String` (not `&'static str`) since a batch action's label carries a
/// selection count computed at click time, e.g. "Add 3 to Queue".
pub type ContextAction = (String, Box<dyn Fn()>);

/// Shows a popover of one flat button per `(label, action)` pair in `actions`,
/// anchored at `(x, y)` — widget-local coordinates, as reported by a
/// `GestureClick` attached to `parent`. The popover detaches itself from
/// `parent` once closed, so repeated right-clicks don't accumulate stale
/// popovers.
pub fn show_context_menu(parent: &Widget, x: f64, y: f64, actions: Vec<ContextAction>) {
    let popover = Popover::new();
    popover.set_parent(parent);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&Rectangle::new(x as i32, y as i32, 1, 1)));

    let list = GtkBox::new(Orientation::Vertical, 0);
    for (label, action) in actions {
        let button = Button::with_label(&label);
        button.add_css_class("flat");
        let for_click = popover.clone();
        button.connect_clicked(move |_| {
            action();
            for_click.popdown();
        });
        list.append(&button);
    }
    popover.set_child(Some(&list));
    popover.connect_closed(|pop| pop.unparent());

    popover.popup();
}

/// The app-wide right-click convention for a track row, shared by the
/// library list and the album drawer: a row that's part of a wider
/// multi-selection (`batch` is `Some`) offers actions over the whole
/// selection ("Add N to Queue" / "Edit N Tracks…"); anything else offers
/// singular actions for `track` alone ("Add to Queue" / "Edit Track…").
/// Both callbacks take `Vec<Track>` — the singular case is just `batch`'s
/// one-element special case, not a different action.
pub fn track_actions(
    track: &Track,
    batch: Option<Vec<Track>>,
    on_enqueue: Option<Rc<dyn Fn(Vec<Track>)>>,
    on_edit: Option<Rc<dyn Fn(Vec<Track>)>>,
) -> Vec<ContextAction> {
    let tracks = batch.unwrap_or_else(|| vec![track.clone()]);
    let count = tracks.len();
    let mut actions: Vec<ContextAction> = Vec::new();
    if let Some(callback) = on_enqueue {
        let tracks = tracks.clone();
        actions.push((
            enqueue_label(count),
            Box::new(move || callback(tracks.clone())) as Box<dyn Fn()>,
        ));
    }
    if let Some(callback) = on_edit {
        actions.push((
            edit_label(count),
            Box::new(move || callback(tracks.clone())) as Box<dyn Fn()>,
        ));
    }
    actions
}

fn enqueue_label(count: usize) -> String {
    if count == 1 {
        "Add to Queue".to_string()
    } else {
        format!("Add {count} to Queue")
    }
}

fn edit_label(count: usize) -> String {
    if count == 1 {
        "Edit Track…".to_string()
    } else {
        format!("Edit {count} Tracks…")
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::library::track::AlbumTitle;
    use crate::library::track::Artist;
    use crate::library::track::Composer;
    use crate::library::track::DiscNumber;
    use crate::library::track::Genre;
    use crate::library::track::Title;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::TrackPath;
    use crate::library::track::Year;

    fn geogaddi_track() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/home/user/Music/boc/geogaddi/01_alpha_and_omega.flac").unwrap(),
            title: Title::new("Alpha and Omega"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new(""),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Ambient"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(133),
            track_number: TrackNumber::new(1),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
        }
    }

    #[test]
    fn enqueue_label_reads_add_to_queue_for_a_single_track() {
        assert_eq!(enqueue_label(1), "Add to Queue");
    }

    #[test]
    fn enqueue_label_reads_add_n_to_queue_for_a_batch() {
        assert_eq!(enqueue_label(3), "Add 3 to Queue");
    }

    #[test]
    fn edit_label_reads_edit_track_for_a_single_track() {
        assert_eq!(edit_label(1), "Edit Track…");
    }

    #[test]
    fn edit_label_reads_edit_n_tracks_for_a_batch() {
        assert_eq!(edit_label(3), "Edit 3 Tracks…");
    }

    #[test]
    fn track_actions_uses_singular_labels_when_batch_is_none() {
        let track = geogaddi_track();
        let actions = track_actions(&track, None, Some(Rc::new(|_| {})), Some(Rc::new(|_| {})));
        let labels: Vec<&str> = actions.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, vec!["Add to Queue", "Edit Track…"]);
    }

    #[test]
    fn track_actions_uses_batch_labels_when_batch_is_some() {
        let track = geogaddi_track();
        let mut second = geogaddi_track();
        second.title = Title::new("In a Beautiful Place Out in the Country");
        let actions = track_actions(
            &track,
            Some(vec![track.clone(), second]),
            Some(Rc::new(|_| {})),
            Some(Rc::new(|_| {})),
        );
        let labels: Vec<&str> = actions.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, vec!["Add 2 to Queue", "Edit 2 Tracks…"]);
    }

    #[test]
    fn track_actions_omits_enqueue_when_no_handler_is_registered() {
        let track = geogaddi_track();
        let actions = track_actions(&track, None, None, Some(Rc::new(|_| {})));
        let labels: Vec<&str> = actions.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, vec!["Edit Track…"]);
    }

    #[test]
    fn track_actions_omits_edit_when_no_handler_is_registered() {
        let track = geogaddi_track();
        let actions = track_actions(&track, None, Some(Rc::new(|_| {})), None);
        let labels: Vec<&str> = actions.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, vec!["Add to Queue"]);
    }

    #[test]
    fn track_actions_invokes_the_enqueue_callback_with_the_batch() {
        let track = geogaddi_track();
        let mut second = geogaddi_track();
        second.id = TrackId::new(2);
        let received: Rc<RefCell<Vec<Track>>> = Rc::new(RefCell::new(Vec::new()));
        let received_in_callback = Rc::clone(&received);
        let actions = track_actions(
            &track,
            Some(vec![track.clone(), second.clone()]),
            Some(Rc::new(move |tracks| {
                *received_in_callback.borrow_mut() = tracks
            })),
            None,
        );
        (actions[0].1)();
        assert_eq!(*received.borrow(), vec![track, second]);
    }
}
