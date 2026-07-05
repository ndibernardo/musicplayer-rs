use crate::library::track::Track;

/// An ordered list of tracks with a cursor on the one currently selected for
/// playback. A queue is empty when there is nothing to play; otherwise the
/// cursor always points at a real track (`cursor < tracks.len()`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Queue {
    tracks: Vec<Track>,
    cursor: usize,
}

impl Queue {
    /// An empty queue with nothing to play.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A queue over `tracks` positioned at `start`. `start` is clamped to the
    /// last track so the cursor never points past the end; an empty `tracks`
    /// yields an empty queue.
    pub fn new(tracks: Vec<Track>, start: usize) -> Self {
        let cursor = start.min(tracks.len().saturating_sub(1));
        Self { tracks, cursor }
    }

    /// A queue holding a single track.
    pub fn single(track: Track) -> Self {
        Self {
            tracks: vec![track],
            cursor: 0,
        }
    }

    /// The track under the cursor, or `None` when the queue is empty.
    pub fn current(&self) -> Option<&Track> {
        self.tracks.get(self.cursor)
    }

    /// Moves the cursor to the next track and returns it, or `None` when already
    /// at the last track (the cursor does not move past the end).
    pub fn advance(&mut self) -> Option<&Track> {
        if self.cursor + 1 < self.tracks.len() {
            self.cursor += 1;
            self.tracks.get(self.cursor)
        } else {
            None
        }
    }

    /// Moves the cursor to the previous track and returns it, or `None` when
    /// already at the first track (the cursor does not move before the start).
    pub fn rewind(&mut self) -> Option<&Track> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.tracks.get(self.cursor)
    }

    /// The number of tracks in the queue.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// True when the queue holds no tracks.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The cursor's zero-based position.
    pub fn position(&self) -> usize {
        self.cursor
    }

    /// The full track list, in order.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Appends `tracks` to the end of the queue. The cursor (and hence the
    /// currently playing track) is untouched; on a previously empty queue it
    /// stays at 0, pointing at the first appended track.
    pub fn append(&mut self, tracks: Vec<Track>) {
        self.tracks.extend(tracks);
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

    fn geogaddi(id: i64, title: &str) -> Track {
        Track {
            id: TrackId::new(id),
            path: TrackPath::new(format!("/music/geogaddi/{id:02}.flac")).unwrap(),
            title: Title::new(title),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Electronic"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(200),
            track_number: TrackNumber::new(id as u32),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
        }
    }

    fn three_tracks() -> Queue {
        Queue::new(
            vec![
                geogaddi(1, "Music Is Math"),
                geogaddi(2, "Gyroscope"),
                geogaddi(3, "Dandelion"),
            ],
            0,
        )
    }

    #[test]
    fn empty_queue_has_no_current() {
        assert_eq!(Queue::empty().current(), None);
    }

    #[test]
    fn empty_queue_is_empty() {
        assert!(Queue::empty().is_empty());
    }

    #[test]
    fn single_queue_current_is_the_track() {
        let queue = Queue::single(geogaddi(1, "Music Is Math"));
        assert_eq!(queue.current().unwrap().title.as_str(), "Music Is Math");
    }

    #[test]
    fn new_positions_the_cursor_at_start() {
        let at_two = Queue::new(
            vec![
                geogaddi(1, "Music Is Math"),
                geogaddi(2, "Gyroscope"),
                geogaddi(3, "Dandelion"),
            ],
            1,
        );
        assert_eq!(at_two.current().unwrap().title.as_str(), "Gyroscope");
    }

    #[test]
    fn new_clamps_out_of_range_start_to_last_track() {
        let queue = Queue::new(
            vec![geogaddi(1, "Music Is Math"), geogaddi(2, "Gyroscope")],
            9,
        );
        assert_eq!(queue.current().unwrap().title.as_str(), "Gyroscope");
    }

    #[test]
    fn new_with_no_tracks_is_empty() {
        assert!(Queue::new(vec![], 0).is_empty());
    }

    #[test]
    fn advance_moves_to_next_and_returns_it() {
        let mut queue = three_tracks();
        assert_eq!(queue.advance().unwrap().title.as_str(), "Gyroscope");
        assert_eq!(queue.position(), 1);
    }

    #[test]
    fn advance_at_last_track_returns_none_and_stays() {
        let mut queue = Queue::new(vec![geogaddi(1, "Music Is Math")], 0);
        assert_eq!(queue.advance(), None);
        assert_eq!(queue.position(), 0);
    }

    #[test]
    fn advance_on_empty_queue_returns_none() {
        assert_eq!(Queue::empty().advance(), None);
    }

    #[test]
    fn rewind_moves_to_previous_and_returns_it() {
        let mut queue = Queue::new(
            vec![geogaddi(1, "Music Is Math"), geogaddi(2, "Gyroscope")],
            1,
        );
        assert_eq!(queue.rewind().unwrap().title.as_str(), "Music Is Math");
        assert_eq!(queue.position(), 0);
    }

    #[test]
    fn rewind_at_first_track_returns_none_and_stays() {
        let mut queue = three_tracks();
        assert_eq!(queue.rewind(), None);
        assert_eq!(queue.position(), 0);
    }

    #[test]
    fn rewind_on_empty_queue_returns_none() {
        assert_eq!(Queue::empty().rewind(), None);
    }

    #[test]
    fn len_reports_track_count() {
        assert_eq!(three_tracks().len(), 3);
    }

    #[test]
    fn is_empty_is_false_for_a_populated_queue() {
        assert!(!three_tracks().is_empty());
    }

    #[test]
    fn append_to_empty_queue_positions_cursor_at_first_appended() {
        let mut queue = Queue::empty();
        queue.append(vec![geogaddi(1, "Music Is Math")]);
        assert_eq!(queue.current().unwrap().title.as_str(), "Music Is Math");
        assert_eq!(queue.position(), 0);
    }

    #[test]
    fn append_to_non_empty_queue_keeps_the_cursor_on_the_playing_track() {
        let mut queue = three_tracks();
        queue.advance();
        queue.append(vec![geogaddi(4, "In a Beautiful Place Out in the Country")]);
        assert_eq!(queue.current().unwrap().title.as_str(), "Gyroscope");
        assert_eq!(queue.position(), 1);
    }

    #[test]
    fn append_extends_the_track_count() {
        let mut queue = three_tracks();
        queue.append(vec![geogaddi(4, "In a Beautiful Place Out in the Country")]);
        assert_eq!(queue.len(), 4);
    }

    #[test]
    fn tracks_returns_the_full_list_in_order() {
        let queue = three_tracks();
        let titles: Vec<&str> = queue.tracks().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["Music Is Math", "Gyroscope", "Dandelion"]);
    }

    #[test]
    fn tracks_is_empty_for_an_empty_queue() {
        assert!(Queue::empty().tracks().is_empty());
    }
}
