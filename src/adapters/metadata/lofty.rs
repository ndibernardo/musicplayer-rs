use std::borrow::Cow;

use lofty::file::TaggedFileExt;
use lofty::prelude::AudioFile;
use lofty::tag::Accessor;
use lofty::tag::Tag;

use crate::domain::track::AlbumArtData;
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

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("lofty error reading {path}: {source}")]
    Lofty {
        path: std::path::PathBuf,
        #[source]
        source: lofty::error::LoftyError,
    },
}

/// Reads audio metadata from `path`. Returns a `Track` with `id = TrackId(0)`.
///
/// # Errors
/// `MetadataError::Lofty` — format unrecognised or parse failure.
pub fn read(path: &TrackPath) -> Result<Track, MetadataError> {
    let tagged = lofty::read_from_path(path.as_path()).map_err(|source| MetadataError::Lofty {
        path: path.as_path().to_path_buf(),
        source,
    })?;

    let duration = TrackDuration::from_millis(tagged.properties().duration().as_millis() as u64);
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    Ok(Track {
        id: TrackId::new(0),
        path: path.clone(),
        title: Title::new(str_field(tag, |t| t.title())),
        artist: Artist::new(str_field(tag, |t| t.artist())),
        album: AlbumTitle::new(str_field(tag, |t| t.album())),
        genre: Genre::new(str_field(tag, |t| t.genre())),
        track_number: TrackNumber::new(num_field(tag, |t| t.track())),
        disc_number: DiscNumber::new(num_field(tag, |t| t.disk())),
        // `year()` was removed in lofty 0.22+; extract the year from the Timestamp returned by `date()`.
        year: Year::new(tag.and_then(|t| t.date()).map(|ts| ts.year).unwrap_or(0)),
        art: tag
            .and_then(|t| t.pictures().first())
            .map(|pic| AlbumArtData::new(pic.data().to_vec())),
        duration,
    })
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
