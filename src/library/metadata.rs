use std::borrow::Cow;

use lofty::file::TaggedFileExt;
use lofty::prelude::AudioFile;
use lofty::prelude::ItemKey;
use lofty::tag::Accessor;
use lofty::tag::Tag;

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
    Lofty {
        path: std::path::PathBuf,
        #[source]
        source: lofty::error::LoftyError,
    },
}

/// Reads audio metadata from `path`. Returns the track and any embedded cover art.
/// The returned `Track` has `id = TrackId(0)`.
///
/// # Errors
/// `MetadataError::Lofty` — format unrecognised or parse failure.
pub fn read(path: &TrackPath) -> Result<(Track, Option<AlbumArtData>), MetadataError> {
    let tagged = lofty::read_from_path(path.as_path()).map_err(|source| MetadataError::Lofty {
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
