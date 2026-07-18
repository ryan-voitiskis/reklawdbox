//! Genre BPM ranges, family membership, and within-family depth metadata.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Typical BPM range for a genre. Tracks outside this range are not necessarily
/// mistagged, but the suggestion should be treated with lower confidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BpmRange {
    pub typical_min: f64,
    pub typical_max: f64,
}

impl BpmRange {
    const fn new(typical_min: f64, typical_max: f64) -> Self {
        Self {
            typical_min,
            typical_max,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenreFamily {
    House,
    Techno,
    Bass,
    Hardcore,
    Downtempo,
    Other,
}

/// Combined genre metadata: family classification, depth within family, and
/// optional BPM range. Consolidates what was previously three parallel match
/// trees (`genre_family`, `genre_depth`, `genre_bpm_range`).
struct GenreMeta {
    family: GenreFamily,
    depth: u8,
    bpm: Option<BpmRange>,
}

/// Genres not in this table default to `GenreFamily::Other`, depth 0, no BPM range.
const GENRE_META: &[(&str, GenreMeta)] = &[
    // ── House family (depth: 1 = most energetic/driving, 5 = deepest/darkest) ──
    (
        "Disco",
        GenreMeta {
            family: GenreFamily::House,
            depth: 1,
            bpm: Some(BpmRange::new(110.0, 130.0)),
        },
    ),
    (
        "Speed Garage",
        GenreMeta {
            family: GenreFamily::House,
            depth: 1,
            bpm: Some(BpmRange::new(130.0, 140.0)),
        },
    ),
    (
        "Italo Disco",
        GenreMeta {
            family: GenreFamily::House,
            depth: 1,
            bpm: Some(BpmRange::new(118.0, 135.0)),
        },
    ),
    (
        "Italodance",
        GenreMeta {
            family: GenreFamily::House,
            depth: 1,
            bpm: Some(BpmRange::new(135.0, 145.0)),
        },
    ),
    (
        "Gospel House",
        GenreMeta {
            family: GenreFamily::House,
            depth: 2,
            bpm: Some(BpmRange::new(120.0, 128.0)),
        },
    ),
    (
        "Garage",
        GenreMeta {
            family: GenreFamily::House,
            depth: 2,
            bpm: Some(BpmRange::new(130.0, 138.0)),
        },
    ),
    (
        "Afro House",
        GenreMeta {
            family: GenreFamily::House,
            depth: 2,
            bpm: Some(BpmRange::new(118.0, 128.0)),
        },
    ),
    (
        "2-Step Garage",
        GenreMeta {
            family: GenreFamily::House,
            depth: 2,
            bpm: Some(BpmRange::new(128.0, 135.0)),
        },
    ),
    (
        "UK Funky",
        GenreMeta {
            family: GenreFamily::House,
            depth: 2,
            bpm: Some(BpmRange::new(125.0, 135.0)),
        },
    ),
    (
        "House",
        GenreMeta {
            family: GenreFamily::House,
            depth: 3,
            bpm: Some(BpmRange::new(120.0, 130.0)),
        },
    ),
    (
        "Tech House",
        GenreMeta {
            family: GenreFamily::House,
            depth: 3,
            bpm: Some(BpmRange::new(124.0, 132.0)),
        },
    ),
    (
        "Progressive House",
        GenreMeta {
            family: GenreFamily::House,
            depth: 4,
            bpm: Some(BpmRange::new(122.0, 132.0)),
        },
    ),
    (
        "Deep House",
        GenreMeta {
            family: GenreFamily::House,
            depth: 5,
            bpm: Some(BpmRange::new(118.0, 126.0)),
        },
    ),
    // ── Techno family ──
    (
        "Hard Techno",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 1,
            bpm: Some(BpmRange::new(145.0, 160.0)),
        },
    ),
    (
        "Psytrance",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 1,
            bpm: Some(BpmRange::new(138.0, 148.0)),
        },
    ),
    (
        "Acid",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 2,
            bpm: Some(BpmRange::new(120.0, 145.0)),
        },
    ),
    (
        "EBM",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 2,
            bpm: Some(BpmRange::new(110.0, 140.0)),
        },
    ),
    (
        "Electro",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 2,
            bpm: None,
        },
    ),
    (
        "Trance",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 2,
            bpm: Some(BpmRange::new(136.0, 145.0)),
        },
    ),
    (
        "Techno",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 3,
            bpm: Some(BpmRange::new(128.0, 140.0)),
        },
    ),
    (
        "Minimal",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 4,
            bpm: Some(BpmRange::new(120.0, 132.0)),
        },
    ),
    (
        "Deep Techno",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 5,
            bpm: Some(BpmRange::new(120.0, 132.0)),
        },
    ),
    (
        "Dub Techno",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 6,
            bpm: Some(BpmRange::new(118.0, 132.0)),
        },
    ),
    (
        "Ambient Techno",
        GenreMeta {
            family: GenreFamily::Techno,
            depth: 7,
            bpm: Some(BpmRange::new(110.0, 130.0)),
        },
    ),
    // ── Hardcore family (1 = most commercial/euphoric, 5 = most raw/aggressive) ──
    (
        "Hardstyle",
        GenreMeta {
            family: GenreFamily::Hardcore,
            depth: 1,
            bpm: Some(BpmRange::new(148.0, 160.0)),
        },
    ),
    (
        "Happy Hardcore",
        GenreMeta {
            family: GenreFamily::Hardcore,
            depth: 2,
            bpm: Some(BpmRange::new(165.0, 180.0)),
        },
    ),
    (
        "Hard Trance",
        GenreMeta {
            family: GenreFamily::Hardcore,
            depth: 3,
            bpm: Some(BpmRange::new(138.0, 150.0)),
        },
    ),
    (
        "Hardcore",
        GenreMeta {
            family: GenreFamily::Hardcore,
            depth: 4,
            bpm: Some(BpmRange::new(160.0, 180.0)),
        },
    ),
    (
        "Gabber",
        GenreMeta {
            family: GenreFamily::Hardcore,
            depth: 5,
            bpm: Some(BpmRange::new(160.0, 190.0)),
        },
    ),
    // ── Bass family ──
    (
        "Grime",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 1,
            bpm: Some(BpmRange::new(138.0, 145.0)),
        },
    ),
    (
        "Bassline",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 1,
            bpm: Some(BpmRange::new(130.0, 142.0)),
        },
    ),
    (
        "Drum & Bass",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 2,
            bpm: Some(BpmRange::new(168.0, 180.0)),
        },
    ),
    (
        "Footwork",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 2,
            bpm: Some(BpmRange::new(155.0, 165.0)),
        },
    ),
    (
        "Jungle",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 3,
            bpm: Some(BpmRange::new(160.0, 175.0)),
        },
    ),
    (
        "Breakbeat",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 3,
            bpm: None,
        },
    ),
    (
        "Dubstep",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 4,
            bpm: Some(BpmRange::new(136.0, 144.0)),
        },
    ),
    (
        "Future Garage",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 5,
            bpm: Some(BpmRange::new(125.0, 138.0)),
        },
    ),
    (
        "Broken Beat",
        GenreMeta {
            family: GenreFamily::Bass,
            depth: 5,
            bpm: None,
        },
    ),
    // ── Downtempo family ──
    (
        "IDM",
        GenreMeta {
            family: GenreFamily::Downtempo,
            depth: 2,
            bpm: None,
        },
    ),
    (
        "Experimental",
        GenreMeta {
            family: GenreFamily::Downtempo,
            depth: 2,
            bpm: None,
        },
    ),
    (
        "Downtempo",
        GenreMeta {
            family: GenreFamily::Downtempo,
            depth: 3,
            bpm: Some(BpmRange::new(80.0, 115.0)),
        },
    ),
    (
        "Trip-Hop",
        GenreMeta {
            family: GenreFamily::Downtempo,
            depth: 3,
            bpm: Some(BpmRange::new(80.0, 100.0)),
        },
    ),
    (
        "Dub",
        GenreMeta {
            family: GenreFamily::Downtempo,
            depth: 4,
            bpm: Some(BpmRange::new(60.0, 90.0)),
        },
    ),
    (
        "Ambient",
        GenreMeta {
            family: GenreFamily::Downtempo,
            depth: 5,
            bpm: None,
        },
    ),
    // ── Other (has BPM range but no family/depth) ──
    (
        "Dancehall",
        GenreMeta {
            family: GenreFamily::Other,
            depth: 0,
            bpm: Some(BpmRange::new(85.0, 108.0)),
        },
    ),
];

fn genre_meta_map() -> &'static HashMap<&'static str, GenreMeta> {
    static MAP: OnceLock<HashMap<&'static str, GenreMeta>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::with_capacity(GENRE_META.len());
        for &(genre, ref meta) in GENRE_META {
            let previous = map.insert(
                genre,
                GenreMeta {
                    family: meta.family,
                    depth: meta.depth,
                    bpm: meta.bpm,
                },
            );
            assert!(
                previous.is_none(),
                "duplicate GENRE_META entry for '{genre}'"
            );
        }
        map
    })
}

/// Returns the typical BPM range for a canonical genre, if BPM is diagnostic for that genre.
/// Returns `None` for genres where BPM spread is too wide to be useful (e.g. IDM, Jazz, Experimental).
pub fn genre_bpm_range(canonical: &str) -> Option<BpmRange> {
    genre_meta_map().get(canonical).and_then(|m| m.bpm)
}

/// Depth score within a genre's family. Higher = deeper/darker/more atmospheric.
///
/// Used by the classification decision tree (C rule 9a) when two same-family genres
/// disagree: if audio is atmospheric/low_energy, prefer the higher-depth genre;
/// if audio is high_energy, prefer the lower-depth genre. Equal depth → present both.
///
/// Audio profile validates specificity claims: e.g. if enrichment says "Deep Techno"
/// but audio is high_energy (not atmospheric), the server prefers "Techno" instead.
pub fn genre_depth(canonical: &str) -> u8 {
    genre_meta_map()
        .get(canonical)
        .map(|m| m.depth)
        .unwrap_or(0)
}

/// Non-canonical names fall through to `Other`.
pub fn genre_family(canonical: &str) -> GenreFamily {
    genre_meta_map()
        .get(canonical)
        .map(|m| m.family)
        .unwrap_or(GenreFamily::Other)
}
