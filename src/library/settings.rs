use crate::library::album::AlbumSort;
use crate::library::album::AlbumSortField;
use crate::library::album::SortDirection;
use crate::library::column::ColumnConfig;
use crate::library::column::ColumnPrefs;
use crate::library::db::Db;
use crate::library::format;
use crate::library::format::FormatExpr;
use crate::library::format::TrackField;
use crate::library::track::TrackId;
use crate::library::view_mode::ViewMode;

const COVER_SIZE_KEY: &str = "cover_size";
pub const COVER_SIZE_MIN: i32 = 200;
pub const COVER_SIZE_MAX: i32 = 500;

const VOLUME_KEY: &str = "volume";
const DEFAULT_VOLUME: f64 = 70.0;

const QUEUE_KEY: &str = "queue";
const QUEUE_CURRENT_KEY: &str = "queue_current";
const QUEUE_POSITION_KEY: &str = "queue_position";

const ALBUM_SORT_FIELD_KEY: &str = "album_sort_field";
const ALBUM_SORT_DIR_KEY: &str = "album_sort_dir";

pub const VIEW_MODE_KEY: &str = "view_mode";

const WINDOW_WIDTH_KEY: &str = "window_width";
const WINDOW_HEIGHT_KEY: &str = "window_height";
const WINDOW_MAXIMIZED_KEY: &str = "window_maximized";

const COLUMN_ORDER_KEY: &str = "column_order";

const LEFT_SIDEBAR_OPEN_KEY: &str = "left_sidebar_open";
const LEFT_SIDEBAR_WIDTH_KEY: &str = "left_sidebar_width";
const RIGHT_SIDEBAR_OPEN_KEY: &str = "right_sidebar_open";
const RIGHT_SIDEBAR_WIDTH_KEY: &str = "right_sidebar_width";
const DEFAULT_SIDEBAR_WIDTH: i32 = 220;

/// The settings key holding `field`'s format string.
fn column_format_key(field: TrackField) -> String {
    format!("column_format_{}", field.as_key())
}

/// The settings key holding `field`'s fixed column width in pixels.
fn column_width_key(field: TrackField) -> String {
    format!("column_width_{}", field.as_key())
}

/// A typed façade over the persisted application settings stored in `Db`.
/// All methods return domain-appropriate defaults when the setting is absent
/// or stored with an unrecognised value.
pub struct Settings<'a>(&'a Db);

impl<'a> Settings<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self(db)
    }

    /// Persisted cover-size in px, clamped to `[COVER_SIZE_MIN, COVER_SIZE_MAX]`.
    /// Defaults to `COVER_SIZE_MIN` when unset.
    pub fn cover_size(&self) -> i32 {
        self.get(COVER_SIZE_KEY)
            .and_then(|s| s.parse::<i32>().ok())
            .map(|n| n.clamp(COVER_SIZE_MIN, COVER_SIZE_MAX))
            .unwrap_or(COVER_SIZE_MIN)
    }

    pub fn set_cover_size(&self, px: i32) {
        self.set(COVER_SIZE_KEY, &px.to_string());
    }

    /// Persisted playback volume 0–100. Defaults to 70.0 when unset.
    pub fn volume(&self) -> f64 {
        self.get(VOLUME_KEY)
            .and_then(|s| s.parse::<f64>().ok())
            .map(|v| v.clamp(0.0, 100.0))
            .unwrap_or(DEFAULT_VOLUME)
    }

    pub fn set_volume(&self, v: f64) {
        self.set(VOLUME_KEY, &v.to_string());
    }

    /// Persisted album-grid sort. Defaults to album-artist ascending.
    pub fn album_sort(&self) -> AlbumSort {
        let field = self
            .get(ALBUM_SORT_FIELD_KEY)
            .and_then(|k| AlbumSortField::from_key(&k))
            .unwrap_or(AlbumSortField::AlbumArtist);
        let direction = self
            .get(ALBUM_SORT_DIR_KEY)
            .and_then(|k| SortDirection::from_key(&k))
            .unwrap_or(SortDirection::Ascending);
        AlbumSort::new(field, direction)
    }

    pub fn set_album_sort(&self, sort: AlbumSort) {
        self.set(ALBUM_SORT_FIELD_KEY, sort.field.as_key());
        self.set(ALBUM_SORT_DIR_KEY, sort.direction.as_key());
    }

    /// Persisted queue as an ordered list of track ids. Returns an empty vec when
    /// the setting is absent or contains no parseable ids.
    pub fn queue_track_ids(&self) -> Vec<TrackId> {
        let Some(raw) = self.get(QUEUE_KEY) else {
            return Vec::new();
        };
        raw.split(',')
            .filter_map(|s| s.parse::<i64>().ok())
            .map(TrackId::new)
            .collect()
    }

    pub fn set_queue(&self, ids: &[TrackId]) {
        let encoded = ids
            .iter()
            .map(|id| id.value().to_string())
            .collect::<Vec<_>>()
            .join(",");
        self.set(QUEUE_KEY, &encoded);
    }

    /// Persisted current track id, or `None` when unset.
    pub fn queue_current_id(&self) -> Option<i64> {
        self.get(QUEUE_CURRENT_KEY)
            .and_then(|s| s.parse::<i64>().ok())
    }

    pub fn set_queue_current(&self, id: TrackId) {
        self.set(QUEUE_CURRENT_KEY, &id.value().to_string());
    }

    /// Persisted playback position in milliseconds. Defaults to 0.
    pub fn queue_position_millis(&self) -> u64 {
        self.get(QUEUE_POSITION_KEY)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    }

    pub fn set_queue_position_millis(&self, millis: u64) {
        self.set(QUEUE_POSITION_KEY, &millis.to_string());
    }

    /// Persisted view mode. Returns `None` when unset or unrecognised.
    pub fn view_mode(&self) -> Option<ViewMode> {
        self.get(VIEW_MODE_KEY)
            .as_deref()
            .and_then(ViewMode::from_name)
    }

    pub fn set_view_mode(&self, mode: ViewMode) {
        self.set(VIEW_MODE_KEY, mode.child_name());
    }

    /// Persisted window size in device pixels, or `None` on first run (both
    /// dimensions must be present and parse; a half-written pair is treated
    /// as absent).
    pub fn window_size(&self) -> Option<(i32, i32)> {
        let width = self.get(WINDOW_WIDTH_KEY)?.parse::<i32>().ok()?;
        let height = self.get(WINDOW_HEIGHT_KEY)?.parse::<i32>().ok()?;
        Some((width, height))
    }

    pub fn set_window_size(&self, width: i32, height: i32) {
        self.set(WINDOW_WIDTH_KEY, &width.to_string());
        self.set(WINDOW_HEIGHT_KEY, &height.to_string());
    }

    /// Persisted maximized state. Defaults to `false` when unset.
    pub fn window_maximized(&self) -> bool {
        self.get(WINDOW_MAXIMIZED_KEY).as_deref() == Some("true")
    }

    pub fn set_window_maximized(&self, maximized: bool) {
        self.set(
            WINDOW_MAXIMIZED_KEY,
            if maximized { "true" } else { "false" },
        );
    }

    /// Persisted track-list column preferences: which fields are visible, in
    /// what order, and each column's format string. Falls back to
    /// `ColumnPrefs::default()` when unset, when every listed field key is
    /// unrecognised (forward/backward compat), or when the stored order
    /// names no field at all.
    pub fn column_prefs(&self) -> ColumnPrefs {
        let Some(order) = self.get(COLUMN_ORDER_KEY) else {
            return ColumnPrefs::default();
        };
        let columns: Vec<ColumnConfig> = order
            .split(',')
            .filter_map(TrackField::from_key)
            .map(|field| {
                let format = self
                    .get(&column_format_key(field))
                    .and_then(|raw| format::parse(&raw).ok())
                    .unwrap_or_else(|| FormatExpr::field_only(field));
                let width = self.column_width(field);
                ColumnConfig {
                    field,
                    format,
                    width,
                }
            })
            .collect();
        if columns.is_empty() {
            ColumnPrefs::default()
        } else {
            ColumnPrefs::new(columns)
        }
    }

    /// Persists `prefs`: the visible field order, and every column's format
    /// string and width — written unconditionally rather than only when
    /// customized, since comparing against "the default" would need its own
    /// drift-prone bookkeeping for no real benefit at this scale.
    pub fn set_column_prefs(&self, prefs: &ColumnPrefs) {
        let order = prefs
            .columns()
            .iter()
            .map(|c| c.field.as_key())
            .collect::<Vec<_>>()
            .join(",");
        self.set(COLUMN_ORDER_KEY, &order);
        for column in prefs.columns() {
            self.set(&column_format_key(column.field), &column.format.to_string());
            self.set_column_width(column.field, column.width);
        }
    }

    /// Persisted fixed width for `field`'s column, or `None` for GTK's
    /// natural size (unset, or a stored value that no longer parses).
    fn column_width(&self, field: TrackField) -> Option<i32> {
        self.get(&column_width_key(field))
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<i32>().ok())
    }

    /// Persists `field`'s fixed column width in isolation — used when the
    /// user drags a header to resize it, so that one change doesn't need a
    /// full `ColumnPrefs` snapshot (and the caller doesn't need to rebuild
    /// the column view just to save a width it already applied natively).
    pub fn set_column_width(&self, field: TrackField, width: Option<i32>) {
        self.set(
            &column_width_key(field),
            &width.map(|w| w.to_string()).unwrap_or_default(),
        );
    }

    /// Persisted open/collapsed state of the left (filters) sidebar. Defaults
    /// to open when unset.
    pub fn left_sidebar_open(&self) -> bool {
        self.get(LEFT_SIDEBAR_OPEN_KEY).as_deref() != Some("false")
    }

    pub fn set_left_sidebar_open(&self, open: bool) {
        self.set(LEFT_SIDEBAR_OPEN_KEY, if open { "true" } else { "false" });
    }

    /// Persisted width in pixels of the left sidebar while open. Defaults to
    /// `DEFAULT_SIDEBAR_WIDTH` when unset or unparsable.
    pub fn left_sidebar_width(&self) -> i32 {
        self.get(LEFT_SIDEBAR_WIDTH_KEY)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(DEFAULT_SIDEBAR_WIDTH)
    }

    pub fn set_left_sidebar_width(&self, width: i32) {
        self.set(LEFT_SIDEBAR_WIDTH_KEY, &width.to_string());
    }

    /// Persisted open/collapsed state of the right (queue/folders/status)
    /// sidebar. Defaults to open when unset.
    pub fn right_sidebar_open(&self) -> bool {
        self.get(RIGHT_SIDEBAR_OPEN_KEY).as_deref() != Some("false")
    }

    pub fn set_right_sidebar_open(&self, open: bool) {
        self.set(RIGHT_SIDEBAR_OPEN_KEY, if open { "true" } else { "false" });
    }

    /// Persisted width in pixels of the right sidebar while open. Defaults to
    /// `DEFAULT_SIDEBAR_WIDTH` when unset or unparsable.
    pub fn right_sidebar_width(&self) -> i32 {
        self.get(RIGHT_SIDEBAR_WIDTH_KEY)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(DEFAULT_SIDEBAR_WIDTH)
    }

    pub fn set_right_sidebar_width(&self, width: i32) {
        self.set(RIGHT_SIDEBAR_WIDTH_KEY, &width.to_string());
    }

    fn get(&self, key: &str) -> Option<String> {
        self.0.get_setting(key).ok().flatten()
    }

    fn set(&self, key: &str, value: &str) {
        if let Err(e) = self.0.set_setting(key, value) {
            tracing::error!("Failed to save setting {key}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::Db;

    fn fresh() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[test]
    fn cover_size_defaults_to_min_when_unset() {
        let db = fresh();
        assert_eq!(Settings::new(&db).cover_size(), COVER_SIZE_MIN);
    }

    #[test]
    fn cover_size_round_trips() {
        let db = fresh();
        Settings::new(&db).set_cover_size(350);
        assert_eq!(Settings::new(&db).cover_size(), 350);
    }

    #[test]
    fn cover_size_clamps_below_min() {
        let db = fresh();
        Settings::new(&db).set_cover_size(10);
        assert_eq!(Settings::new(&db).cover_size(), COVER_SIZE_MIN);
    }

    #[test]
    fn cover_size_clamps_above_max() {
        let db = fresh();
        Settings::new(&db).set_cover_size(9999);
        assert_eq!(Settings::new(&db).cover_size(), COVER_SIZE_MAX);
    }

    #[test]
    fn volume_defaults_to_70_when_unset() {
        let db = fresh();
        assert!((Settings::new(&db).volume() - 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn volume_round_trips() {
        let db = fresh();
        Settings::new(&db).set_volume(42.0);
        assert!((Settings::new(&db).volume() - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn volume_clamps_above_100() {
        let db = fresh();
        Settings::new(&db).set_volume(150.0);
        assert!((Settings::new(&db).volume() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn album_sort_defaults_to_artist_ascending_when_unset() {
        let db = fresh();
        let sort = Settings::new(&db).album_sort();
        assert_eq!(sort.field, AlbumSortField::AlbumArtist);
        assert_eq!(sort.direction, SortDirection::Ascending);
    }

    #[test]
    fn album_sort_round_trips() {
        let db = fresh();
        let sort = AlbumSort::new(AlbumSortField::Year, SortDirection::Descending);
        Settings::new(&db).set_album_sort(sort);
        let loaded = Settings::new(&db).album_sort();
        assert_eq!(loaded.field, AlbumSortField::Year);
        assert_eq!(loaded.direction, SortDirection::Descending);
    }

    #[test]
    fn queue_track_ids_returns_empty_when_unset() {
        let db = fresh();
        assert!(Settings::new(&db).queue_track_ids().is_empty());
    }

    #[test]
    fn queue_track_ids_round_trips() {
        let db = fresh();
        let ids = vec![TrackId::new(3), TrackId::new(7), TrackId::new(1)];
        Settings::new(&db).set_queue(&ids);
        assert_eq!(Settings::new(&db).queue_track_ids(), ids);
    }

    #[test]
    fn queue_current_id_returns_none_when_unset() {
        let db = fresh();
        assert!(Settings::new(&db).queue_current_id().is_none());
    }

    #[test]
    fn queue_current_id_round_trips() {
        let db = fresh();
        Settings::new(&db).set_queue_current(TrackId::new(42));
        assert_eq!(Settings::new(&db).queue_current_id(), Some(42));
    }

    #[test]
    fn queue_position_millis_defaults_to_zero_when_unset() {
        let db = fresh();
        assert_eq!(Settings::new(&db).queue_position_millis(), 0);
    }

    #[test]
    fn queue_position_millis_round_trips() {
        let db = fresh();
        Settings::new(&db).set_queue_position_millis(90_000);
        assert_eq!(Settings::new(&db).queue_position_millis(), 90_000);
    }

    #[test]
    fn view_mode_returns_none_when_unset() {
        let db = fresh();
        assert!(Settings::new(&db).view_mode().is_none());
    }

    #[test]
    fn view_mode_round_trips() {
        let db = fresh();
        Settings::new(&db).set_view_mode(ViewMode::Grid);
        assert_eq!(Settings::new(&db).view_mode(), Some(ViewMode::Grid));
    }

    #[test]
    fn view_mode_returns_none_for_an_unrecognised_stored_value() {
        let db = fresh();
        db.set_setting(VIEW_MODE_KEY, "carousel").unwrap();
        assert!(Settings::new(&db).view_mode().is_none());
    }

    #[test]
    fn window_size_returns_none_when_unset() {
        let db = fresh();
        assert_eq!(Settings::new(&db).window_size(), None);
    }

    #[test]
    fn window_size_round_trips() {
        let db = fresh();
        Settings::new(&db).set_window_size(1440, 900);
        assert_eq!(Settings::new(&db).window_size(), Some((1440, 900)));
    }

    #[test]
    fn window_size_is_none_when_only_width_is_set() {
        let db = fresh();
        db.set_setting(WINDOW_WIDTH_KEY, "1440").unwrap();
        assert_eq!(Settings::new(&db).window_size(), None);
    }

    #[test]
    fn window_maximized_defaults_to_false_when_unset() {
        let db = fresh();
        assert!(!Settings::new(&db).window_maximized());
    }

    #[test]
    fn window_maximized_round_trips() {
        let db = fresh();
        Settings::new(&db).set_window_maximized(true);
        assert!(Settings::new(&db).window_maximized());
    }

    #[test]
    fn column_prefs_defaults_to_the_original_six_columns_when_unset() {
        let db = fresh();
        assert_eq!(Settings::new(&db).column_prefs(), ColumnPrefs::default());
    }

    #[test]
    fn column_prefs_round_trips_order_custom_format_and_width() {
        let db = fresh();
        let prefs = ColumnPrefs::new(vec![
            ColumnConfig {
                field: TrackField::Artist,
                format: format::parse("%artist% \u{2013} %album%").unwrap(),
                width: Some(280),
            },
            ColumnConfig::default_for(TrackField::Title),
        ]);
        Settings::new(&db).set_column_prefs(&prefs);
        assert_eq!(Settings::new(&db).column_prefs(), prefs);
    }

    #[test]
    fn column_prefs_skips_unrecognised_field_keys_in_the_stored_order() {
        let db = fresh();
        db.set_setting(COLUMN_ORDER_KEY, "title,not_a_real_field,artist")
            .unwrap();
        let fields: Vec<TrackField> = Settings::new(&db)
            .column_prefs()
            .columns()
            .iter()
            .map(|c| c.field)
            .collect();
        assert_eq!(fields, vec![TrackField::Title, TrackField::Artist]);
    }

    #[test]
    fn column_prefs_falls_back_to_default_format_when_stored_format_does_not_parse() {
        let db = fresh();
        db.set_setting(COLUMN_ORDER_KEY, "title").unwrap();
        db.set_setting("column_format_title", "%unterminated")
            .unwrap();
        let prefs = Settings::new(&db).column_prefs();
        assert_eq!(
            prefs.columns()[0].format,
            ColumnConfig::default_for(TrackField::Title).format
        );
    }

    #[test]
    fn column_prefs_falls_back_to_default_when_every_stored_key_is_unrecognised() {
        let db = fresh();
        db.set_setting(COLUMN_ORDER_KEY, "not_a_real_field")
            .unwrap();
        assert_eq!(Settings::new(&db).column_prefs(), ColumnPrefs::default());
    }

    #[test]
    fn column_prefs_defaults_to_natural_width_when_stored_width_does_not_parse() {
        let db = fresh();
        db.set_setting(COLUMN_ORDER_KEY, "title").unwrap();
        db.set_setting("column_width_title", "not_a_number")
            .unwrap();
        let prefs = Settings::new(&db).column_prefs();
        assert_eq!(prefs.columns()[0].width, None);
    }

    #[test]
    fn set_column_width_persists_in_isolation() {
        let db = fresh();
        Settings::new(&db).set_column_width(TrackField::Duration, Some(96));
        assert_eq!(
            Settings::new(&db).column_width(TrackField::Duration),
            Some(96)
        );
    }

    #[test]
    fn set_column_width_none_clears_a_previously_stored_width() {
        let db = fresh();
        let settings = Settings::new(&db);
        settings.set_column_width(TrackField::Duration, Some(96));
        settings.set_column_width(TrackField::Duration, None);
        assert_eq!(settings.column_width(TrackField::Duration), None);
    }

    #[test]
    fn left_sidebar_open_defaults_to_true_when_unset() {
        let db = fresh();
        assert!(Settings::new(&db).left_sidebar_open());
    }

    #[test]
    fn left_sidebar_open_round_trips_false() {
        let db = fresh();
        Settings::new(&db).set_left_sidebar_open(false);
        assert!(!Settings::new(&db).left_sidebar_open());
    }

    #[test]
    fn left_sidebar_width_defaults_when_unset() {
        let db = fresh();
        assert_eq!(
            Settings::new(&db).left_sidebar_width(),
            DEFAULT_SIDEBAR_WIDTH
        );
    }

    #[test]
    fn left_sidebar_width_round_trips() {
        let db = fresh();
        Settings::new(&db).set_left_sidebar_width(260);
        assert_eq!(Settings::new(&db).left_sidebar_width(), 260);
    }

    #[test]
    fn right_sidebar_open_defaults_to_true_when_unset() {
        let db = fresh();
        assert!(Settings::new(&db).right_sidebar_open());
    }

    #[test]
    fn right_sidebar_open_round_trips_false() {
        let db = fresh();
        Settings::new(&db).set_right_sidebar_open(false);
        assert!(!Settings::new(&db).right_sidebar_open());
    }

    #[test]
    fn right_sidebar_width_defaults_when_unset() {
        let db = fresh();
        assert_eq!(
            Settings::new(&db).right_sidebar_width(),
            DEFAULT_SIDEBAR_WIDTH
        );
    }

    #[test]
    fn right_sidebar_width_round_trips() {
        let db = fresh();
        Settings::new(&db).set_right_sidebar_width(300);
        assert_eq!(Settings::new(&db).right_sidebar_width(), 300);
    }

    proptest::proptest! {
        #[test]
        fn volume_round_trips_for_any_value_in_the_valid_range(v in 0.0f64..=100.0) {
            let db = fresh();
            Settings::new(&db).set_volume(v);
            proptest::prop_assert_eq!(Settings::new(&db).volume(), v);
        }
    }
}
