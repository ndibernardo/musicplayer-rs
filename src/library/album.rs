use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Genre;
use crate::library::track::Year;

/// One album's worth of grid metadata: its title, its (primary) artist, genre,
/// release year, and whether any track in the album carries embedded cover art.
/// `has_art` avoids shipping every cover's bytes on every grid refresh; callers
/// fetch the bytes separately (e.g. `Db::art_for_album`) only when they need to
/// render a cover that isn't already cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumSummary {
    pub album: AlbumTitle,
    pub artist: Artist,
    pub genre: Genre,
    pub year: Year,
    pub has_art: bool,
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
