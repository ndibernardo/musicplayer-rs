//! Column preferences for the track list: which `TrackField`s are visible,
//! in what order, and what each renders — a user-editable [`FormatExpr`]
//! rather than a hard-coded renderer. Pure and headless.

use crate::library::format::FormatExpr;
use crate::library::format::TrackField;

/// One visible column: which field it is, and how it renders. `format`
/// defaults to the bare field placeholder (see [`ColumnConfig::default_for`])
/// but the user may customize it to any format string. `width` is `None`
/// until the user drags the header to resize it, meaning "GTK's natural
/// size."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnConfig {
    pub field: TrackField,
    pub format: FormatExpr,
    pub width: Option<i32>,
}

impl ColumnConfig {
    /// The default column for `field`: its bare `%field%` placeholder, at
    /// GTK's natural width.
    pub fn default_for(field: TrackField) -> Self {
        Self {
            field,
            format: FormatExpr::field_only(field),
            width: None,
        }
    }
}

/// The user's chosen visible columns, in display order. A field absent from
/// the list is simply hidden — there is no separate "hidden columns" set to
/// keep in sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnPrefs(Vec<ColumnConfig>);

impl ColumnPrefs {
    /// Builds prefs from an explicit, caller-ordered column list.
    pub fn new(columns: Vec<ColumnConfig>) -> Self {
        Self(columns)
    }

    /// The visible columns, in display order.
    pub fn columns(&self) -> &[ColumnConfig] {
        &self.0
    }

    /// Whether `field` currently has a visible column.
    pub fn is_visible(&self, field: TrackField) -> bool {
        self.0.iter().any(|c| c.field == field)
    }
}

impl Default for ColumnPrefs {
    /// The six columns the list view showed before columns were
    /// customizable, in their original order.
    fn default() -> Self {
        Self::new(
            [
                TrackField::Title,
                TrackField::Artist,
                TrackField::Album,
                TrackField::Genre,
                TrackField::Year,
                TrackField::Duration,
            ]
            .into_iter()
            .map(ColumnConfig::default_for)
            .collect(),
        )
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
    use crate::library::track::Track;
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::TrackPath;
    use crate::library::track::Year;

    fn geogaddi_track() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/home/user/Music/boc/geogaddi/02_music_is_math.flac").unwrap(),
            title: Title::new("Music Is Math"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new(""),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Ambient"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(230),
            track_number: TrackNumber::new(2),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
        }
    }

    #[test]
    fn default_matches_the_original_six_columns_in_order() {
        let fields: Vec<TrackField> = ColumnPrefs::default()
            .columns()
            .iter()
            .map(|c| c.field)
            .collect();
        assert_eq!(
            fields,
            vec![
                TrackField::Title,
                TrackField::Artist,
                TrackField::Album,
                TrackField::Genre,
                TrackField::Year,
                TrackField::Duration,
            ]
        );
    }

    #[test]
    fn default_column_renders_the_bare_field_value() {
        let prefs = ColumnPrefs::default();
        let title_column = &prefs.columns()[0];
        assert_eq!(
            crate::library::format::render(&title_column.format, &geogaddi_track()),
            "Music Is Math"
        );
    }

    #[test]
    fn is_visible_true_for_a_column_in_the_list() {
        assert!(ColumnPrefs::default().is_visible(TrackField::Artist));
    }

    #[test]
    fn is_visible_false_for_a_field_not_in_the_list() {
        assert!(!ColumnPrefs::default().is_visible(TrackField::Composer));
    }

    #[test]
    fn new_preserves_the_given_order_without_resorting() {
        let prefs = ColumnPrefs::new(vec![
            ColumnConfig::default_for(TrackField::Duration),
            ColumnConfig::default_for(TrackField::Title),
        ]);
        let fields: Vec<TrackField> = prefs.columns().iter().map(|c| c.field).collect();
        assert_eq!(fields, vec![TrackField::Duration, TrackField::Title]);
    }

    #[test]
    fn default_for_uses_the_bare_placeholder_for_the_given_field() {
        let config = ColumnConfig::default_for(TrackField::Genre);
        assert_eq!(config.format.to_string(), "%genre%");
    }

    #[test]
    fn default_for_has_no_fixed_width() {
        let config = ColumnConfig::default_for(TrackField::Genre);
        assert_eq!(config.width, None);
    }

    #[test]
    fn new_preserves_a_custom_width() {
        let prefs = ColumnPrefs::new(vec![ColumnConfig {
            width: Some(240),
            ..ColumnConfig::default_for(TrackField::Title)
        }]);
        assert_eq!(prefs.columns()[0].width, Some(240));
    }
}
