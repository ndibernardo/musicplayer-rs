//! A foobar2000-style format-string mini-language for rendering a `Track` as
//! text: literal characters, `%field%` placeholders, and `[...]` optional
//! groups that vanish when every field inside them is absent. Parsing and
//! rendering are pure and headless — no GTK, no I/O.

use crate::library::track::Track;

/// A `Track` field a format string can reference via `%field%`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackField {
    Title,
    Artist,
    AlbumArtist,
    Album,
    Genre,
    Composer,
    TrackNumber,
    DiscNumber,
    Year,
    Duration,
}

impl TrackField {
    /// Every field, in the canonical order the column picker lists them.
    pub fn all() -> [TrackField; 10] {
        [
            TrackField::Title,
            TrackField::Artist,
            TrackField::AlbumArtist,
            TrackField::Album,
            TrackField::Genre,
            TrackField::Composer,
            TrackField::TrackNumber,
            TrackField::DiscNumber,
            TrackField::Year,
            TrackField::Duration,
        ]
    }

    /// The label shown for this field in the column picker and, by default,
    /// as a column header.
    pub fn label(self) -> &'static str {
        match self {
            TrackField::Title => "Title",
            TrackField::Artist => "Artist",
            TrackField::AlbumArtist => "Album Artist",
            TrackField::Album => "Album",
            TrackField::Genre => "Genre",
            TrackField::Composer => "Composer",
            TrackField::TrackNumber => "Track #",
            TrackField::DiscNumber => "Disc #",
            TrackField::Year => "Year",
            TrackField::Duration => "Duration",
        }
    }

    /// The name used inside `%...%` and persisted as a settings key.
    pub fn as_key(self) -> &'static str {
        match self {
            TrackField::Title => "title",
            TrackField::Artist => "artist",
            TrackField::AlbumArtist => "album_artist",
            TrackField::Album => "album",
            TrackField::Genre => "genre",
            TrackField::Composer => "composer",
            TrackField::TrackNumber => "track",
            TrackField::DiscNumber => "disc",
            TrackField::Year => "year",
            TrackField::Duration => "duration",
        }
    }

    /// Parses a `%field%` name or a persisted settings key, or `None` when it
    /// names no known field.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "title" => Some(TrackField::Title),
            "artist" => Some(TrackField::Artist),
            "album_artist" => Some(TrackField::AlbumArtist),
            "album" => Some(TrackField::Album),
            "genre" => Some(TrackField::Genre),
            "composer" => Some(TrackField::Composer),
            "track" => Some(TrackField::TrackNumber),
            "disc" => Some(TrackField::DiscNumber),
            "year" => Some(TrackField::Year),
            "duration" => Some(TrackField::Duration),
            _ => None,
        }
    }

    /// Whether `track` has no value for this field — the condition an
    /// enclosing `[...]` group checks to decide whether it vanishes.
    /// `Duration` is never absent: a zero duration is a known value, not an
    /// unset tag.
    fn is_absent(self, track: &Track) -> bool {
        match self {
            TrackField::Title => track.title.is_unknown(),
            TrackField::Artist => track.artist.is_unknown(),
            TrackField::AlbumArtist => track.album_artist.is_unknown(),
            TrackField::Album => track.album.as_str().is_empty(),
            TrackField::Genre => track.genre.as_str().is_empty(),
            TrackField::Composer => track.composer.is_unknown(),
            TrackField::TrackNumber => track.track_number.is_unknown(),
            TrackField::DiscNumber => track.disc_number.is_unknown(),
            TrackField::Year => track.year.is_unknown(),
            TrackField::Duration => false,
        }
    }

    /// Renders this field's value, or an empty string when absent — absence
    /// never leaks a literal placeholder like "Unknown".
    fn render(self, track: &Track) -> String {
        if self.is_absent(track) {
            return String::new();
        }
        match self {
            TrackField::Title => track.title.as_str().to_owned(),
            TrackField::Artist => track.artist.as_str().to_owned(),
            TrackField::AlbumArtist => track.album_artist.as_str().to_owned(),
            TrackField::Album => track.album.as_str().to_owned(),
            TrackField::Genre => track.genre.as_str().to_owned(),
            TrackField::Composer => track.composer.as_str().to_owned(),
            TrackField::TrackNumber => track.track_number.value().to_string(),
            TrackField::DiscNumber => track.disc_number.value().to_string(),
            TrackField::Year => track.year.value().to_string(),
            TrackField::Duration => format_duration_secs(track.duration.as_secs()),
        }
    }

    /// Compares two tracks by this field's typed value — never by rendered
    /// text, since e.g. `"10:00"` sorts before `"2:00"` lexicographically
    /// but not numerically. Drives column-header click-to-sort.
    pub fn compare(self, a: &Track, b: &Track) -> std::cmp::Ordering {
        match self {
            TrackField::Title => a.title.as_str().cmp(b.title.as_str()),
            TrackField::Artist => a.artist.as_str().cmp(b.artist.as_str()),
            TrackField::AlbumArtist => a.album_artist.as_str().cmp(b.album_artist.as_str()),
            TrackField::Album => a.album.as_str().cmp(b.album.as_str()),
            TrackField::Genre => a.genre.as_str().cmp(b.genre.as_str()),
            TrackField::Composer => a.composer.as_str().cmp(b.composer.as_str()),
            TrackField::TrackNumber => a.track_number.value().cmp(&b.track_number.value()),
            TrackField::DiscNumber => a.disc_number.value().cmp(&b.disc_number.value()),
            TrackField::Year => a.year.value().cmp(&b.year.value()),
            TrackField::Duration => a.duration.as_duration().cmp(&b.duration.as_duration()),
        }
    }
}

/// Formats a raw second count as `m:ss`. Kept private and duplicated from
/// `ui::format::format_duration_secs` rather than imported: `library/` must
/// compile with zero dependency on `ui/`, in either direction.
fn format_duration_secs(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// One node of a parsed format string.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FormatNode {
    Literal(String),
    Field(TrackField),
    /// Renders its children, or nothing at all when every `Field` reachable
    /// inside it (including nested groups) is absent.
    Optional(Vec<FormatNode>),
}

/// A parsed format string, ready to render against any `Track`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatExpr(Vec<FormatNode>);

impl FormatExpr {
    /// The bare `%field%` expression for `field` — infallible, since it never
    /// goes through the string parser. This is the default format a column
    /// slot uses before the user customizes it.
    pub fn field_only(field: TrackField) -> Self {
        Self(vec![FormatNode::Field(field)])
    }
}

impl std::fmt::Display for FormatExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_nodes(&self.0, f)
    }
}

fn write_nodes(nodes: &[FormatNode], f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    nodes.iter().try_for_each(|node| write_node(node, f))
}

fn write_node(node: &FormatNode, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match node {
        FormatNode::Literal(s) => s.chars().try_for_each(|c| {
            if matches!(c, '%' | '[' | ']' | '\\') {
                write!(f, "\\{c}")
            } else {
                write!(f, "{c}")
            }
        }),
        FormatNode::Field(field) => write!(f, "%{}%", field.as_key()),
        FormatNode::Optional(inner) => {
            write!(f, "[")?;
            write_nodes(inner, f)?;
            write!(f, "]")
        }
    }
}

/// Everything that can go wrong parsing a format string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatParseError {
    #[error("unknown field: %{0}%")]
    UnknownField(String),
    #[error("unterminated %field% placeholder")]
    UnterminatedField,
    #[error("unterminated [ group")]
    UnterminatedGroup,
    #[error("unexpected ] with no matching [")]
    UnexpectedGroupClose,
    #[error("trailing backslash with nothing to escape")]
    TrailingEscape,
}

/// Parses a format string. Grammar: literal text; `%field%` placeholders
/// naming a [`TrackField`]; `[...]` optional groups; `\` escapes the next
/// character literally (so `\%`, `\[`, `\]`, `\\` render as themselves).
pub fn parse(input: &str) -> Result<FormatExpr, FormatParseError> {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;
    let nodes = parse_nodes(&chars, &mut pos, false)?;
    Ok(FormatExpr(nodes))
}

fn parse_nodes(
    chars: &[char],
    pos: &mut usize,
    in_group: bool,
) -> Result<Vec<FormatNode>, FormatParseError> {
    let mut nodes = Vec::new();
    let mut literal = String::new();
    while *pos < chars.len() {
        match chars[*pos] {
            ']' if in_group => break,
            ']' => return Err(FormatParseError::UnexpectedGroupClose),
            '[' => {
                if !literal.is_empty() {
                    nodes.push(FormatNode::Literal(std::mem::take(&mut literal)));
                }
                *pos += 1;
                let inner = parse_nodes(chars, pos, true)?;
                if *pos >= chars.len() || chars[*pos] != ']' {
                    return Err(FormatParseError::UnterminatedGroup);
                }
                *pos += 1;
                nodes.push(FormatNode::Optional(inner));
            }
            '%' => {
                if !literal.is_empty() {
                    nodes.push(FormatNode::Literal(std::mem::take(&mut literal)));
                }
                *pos += 1;
                let start = *pos;
                while *pos < chars.len() && chars[*pos] != '%' {
                    *pos += 1;
                }
                if *pos >= chars.len() {
                    return Err(FormatParseError::UnterminatedField);
                }
                let name: String = chars[start..*pos].iter().collect();
                *pos += 1;
                let field =
                    TrackField::from_key(&name).ok_or(FormatParseError::UnknownField(name))?;
                nodes.push(FormatNode::Field(field));
            }
            '\\' => {
                *pos += 1;
                let Some(&escaped) = chars.get(*pos) else {
                    return Err(FormatParseError::TrailingEscape);
                };
                literal.push(escaped);
                *pos += 1;
            }
            c => {
                literal.push(c);
                *pos += 1;
            }
        }
    }
    if !literal.is_empty() {
        nodes.push(FormatNode::Literal(literal));
    }
    Ok(nodes)
}

/// Renders `expr` against `track`.
pub fn render(expr: &FormatExpr, track: &Track) -> String {
    render_nodes(&expr.0, track)
}

fn render_nodes(nodes: &[FormatNode], track: &Track) -> String {
    nodes.iter().map(|n| render_node(n, track)).collect()
}

fn render_node(node: &FormatNode, track: &Track) -> String {
    match node {
        FormatNode::Literal(s) => s.clone(),
        FormatNode::Field(field) => field.render(track),
        FormatNode::Optional(inner) => {
            if any_field_present(inner, track) {
                render_nodes(inner, track)
            } else {
                String::new()
            }
        }
    }
}

fn any_field_present(nodes: &[FormatNode], track: &Track) -> bool {
    nodes.iter().any(|n| match n {
        FormatNode::Literal(_) => false,
        FormatNode::Field(field) => !field.is_absent(track),
        FormatNode::Optional(inner) => any_field_present(inner, track),
    })
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
    use crate::library::track::TrackDuration;
    use crate::library::track::TrackId;
    use crate::library::track::TrackNumber;
    use crate::library::track::TrackPath;
    use crate::library::track::Year;

    fn geogaddi_track() -> Track {
        Track {
            id: TrackId::new(1),
            path: TrackPath::new("/home/user/Music/boc/geogaddi/01_alpha_and_omega.flac").unwrap(),
            title: Title::new("Alpha and Omega"),
            artist: Artist::new("Boards of Canada"),
            album_artist: Artist::new(""),
            album: AlbumTitle::new("Geogaddi"),
            genre: Genre::new("Ambient"),
            composer: Composer::new(""),
            duration: TrackDuration::from_secs(133),
            track_number: TrackNumber::new(1),
            disc_number: DiscNumber::new(1),
            year: Year::new(2002),
        }
    }

    #[test]
    fn parse_literal_text_only_renders_unchanged() {
        let expr = parse("Boards of Canada").unwrap();
        assert_eq!(render(&expr, &geogaddi_track()), "Boards of Canada");
    }

    #[test]
    fn parse_single_field_placeholder_substitutes_value() {
        let expr = parse("%artist%").unwrap();
        assert_eq!(render(&expr, &geogaddi_track()), "Boards of Canada");
    }

    #[test]
    fn parse_mixes_literal_text_and_fields() {
        let expr = parse("%artist% \u{2013} %title%").unwrap();
        assert_eq!(
            render(&expr, &geogaddi_track()),
            "Boards of Canada \u{2013} Alpha and Omega"
        );
    }

    #[test]
    fn parse_unknown_field_returns_error() {
        assert_eq!(
            parse("%bitrate%"),
            Err(FormatParseError::UnknownField("bitrate".to_string()))
        );
    }

    #[test]
    fn parse_unterminated_field_returns_error() {
        assert_eq!(parse("%artist"), Err(FormatParseError::UnterminatedField));
    }

    #[test]
    fn optional_group_renders_when_field_present() {
        let expr = parse("[%album% ]CD%disc%").unwrap();
        assert_eq!(render(&expr, &geogaddi_track()), "Geogaddi CD1");
    }

    #[test]
    fn optional_group_vanishes_when_every_field_inside_is_absent() {
        let expr = parse("[%album_artist% \u{2013} ]%title%").unwrap();
        // album_artist is empty on this fixture, so the whole group — its
        // literal text included — must not appear.
        assert_eq!(render(&expr, &geogaddi_track()), "Alpha and Omega");
    }

    #[test]
    fn nested_optional_groups_use_transitive_presence() {
        let expr = parse("[[%album_artist%]%album%]").unwrap();
        // The inner group's field is absent, but the outer group also
        // contains %album%, which is present, so the outer group renders.
        assert_eq!(render(&expr, &geogaddi_track()), "Geogaddi");
    }

    #[test]
    fn parse_unterminated_group_returns_error() {
        assert_eq!(parse("[%album%"), Err(FormatParseError::UnterminatedGroup));
    }

    #[test]
    fn parse_unexpected_group_close_returns_error() {
        assert_eq!(
            parse("%album%]"),
            Err(FormatParseError::UnexpectedGroupClose)
        );
    }

    #[test]
    fn escaped_reserved_characters_render_literally() {
        let expr = parse("100\\% \\[live\\]").unwrap();
        assert_eq!(render(&expr, &geogaddi_track()), "100% [live]");
    }

    #[test]
    fn parse_trailing_backslash_returns_error() {
        assert_eq!(parse("track\\"), Err(FormatParseError::TrailingEscape));
    }

    #[test]
    fn bare_field_renders_empty_not_a_placeholder_word_when_absent() {
        let expr = parse("%album_artist%").unwrap();
        assert_eq!(render(&expr, &geogaddi_track()), "");
    }

    #[test]
    fn duration_field_renders_as_minutes_colon_seconds() {
        let expr = parse("%duration%").unwrap();
        assert_eq!(render(&expr, &geogaddi_track()), "2:13");
    }

    #[test]
    fn display_round_trips_through_parse() {
        let original = parse("[%album% ]CD%disc%/%track% \\[100\\%\\]").unwrap();
        let reparsed = parse(&original.to_string()).unwrap();
        assert_eq!(original, reparsed);
    }

    #[test]
    fn all_lists_every_field_exactly_once() {
        let all = TrackField::all();
        assert_eq!(all.len(), 10);
        let unique: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(unique.len(), 10, "no field is listed twice");
    }

    #[test]
    fn label_is_human_readable_for_every_field() {
        assert_eq!(TrackField::Title.label(), "Title");
        assert_eq!(TrackField::AlbumArtist.label(), "Album Artist");
        assert_eq!(TrackField::TrackNumber.label(), "Track #");
    }

    #[test]
    fn as_key_round_trips_through_from_key_for_every_field() {
        for field in TrackField::all() {
            assert_eq!(TrackField::from_key(field.as_key()), Some(field));
        }
    }

    #[test]
    fn field_only_renders_the_same_as_a_parsed_bare_placeholder() {
        let via_constructor = FormatExpr::field_only(TrackField::Artist);
        let via_parser = parse("%artist%").unwrap();
        assert_eq!(
            render(&via_constructor, &geogaddi_track()),
            render(&via_parser, &geogaddi_track())
        );
    }

    #[test]
    fn field_only_display_matches_the_bare_placeholder_syntax() {
        assert_eq!(
            FormatExpr::field_only(TrackField::Album).to_string(),
            "%album%"
        );
    }

    #[test]
    fn compare_orders_by_title_lexicographically() {
        let a = geogaddi_track();
        let b = Track {
            title: Title::new("Beta Bends"),
            ..geogaddi_track()
        };
        assert_eq!(TrackField::Title.compare(&a, &b), std::cmp::Ordering::Less);
    }

    #[test]
    fn compare_orders_by_artist_lexicographically() {
        let a = geogaddi_track();
        let b = Track {
            artist: Artist::new("Emeralds"),
            ..geogaddi_track()
        };
        assert_eq!(TrackField::Artist.compare(&a, &b), std::cmp::Ordering::Less);
    }

    #[test]
    fn compare_orders_by_album_artist_lexicographically() {
        let a = geogaddi_track();
        let b = Track {
            album_artist: Artist::new("Various Artists"),
            ..geogaddi_track()
        };
        assert_eq!(
            TrackField::AlbumArtist.compare(&a, &b),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_orders_by_album_lexicographically() {
        let a = geogaddi_track();
        let b = Track {
            album: AlbumTitle::new("Music Has the Right to Children"),
            ..geogaddi_track()
        };
        assert_eq!(TrackField::Album.compare(&a, &b), std::cmp::Ordering::Less);
    }

    #[test]
    fn compare_orders_by_genre_lexicographically() {
        let a = geogaddi_track();
        let b = Track {
            genre: Genre::new("Electronic"),
            ..geogaddi_track()
        };
        assert_eq!(TrackField::Genre.compare(&a, &b), std::cmp::Ordering::Less);
    }

    #[test]
    fn compare_orders_by_composer_lexicographically() {
        let a = geogaddi_track();
        let b = Track {
            composer: Composer::new("Erik Satie"),
            ..geogaddi_track()
        };
        assert_eq!(
            TrackField::Composer.compare(&a, &b),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_orders_by_track_number_numerically_not_lexicographically() {
        let two = Track {
            track_number: TrackNumber::new(2),
            ..geogaddi_track()
        };
        let ten = Track {
            track_number: TrackNumber::new(10),
            ..geogaddi_track()
        };
        // "10" < "2" as strings, but 10 tracks after 2 numerically.
        assert_eq!(
            TrackField::TrackNumber.compare(&two, &ten),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_orders_by_disc_number_numerically() {
        let disc_one = geogaddi_track();
        let disc_two = Track {
            disc_number: DiscNumber::new(2),
            ..geogaddi_track()
        };
        assert_eq!(
            TrackField::DiscNumber.compare(&disc_one, &disc_two),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_orders_by_year_numerically() {
        let earlier = geogaddi_track();
        let later = Track {
            year: Year::new(2005),
            ..geogaddi_track()
        };
        assert_eq!(
            TrackField::Year.compare(&earlier, &later),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_orders_by_duration_numerically_not_by_rendered_text() {
        let short = geogaddi_track();
        let long = Track {
            duration: TrackDuration::from_secs(600),
            ..geogaddi_track()
        };
        // Rendered as "2:13" vs "10:00" — lexicographically "10:00" < "2:13",
        // but the comparator must order by actual duration instead.
        assert_eq!(
            TrackField::Duration.compare(&short, &long),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_is_equal_for_identical_tracks() {
        let track = geogaddi_track();
        assert_eq!(
            TrackField::Title.compare(&track, &track),
            std::cmp::Ordering::Equal
        );
    }

    proptest::proptest! {
        #[test]
        fn parse_never_panics_on_arbitrary_input(s in ".*") {
            let _ = parse(&s);
        }

        #[test]
        fn render_never_panics_on_arbitrary_valid_input(s in "[a-zA-Z0-9 %\\[\\]\\\\]{0,40}") {
            if let Ok(expr) = parse(&s) {
                let _ = render(&expr, &geogaddi_track());
            }
        }
    }
}
