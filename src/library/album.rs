use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Genre;
use crate::library::track::Track;
use crate::library::track::Year;

/// Identifies one album's cover art: the (album, effective album artist) pair
/// the `album_art` table is keyed by. The effective-album-artist rule (album
/// artist, falling back to the track artist) and the "no album, no key" rule
/// are both enforced by the constructors — nowhere else needs to know them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtKey {
    album: AlbumTitle,
    album_artist: Artist,
}

impl ArtKey {
    /// Returns `None` when `album` is empty — art without an album cannot be
    /// keyed, and grouping already collapses everything else onto one artist.
    pub fn new(album: AlbumTitle, album_artist: Artist) -> Option<Self> {
        if album.as_str().is_empty() {
            return None;
        }
        Some(Self {
            album,
            album_artist,
        })
    }

    /// The key for `track`'s album, using its effective album artist: the
    /// tagged album artist, falling back to the track artist when absent.
    pub fn for_track(track: &Track) -> Option<Self> {
        let album_artist = if track.album_artist.is_unknown() {
            track.artist.clone()
        } else {
            track.album_artist.clone()
        };
        Self::new(track.album.clone(), album_artist)
    }

    pub fn album(&self) -> &AlbumTitle {
        &self.album
    }

    pub fn album_artist(&self) -> &Artist {
        &self.album_artist
    }
}

/// Whether an album has cover art, and if so under which key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverArt {
    Absent,
    Available(ArtKey),
}

/// One album's worth of grid metadata: its title, its (primary) artist, genre,
/// release year, and its cover art, if any. `CoverArt` carries only the key,
/// not the bytes — callers fetch bytes separately (e.g. `Db::art_for`) only
/// when they need to render a cover that isn't already cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumSummary {
    pub album: AlbumTitle,
    pub artist: Artist,
    pub genre: Genre,
    pub year: Year,
    pub art: CoverArt,
}

/// The field the album grid is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumSortField {
    AlbumArtist,
    Year,
    Genre,
    Album,
}

impl AlbumSortField {
    /// The grouped-query column alias this field sorts on.
    fn column(self) -> &'static str {
        match self {
            AlbumSortField::AlbumArtist => "album_artist",
            AlbumSortField::Year => "album_year",
            AlbumSortField::Genre => "album_genre",
            AlbumSortField::Album => "album",
        }
    }

    /// The stable string persisted in settings.
    pub fn as_key(self) -> &'static str {
        match self {
            AlbumSortField::AlbumArtist => "album_artist",
            AlbumSortField::Year => "year",
            AlbumSortField::Genre => "genre",
            AlbumSortField::Album => "album",
        }
    }

    /// Parses a persisted key, or `None` when unknown.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "album_artist" => Some(AlbumSortField::AlbumArtist),
            "year" => Some(AlbumSortField::Year),
            "genre" => Some(AlbumSortField::Genre),
            "album" => Some(AlbumSortField::Album),
            _ => None,
        }
    }
}

/// The direction an ordering runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    fn sql(self) -> &'static str {
        match self {
            SortDirection::Ascending => "ASC",
            SortDirection::Descending => "DESC",
        }
    }

    pub fn as_key(self) -> &'static str {
        match self {
            SortDirection::Ascending => "asc",
            SortDirection::Descending => "desc",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "asc" => Some(SortDirection::Ascending),
            "desc" => Some(SortDirection::Descending),
            _ => None,
        }
    }
}

/// How the album grid is ordered: a field and a direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlbumSort {
    pub field: AlbumSortField,
    pub direction: SortDirection,
}

impl AlbumSort {
    pub fn new(field: AlbumSortField, direction: SortDirection) -> Self {
        Self { field, direction }
    }

    /// The SQL `ORDER BY` clause for the grouped album-summary query, with
    /// album artist and album appended as stable tie-breakers. The field names
    /// come from a fixed set, never user input, so this cannot inject.
    pub fn order_by_clause(&self) -> String {
        let primary = format!("{} {}", self.field.column(), self.direction.sql());
        let mut keys = vec![primary];
        for tie_break in ["album_artist", "album"] {
            if tie_break != self.field.column() {
                keys.push(format!("{tie_break} ASC"));
            }
        }
        format!("ORDER BY {}", keys.join(", "))
    }
}

impl Default for AlbumSort {
    /// Album artist ascending, matching the original grid order.
    fn default() -> Self {
        Self::new(AlbumSortField::AlbumArtist, SortDirection::Ascending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::track::Composer;
    use crate::library::track::DiscNumber;
    use crate::library::track::Title;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::TrackPath;

    fn roygbiv() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/music/boc/roygbiv.flac").unwrap(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new(""),
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
    fn art_key_new_rejects_empty_album() {
        assert_eq!(
            ArtKey::new(AlbumTitle::new(""), Artist::new("Various Artists")),
            None
        );
    }

    #[test]
    fn art_key_new_accepts_non_empty_album() {
        let key = ArtKey::new(AlbumTitle::new("Geogaddi"), Artist::new("Boards of Canada"));
        assert!(key.is_some());
    }

    #[test]
    fn art_key_for_track_uses_the_tagged_album_artist() {
        let track = Track {
            album_artist: Artist::new("Various Artists"),
            ..roygbiv()
        };
        let key = ArtKey::for_track(&track).unwrap();
        assert_eq!(key.album_artist().as_str(), "Various Artists");
    }

    #[test]
    fn art_key_for_track_falls_back_to_artist_when_album_artist_absent() {
        // roygbiv() has no album-artist tag — is_unknown, per Artist::new("").
        let key = ArtKey::for_track(&roygbiv()).unwrap();
        assert_eq!(key.album_artist().as_str(), "Boards of Canada");
    }

    #[test]
    fn art_key_for_track_is_none_when_album_is_empty() {
        let track = Track {
            album: AlbumTitle::new(""),
            ..roygbiv()
        };
        assert_eq!(ArtKey::for_track(&track), None);
    }

    #[test]
    fn default_sort_orders_by_album_artist_ascending() {
        assert_eq!(
            AlbumSort::default().order_by_clause(),
            "ORDER BY album_artist ASC, album ASC"
        );
    }

    #[test]
    fn year_descending_orders_by_year_then_tie_breaks() {
        let sort = AlbumSort::new(AlbumSortField::Year, SortDirection::Descending);
        assert_eq!(
            sort.order_by_clause(),
            "ORDER BY album_year DESC, album_artist ASC, album ASC"
        );
    }

    #[test]
    fn genre_ascending_orders_by_genre_then_tie_breaks() {
        let sort = AlbumSort::new(AlbumSortField::Genre, SortDirection::Ascending);
        assert_eq!(
            sort.order_by_clause(),
            "ORDER BY album_genre ASC, album_artist ASC, album ASC"
        );
    }

    #[test]
    fn album_field_drops_itself_from_tie_breaks() {
        let sort = AlbumSort::new(AlbumSortField::Album, SortDirection::Ascending);
        assert_eq!(
            sort.order_by_clause(),
            "ORDER BY album ASC, album_artist ASC"
        );
    }

    #[test]
    fn sort_field_key_round_trips() {
        for field in [
            AlbumSortField::AlbumArtist,
            AlbumSortField::Year,
            AlbumSortField::Genre,
            AlbumSortField::Album,
        ] {
            assert_eq!(AlbumSortField::from_key(field.as_key()), Some(field));
        }
    }

    #[test]
    fn sort_direction_key_round_trips() {
        assert_eq!(
            SortDirection::from_key(SortDirection::Ascending.as_key()),
            Some(SortDirection::Ascending)
        );
        assert_eq!(
            SortDirection::from_key(SortDirection::Descending.as_key()),
            Some(SortDirection::Descending)
        );
    }

    #[test]
    fn sort_field_from_unknown_key_is_none() {
        assert_eq!(AlbumSortField::from_key("bitrate"), None);
    }
}
