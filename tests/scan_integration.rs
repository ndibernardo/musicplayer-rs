//! End-to-end coverage of the public scan API: scan a real temp directory,
//! mutate a file on disk, rescan, and confirm the incremental-skip and
//! pruning behaviour through `library::scan` and `library::db` alone — no
//! internal test-only hooks.

use musicplayer_rs::library::db::Db;
use musicplayer_rs::library::db::LibraryFolder;
use musicplayer_rs::library::scan::ScannedFile;
use musicplayer_rs::library::scan::scan_folder;
use musicplayer_rs::library::track::AlbumTitle;
use musicplayer_rs::library::track::Artist;
use musicplayer_rs::library::track::Composer;
use musicplayer_rs::library::track::DiscNumber;
use musicplayer_rs::library::track::Genre;
use musicplayer_rs::library::track::Title;
use musicplayer_rs::library::track::Track;
use musicplayer_rs::library::track::TrackDuration;
use musicplayer_rs::library::track::TrackId;
use musicplayer_rs::library::track::TrackNumber;
use musicplayer_rs::library::track::TrackPath;
use musicplayer_rs::library::track::Year;

fn boc_track(path: &TrackPath) -> ScannedFile {
    ScannedFile {
        track: Track {
            id: TrackId::new(0),
            path: path.clone(),
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
        },
        art: None,
    }
}

#[test]
fn scan_mutate_rescan_reflects_incremental_skip_and_pruning() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("roygbiv.flac");
    std::fs::write(&file, b"original bytes").unwrap();

    let db = Db::open_in_memory().unwrap();
    let folder = LibraryFolder::new(dir.path()).unwrap();

    // First scan: the file is new, so it must be indexed.
    let first = scan_folder(&folder, &db, |p| Some(boc_track(p))).unwrap();
    assert_eq!(first.indexed, 1, "new file is indexed on first scan");
    assert_eq!(db.track_count().unwrap(), 1);

    // Second scan, nothing changed on disk: the file must be skipped.
    let second = scan_folder(&folder, &db, |p| Some(boc_track(p))).unwrap();
    assert_eq!(second.indexed, 0, "unchanged file is not re-indexed");
    assert_eq!(
        second.unchanged, 1,
        "unchanged file is counted as unchanged"
    );

    // Mutate the file on disk: size changes, so the next scan must re-index it.
    std::fs::write(&file, b"mutated bytes with a different length").unwrap();
    let third = scan_folder(&folder, &db, |p| Some(boc_track(p))).unwrap();
    assert_eq!(third.indexed, 1, "mutated file is re-indexed");
    assert_eq!(db.track_count().unwrap(), 1, "still one row — same path");

    // Remove the file: the next scan must prune it from the library.
    std::fs::remove_file(&file).unwrap();
    let fourth = scan_folder(&folder, &db, |p| Some(boc_track(p))).unwrap();
    assert_eq!(fourth.removed, 1, "deleted file is pruned");
    assert_eq!(db.track_count().unwrap(), 0);
}
