use crate::adapters::db::sqlite::Db;
use crate::adapters::db::sqlite::DbError;
use crate::domain::track::Track;

pub fn all_tracks(db: &Db) -> Result<Vec<Track>, DbError> {
    db.list_tracks()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::db::sqlite::Db;
    use crate::domain::track::AlbumTitle;
    use crate::domain::track::Artist;
    use crate::domain::track::DiscNumber;
    use crate::domain::track::Genre;
    use crate::domain::track::Title;
    use crate::domain::track::Track;
    use crate::domain::track::TrackDuration;
    use crate::domain::track::TrackId;
    use crate::domain::track::TrackNumber;
    use crate::domain::track::TrackPath;
    use crate::domain::track::Year;

    fn boc_track(path: &str, track_number: u32) -> Track {
        Track {
            id: TrackId::new(0),
            path: TrackPath::new(path).unwrap(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Music Has the Right to Children"),
            genre: Genre::new("Electronic"),
            duration: TrackDuration::from_secs(193),
            track_number: TrackNumber::new(track_number),
            disc_number: DiscNumber::new(1),
            year: Year::new(1998),
            art: None,
        }
    }

    #[test]
    fn all_tracks_returns_empty_for_fresh_db() {
        let db = Db::open_in_memory().unwrap();
        assert!(all_tracks(&db).unwrap().is_empty());
    }

    #[test]
    fn all_tracks_returns_inserted_track() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&boc_track("/music/boc/roygbiv.flac", 7))
            .unwrap();
        let tracks = all_tracks(&db).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_str(), "Roygbiv");
    }

    #[test]
    fn all_tracks_returns_tracks_ordered_by_artist_album_track() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track(&boc_track("/music/boc/track03.flac", 3))
            .unwrap();
        db.upsert_track(&boc_track("/music/boc/track01.flac", 1))
            .unwrap();
        db.upsert_track(&boc_track("/music/boc/track02.flac", 2))
            .unwrap();

        let tracks = all_tracks(&db).unwrap();
        assert_eq!(tracks[0].track_number.value(), 1);
        assert_eq!(tracks[1].track_number.value(), 2);
        assert_eq!(tracks[2].track_number.value(), 3);
    }
}
