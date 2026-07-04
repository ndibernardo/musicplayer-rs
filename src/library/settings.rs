use crate::library::album::AlbumSort;
use crate::library::album::AlbumSortField;
use crate::library::album::SortDirection;
use crate::library::db::Db;
use crate::library::track::TrackId;

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

    /// Persisted view-mode name (the raw string stored by the UI layer).
    /// Returns `None` when unset; the UI converts to its `ViewMode` type.
    pub fn view_mode_name(&self) -> Option<String> {
        self.get(VIEW_MODE_KEY)
    }

    pub fn set_view_mode_name(&self, name: &str) {
        self.set(VIEW_MODE_KEY, name);
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
    fn view_mode_name_returns_none_when_unset() {
        let db = fresh();
        assert!(Settings::new(&db).view_mode_name().is_none());
    }

    #[test]
    fn view_mode_name_round_trips() {
        let db = fresh();
        Settings::new(&db).set_view_mode_name("grid");
        assert_eq!(Settings::new(&db).view_mode_name().as_deref(), Some("grid"));
    }
}
