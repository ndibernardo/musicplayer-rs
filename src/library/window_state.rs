use crate::library::album::AlbumSort;
use crate::library::album::AlbumSortField;
use crate::library::album::SortDirection;
use crate::library::db::LibraryFolder;
use crate::library::filter::LibraryFilter;
use crate::library::metadata_edit::EditOutcome;
use crate::library::scan::ScanEvent;
use crate::library::track::Track;
use crate::library::view_mode::ViewMode;
use crate::player::PlaybackState;

/// The main window's testable application state: the active library filter,
/// album sort, and play queue. No I/O, no GTK, no OS handles — the folder
/// watcher is infrastructure state, not application state, so it lives
/// outside this (see `ui::main_window::Context`).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowState {
    pub filter: LibraryFilter,
    pub sort: AlbumSort,
    pub queue: Vec<Track>,
}

/// Every user action and external event `MainWindow` reacts to. `ScanEvent`
/// wraps `rusqlite::Error` transitively, which isn't `Clone`/`PartialEq`, so
/// `WindowMessage` only derives `Debug` — nothing needs to clone or compare a
/// whole `WindowMessage`, only the domain values carried inside specific
/// variants.
#[derive(Debug)]
pub enum WindowMessage {
    FilterSelected(LibraryFilter),
    SortFieldChanged(AlbumSortField),
    SortDirectionChanged(SortDirection),
    ViewModeChanged(ViewMode),
    CoverSizeChanged(i32),
    VolumeChanged(f64),
    /// Replaces the queue with `Vec<Track>` at the given start index and plays it.
    Enqueue(Vec<Track>, usize),
    AppendToQueue(Vec<Track>),
    QueueTrackSelected(usize),
    /// The player thread's queue changed — see `player::PlayerHandle::launch`'s
    /// `on_queue_changed` callback.
    PlayerQueueChanged(Vec<Track>),
    /// The player thread emitted a new playback state.
    PlayerStateChanged(PlaybackState),
    ScanRequested,
    ScanEvent(ScanEvent),
    /// The debounced folder watcher wants a rescan.
    RescanRequested,
    FolderAdded(LibraryFolder),
    FolderRemoved(LibraryFolder),
    /// A track/album metadata edit finished saving on a background thread.
    /// `affects_art` is computed at spawn time (the edit itself is gone by
    /// the time the outcome arrives) — see `metadata_edit::TrackEdit::affects_art_grouping`.
    EditSaved {
        affects_art: bool,
        outcome: EditOutcome,
    },
}

/// The pure state transition for `msg`: no I/O, no GTK — fully unit-testable.
/// `ui::main_window::Context::apply` performs the corresponding side effects
/// (DB queries, player commands, widget updates) for the same `msg`.
pub fn reduce(state: WindowState, msg: &WindowMessage) -> WindowState {
    match msg {
        WindowMessage::FilterSelected(filter) => WindowState {
            filter: filter.clone(),
            ..state
        },
        WindowMessage::SortFieldChanged(field) => WindowState {
            sort: AlbumSort {
                field: *field,
                ..state.sort
            },
            ..state
        },
        WindowMessage::SortDirectionChanged(direction) => WindowState {
            sort: AlbumSort {
                direction: *direction,
                ..state.sort
            },
            ..state
        },
        WindowMessage::PlayerQueueChanged(tracks) => WindowState {
            queue: tracks.clone(),
            ..state
        },
        // Every other message is either an infrastructure/widget concern
        // (cover size, volume, view mode, scan/folder events) or is applied to
        // WindowState.queue indirectly via the PlayerQueueChanged echo that
        // follows a PlayerCommand — see Context::apply.
        WindowMessage::ViewModeChanged(_)
        | WindowMessage::CoverSizeChanged(_)
        | WindowMessage::VolumeChanged(_)
        | WindowMessage::Enqueue(_, _)
        | WindowMessage::AppendToQueue(_)
        | WindowMessage::QueueTrackSelected(_)
        | WindowMessage::PlayerStateChanged(_)
        | WindowMessage::ScanRequested
        | WindowMessage::ScanEvent(_)
        | WindowMessage::RescanRequested
        | WindowMessage::FolderAdded(_)
        | WindowMessage::FolderRemoved(_)
        | WindowMessage::EditSaved { .. } => state,
    }
}

#[cfg(test)]
mod tests {
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

    fn initial_state() -> WindowState {
        WindowState {
            filter: LibraryFilter::All,
            sort: AlbumSort::default(),
            queue: Vec::new(),
        }
    }

    fn roygbiv() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/music/boc/roygbiv.flac").unwrap(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Music Has the Right to Children"),
            genre: Genre::new("Electronic"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(193),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(1998),
        }
    }

    #[test]
    fn reduce_filter_selected_updates_filter_and_leaves_sort_and_queue() {
        let state = initial_state();
        let filter = LibraryFilter::ByGenre(Genre::new("Ambient"));
        let next = reduce(
            state.clone(),
            &WindowMessage::FilterSelected(filter.clone()),
        );

        assert_eq!(next.filter, filter);
        assert_eq!(next.sort, state.sort);
        assert_eq!(next.queue, state.queue);
    }

    #[test]
    fn reduce_sort_field_changed_updates_only_the_field() {
        let state = initial_state();
        let next = reduce(
            state.clone(),
            &WindowMessage::SortFieldChanged(AlbumSortField::Year),
        );

        assert_eq!(next.sort.field, AlbumSortField::Year);
        assert_eq!(next.sort.direction, state.sort.direction);
        assert_eq!(next.filter, state.filter);
    }

    #[test]
    fn reduce_sort_direction_changed_updates_only_the_direction() {
        let state = initial_state();
        let next = reduce(
            state.clone(),
            &WindowMessage::SortDirectionChanged(SortDirection::Descending),
        );

        assert_eq!(next.sort.direction, SortDirection::Descending);
        assert_eq!(next.sort.field, state.sort.field);
    }

    #[test]
    fn reduce_player_queue_changed_replaces_the_queue() {
        let state = initial_state();
        let tracks = vec![roygbiv()];
        let next = reduce(state, &WindowMessage::PlayerQueueChanged(tracks.clone()));

        assert_eq!(next.queue, tracks);
    }

    #[test]
    fn reduce_volume_changed_leaves_state_unchanged() {
        let state = initial_state();
        let next = reduce(state.clone(), &WindowMessage::VolumeChanged(42.0));

        assert_eq!(next, state);
    }
}
