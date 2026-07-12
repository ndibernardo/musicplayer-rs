use crate::library::track::AlbumTitle;
use crate::library::track::Artist;
use crate::library::track::Composer;
use crate::library::track::Genre;
use crate::library::track::Year;

/// The active track-list and album-grid filter, driven by the sidebar selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryFilter {
    All,
    ByGenre(Genre),
    ByArtist(Artist),
    ByAlbumArtist(Artist),
    ByAlbum(AlbumTitle),
    ByYear(Year),
    ByComposer(Composer),
}

/// A track field the sidebar can browse as a filter category. A deliberate
/// subset of every track field — only the ones meaningful to browse as a
/// distinct-values list (unlike, say, `Title` or `Duration`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterField {
    Genre,
    AlbumArtist,
    Artist,
    Year,
    Composer,
}

impl FilterField {
    /// How many filterable fields exist — the length of [`FilterField::all`].
    pub const COUNT: usize = 5;

    /// Every filterable field, in the sidebar's canonical section order.
    pub fn all() -> [FilterField; Self::COUNT] {
        [
            FilterField::Genre,
            FilterField::AlbumArtist,
            FilterField::Artist,
            FilterField::Year,
            FilterField::Composer,
        ]
    }

    /// This field's position in [`FilterField::all`] — lets callers key
    /// per-field array storage with the exhaustiveness of a match.
    pub const fn index(self) -> usize {
        match self {
            FilterField::Genre => 0,
            FilterField::AlbumArtist => 1,
            FilterField::Artist => 2,
            FilterField::Year => 3,
            FilterField::Composer => 4,
        }
    }

    /// The label shown for this field in the sidebar's filter picker and as
    /// its section header.
    pub fn label(self) -> &'static str {
        match self {
            FilterField::Genre => "Genre",
            FilterField::AlbumArtist => "Album Artist",
            FilterField::Artist => "Artist",
            FilterField::Year => "Year",
            FilterField::Composer => "Composer",
        }
    }

    /// The name persisted as a settings key.
    pub fn as_key(self) -> &'static str {
        match self {
            FilterField::Genre => "genre",
            FilterField::AlbumArtist => "album_artist",
            FilterField::Artist => "artist",
            FilterField::Year => "year",
            FilterField::Composer => "composer",
        }
    }

    /// Parses a persisted settings key, or `None` when it names no known field.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "genre" => Some(FilterField::Genre),
            "album_artist" => Some(FilterField::AlbumArtist),
            "artist" => Some(FilterField::Artist),
            "year" => Some(FilterField::Year),
            "composer" => Some(FilterField::Composer),
            _ => None,
        }
    }

    /// Turns a selected sidebar row's display value back into the
    /// corresponding `LibraryFilter`. `value` is always one this field's own
    /// `Db::distinct_values_for` produced, so the `Year` parse cannot fail in
    /// practice; `unwrap_or` is a safe fallback rather than a real error path.
    pub fn to_filter(self, value: &str) -> LibraryFilter {
        match self {
            FilterField::Genre => LibraryFilter::ByGenre(Genre::new(value)),
            FilterField::AlbumArtist => LibraryFilter::ByAlbumArtist(Artist::new(value)),
            FilterField::Artist => LibraryFilter::ByArtist(Artist::new(value)),
            FilterField::Year => LibraryFilter::ByYear(Year::new(value.parse().unwrap_or(0))),
            FilterField::Composer => LibraryFilter::ByComposer(Composer::new(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_field_from_key_round_trips_every_field() {
        for field in FilterField::all() {
            assert_eq!(FilterField::from_key(field.as_key()), Some(field));
        }
    }

    #[test]
    fn filter_field_index_matches_position_in_all() {
        for (position, field) in FilterField::all().into_iter().enumerate() {
            assert_eq!(field.index(), position);
        }
    }

    #[test]
    fn filter_field_from_key_returns_none_for_an_unknown_key() {
        assert_eq!(FilterField::from_key("duration"), None);
    }

    #[test]
    fn to_filter_genre_builds_by_genre() {
        assert_eq!(
            FilterField::Genre.to_filter("Ambient"),
            LibraryFilter::ByGenre(Genre::new("Ambient"))
        );
    }

    #[test]
    fn to_filter_album_artist_builds_by_album_artist() {
        assert_eq!(
            FilterField::AlbumArtist.to_filter("Various Artists"),
            LibraryFilter::ByAlbumArtist(Artist::new("Various Artists"))
        );
    }

    #[test]
    fn to_filter_artist_builds_by_artist() {
        assert_eq!(
            FilterField::Artist.to_filter("Boards of Canada"),
            LibraryFilter::ByArtist(Artist::new("Boards of Canada"))
        );
    }

    #[test]
    fn to_filter_year_builds_by_year() {
        assert_eq!(
            FilterField::Year.to_filter("1998"),
            LibraryFilter::ByYear(Year::new(1998))
        );
    }

    #[test]
    fn to_filter_composer_builds_by_composer() {
        assert_eq!(
            FilterField::Composer.to_filter("Erik Satie"),
            LibraryFilter::ByComposer(Composer::new("Erik Satie"))
        );
    }
}
