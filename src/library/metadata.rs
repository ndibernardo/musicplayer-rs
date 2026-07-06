use std::borrow::Cow;
use std::path::PathBuf;

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::Picture;
use lofty::picture::PictureType;
use lofty::prelude::AudioFile;
use lofty::prelude::ItemKey;
use lofty::tag::Accessor;
use lofty::tag::Tag;
use lofty::tag::items::Timestamp;

use crate::library::track::AlbumArtData;
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

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("lofty error reading {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: lofty::error::LoftyError,
    },
    #[error("lofty error writing {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: lofty::error::LoftyError,
    },
    #[error("no tag could be created for {0}")]
    NoTag(PathBuf),
}

/// Reads audio metadata from `path`. Returns the track and any embedded cover art.
/// The returned `Track` has `id = TrackId(0)`.
///
/// # Errors
/// `MetadataError::Read` — format unrecognised or parse failure.
pub fn read(path: &TrackPath) -> Result<(Track, Option<AlbumArtData>), MetadataError> {
    let tagged = lofty::read_from_path(path.as_path()).map_err(|source| MetadataError::Read {
        path: path.as_path().to_path_buf(),
        source,
    })?;

    let duration = TrackDuration::from_millis(tagged.properties().duration().as_millis() as u64);
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let art = tag
        .and_then(|t| t.pictures().first())
        .map(|pic| AlbumArtData::new(pic.data().to_vec()));

    let track = Track {
        id: TrackId::new(0),
        path: path.clone(),
        title: Title::new(str_field(tag, |t| t.title())),
        artist: Artist::new(str_field(tag, |t| t.artist())),
        album_artist: Artist::new(key_field(tag, ItemKey::AlbumArtist)),
        album: AlbumTitle::new(str_field(tag, |t| t.album())),
        genre: Genre::new(str_field(tag, |t| t.genre())),
        composer: Composer::new(key_field(tag, ItemKey::Composer)),
        track_number: TrackNumber::new(num_field(tag, |t| t.track())),
        disc_number: DiscNumber::new(num_field(tag, |t| t.disk())),
        // `year()` was removed in lofty 0.22+; extract the year from the Timestamp returned by `date()`.
        year: Year::new(tag.and_then(|t| t.date()).map(|ts| ts.year).unwrap_or(0)),
        duration,
    };

    Ok((track, art))
}

fn str_field<F>(tag: Option<&Tag>, f: F) -> String
where
    F: Fn(&Tag) -> Option<Cow<str>>,
{
    tag.and_then(f).map_or_else(String::new, |s| s.into_owned())
}

fn num_field<F>(tag: Option<&Tag>, f: F) -> u32
where
    F: Fn(&Tag) -> Option<u32>,
{
    tag.and_then(f).unwrap_or(0)
}

/// Reads a string tag identified by `key` (fields the `Accessor` trait doesn't
/// expose, such as album artist and composer). Empty when absent.
fn key_field(tag: Option<&Tag>, key: ItemKey) -> String {
    tag.and_then(|t| t.get_string(key))
        .map_or_else(String::new, ToOwned::to_owned)
}

/// Writes `track`'s fields, and `art` if present, to the audio file at
/// `track.path`. Untouched tag data (comments, replaygain, etc.) survives —
/// only the fields `apply_fields` manages are set or removed. `art` replaces
/// every embedded picture; `None` leaves existing pictures untouched.
///
/// # Errors
/// `MetadataError::Write` — the file couldn't be read, parsed, or saved.
/// `MetadataError::NoTag` — the format rejected a freshly inserted tag.
pub fn write(track: &Track, art: Option<&AlbumArtData>) -> Result<(), MetadataError> {
    let path = track.path.as_path();
    let mut tagged = lofty::read_from_path(path).map_err(|source| MetadataError::Write {
        path: path.to_path_buf(),
        source,
    })?;

    let tag_type = tagged.primary_tag_type();
    if tagged.tag_mut(tag_type).is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .tag_mut(tag_type)
        .ok_or_else(|| MetadataError::NoTag(path.to_path_buf()))?;

    apply_fields(tag, track);
    if let Some(art) = art {
        while !tag.pictures().is_empty() {
            tag.remove_picture(0);
        }
        tag.push_picture(
            Picture::unchecked(art.as_bytes().to_vec())
                .pic_type(PictureType::CoverFront)
                .build(),
        );
    }

    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|source| MetadataError::Write {
            path: path.to_path_buf(),
            source,
        })
}

/// Sets `tag`'s common fields from `track`. Unknown domain values (empty
/// string / zero) remove the corresponding key rather than writing an empty
/// one, so an untagged field doesn't turn into a stored-empty tag frame.
fn apply_fields(tag: &mut Tag, track: &Track) {
    set_text(tag, track.title.as_str(), Tag::set_title, Tag::remove_title);
    set_text(
        tag,
        track.artist.as_str(),
        Tag::set_artist,
        Tag::remove_artist,
    );
    set_text(tag, track.album.as_str(), Tag::set_album, Tag::remove_album);
    set_text(tag, track.genre.as_str(), Tag::set_genre, Tag::remove_genre);
    set_key(tag, ItemKey::AlbumArtist, track.album_artist.as_str());
    set_key(tag, ItemKey::Composer, track.composer.as_str());
    set_number(
        tag,
        track.track_number.value(),
        Tag::set_track,
        Tag::remove_track,
    );
    set_number(
        tag,
        track.disc_number.value(),
        Tag::set_disk,
        Tag::remove_disk,
    );
    set_year(tag, track.year.value());
}

fn set_text(tag: &mut Tag, value: &str, set: fn(&mut Tag, String), remove: fn(&mut Tag)) {
    if value.is_empty() {
        remove(tag);
    } else {
        set(tag, value.to_owned());
    }
}

fn set_number(tag: &mut Tag, value: u32, set: fn(&mut Tag, u32), remove: fn(&mut Tag)) {
    if value == 0 {
        remove(tag);
    } else {
        set(tag, value);
    }
}

fn set_key(tag: &mut Tag, key: ItemKey, value: &str) {
    if value.is_empty() {
        tag.remove_key(key);
    } else {
        tag.insert_text(key, value.to_owned());
    }
}

fn set_year(tag: &mut Tag, year: u16) {
    if year == 0 {
        tag.remove_date();
    } else {
        tag.set_date(Timestamp {
            year,
            ..Timestamp::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use lofty::tag::TagType;

    use super::*;

    fn geogaddi_roygbiv() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/music/boc/geogaddi/roygbiv.flac").unwrap(),
            title: Title::new("Roygbiv"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new("Boards of Canada"),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Electronic"),
            composer: Composer::new("Michael Sandison"),
            track_number: TrackNumber::new(7),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
            duration: TrackDuration::from_secs(193),
        }
    }

    #[test]
    fn apply_fields_sets_the_accessor_fields() {
        let mut tag = Tag::new(TagType::Id3v2);
        apply_fields(&mut tag, &geogaddi_roygbiv());

        assert_eq!(tag.title().as_deref(), Some("Roygbiv"));
        assert_eq!(tag.artist().as_deref(), Some("Boards of Canada"));
        assert_eq!(tag.album().as_deref(), Some("Geogaddi"));
        assert_eq!(tag.genre().as_deref(), Some("Electronic"));
        assert_eq!(tag.track(), Some(7));
        assert_eq!(tag.disk(), Some(1));
        assert_eq!(tag.date().map(|ts| ts.year), Some(2002));
    }

    #[test]
    fn apply_fields_sets_the_item_key_fields() {
        let mut tag = Tag::new(TagType::Id3v2);
        apply_fields(&mut tag, &geogaddi_roygbiv());

        assert_eq!(
            tag.get_string(ItemKey::AlbumArtist),
            Some("Boards of Canada")
        );
        assert_eq!(tag.get_string(ItemKey::Composer), Some("Michael Sandison"));
    }

    #[test]
    fn apply_fields_removes_the_title_key_for_an_unknown_title() {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title(String::from("Stale Title"));

        let track = Track {
            title: Title::new(""),
            ..geogaddi_roygbiv()
        };
        apply_fields(&mut tag, &track);

        assert_eq!(tag.title(), None);
    }

    #[test]
    fn apply_fields_removes_the_track_number_key_for_an_unknown_track_number() {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_track(7);

        let track = Track {
            track_number: TrackNumber::new(0),
            ..geogaddi_roygbiv()
        };
        apply_fields(&mut tag, &track);

        assert_eq!(tag.track(), None);
    }

    #[test]
    fn apply_fields_removes_the_date_key_for_an_unknown_year() {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_date(Timestamp {
            year: 2002,
            ..Timestamp::default()
        });

        let track = Track {
            year: Year::new(0),
            ..geogaddi_roygbiv()
        };
        apply_fields(&mut tag, &track);

        assert_eq!(tag.date(), None);
    }

    #[test]
    fn apply_fields_removes_the_album_artist_key_for_an_unknown_album_artist() {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::AlbumArtist, String::from("Stale Artist"));

        let track = Track {
            album_artist: Artist::new(""),
            ..geogaddi_roygbiv()
        };
        apply_fields(&mut tag, &track);

        assert_eq!(tag.get_string(ItemKey::AlbumArtist), None);
    }
}
