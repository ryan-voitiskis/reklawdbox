use std::collections::HashMap;
use std::sync::OnceLock;

/// The starter genre taxonomy. Not a closed list — arbitrary genres are accepted.
/// This list serves as a reference for consistency and auto-complete suggestions.
pub const GENRES: &[&str] = &[
    "Acid",
    "Afro House",
    "Ambient",
    "Ambient Techno",
    "Bassline",
    "Breakbeat",
    "Broken Beat",
    "Dancehall",
    "Deep House",
    "Deep Techno",
    "Disco",
    "Downtempo",
    "Drone Techno",
    "Drum & Bass",
    "Dub",
    "Dub Reggae",
    "Dub Techno",
    "Dubstep",
    "Electro",
    "Experimental",
    "Garage",
    "Gospel House",
    "Grime",
    "Hard Techno",
    "Highlife",
    "Hip Hop",
    "House",
    "IDM",
    "Jazz",
    "Jungle",
    "Minimal",
    "Pop",
    "Progressive House",
    "Psytrance",
    "R&B",
    "Reggae",
    "Rock",
    "Speed Garage",
    "Synth-pop",
    "Tech House",
    "Techno",
    "Trance",
    "UK Bass",
];

/// Returns the canonical casing of a genre if it's in the taxonomy.
pub fn canonical_genre_name(genre: &str) -> Option<&'static str> {
    let genre = genre.trim();
    GENRES
        .iter()
        .find(|g| g.eq_ignore_ascii_case(genre))
        .copied()
}

pub fn is_known_genre(genre: &str) -> bool {
    canonical_genre_name(genre).is_some()
}

/// Alias entries mapping non-canonical genre strings to canonical genres.
/// Keys must be lowercase ASCII. Sorted alphabetically by key.
pub const ALIASES: &[(&str, &str)] = &[
    ("140 / deep dubstep / grime", "Dubstep"),
    ("breaks / breakbeat / uk bass", "Breakbeat"),
    ("chill dnb", "Drum & Bass"),
    ("dance / electro pop", "Synth-pop"),
    ("dance-pop", "Synth-pop"),
    ("dnb", "Drum & Bass"),
    ("gabber", "Hard Techno"),
    ("garage house", "House"),
    ("glitch", "IDM"),
    ("hard dance", "Hard Techno"),
    ("hard trance", "Trance"),
    ("hip-hop", "Hip Hop"),
    ("italodance", "Disco"),
    ("loop (hip-hop)", "Hip Hop"),
    ("loop (trance)", "Trance"),
    ("mainstage", "Trance"),
    ("melodic house & techno", "Deep Techno"),
    ("minimal / deep tech", "Minimal"),
    ("minimal techno", "Minimal"),
    ("r & b", "R&B"),
    ("tech trance", "Trance"),
    ("techno (peak time / driving)", "Techno"),
    ("techno (raw / deep / hypnotic)", "Deep Techno"),
    ("trance (main floor)", "Trance"),
    ("trance (raw / deep / hypnotic)", "Trance"),
    ("uk garage", "Garage"),
];

/// Label-to-genre mapping. Keys are lowercase ASCII label names.
/// Only labels with a strong, unambiguous primary genre.
/// Sorted alphabetically by key.
pub const LABEL_GENRES: &[(&str, &str)] = &[
    ("20/20 ldn", "Garage"),
    ("afterlife", "Deep Techno"),
    ("anjunabeats", "Trance"),
    ("anjunadeep", "Deep House"),
    ("aus music", "House"),
    ("basic channel", "Dub Techno"),
    ("bass face", "Drum & Bass"),
    ("bingo bass", "Bassline"),
    ("black sun empire", "Drum & Bass"),
    ("boysnoize", "Electro"),
    ("cadenza", "Minimal"),
    ("cocoon", "Techno"),
    ("compa", "Dubstep"),
    ("critical music", "Drum & Bass"),
    ("crosstown rebels", "House"),
    ("deep medi musik", "Dubstep"),
    ("defected", "House"),
    ("dekmantel", "Techno"),
    ("delsin", "Deep Techno"),
    ("dirtybird", "Tech House"),
    ("dispatch", "Drum & Bass"),
    ("drumcode", "Techno"),
    ("echospace", "Dub Techno"),
    ("ed banger", "Electro"),
    ("fabric", "Techno"),
    ("fuse london", "Tech House"),
    ("ghost orchid", "Ambient"),
    ("good looking", "Drum & Bass"),
    ("hardgroove", "Hard Techno"),
    ("hessle audio", "UK Bass"),
    ("hospital", "Drum & Bass"),
    ("hotflush", "UK Bass"),
    ("hyperdub", "UK Bass"),
    ("ilian tape", "Techno"),
    ("klockworks", "Techno"),
    ("kompakt", "Minimal"),
    ("lobster theremin", "House"),
    ("mac ii", "Drum & Bass"),
    ("mala", "Dubstep"),
    ("metalheadz", "Drum & Bass"),
    ("mord", "Hard Techno"),
    ("mote-evolver", "Deep Techno"),
    ("mute", "Synth-pop"),
    ("ninja tune", "Downtempo"),
    ("non standard", "Experimental"),
    ("nonplus", "Techno"),
    ("objective", "Techno"),
    ("ostgut ton", "Techno"),
    ("pampa", "House"),
    ("perlon", "Minimal"),
    ("planet mu", "IDM"),
    ("planet rhythm", "Electro"),
    ("power house", "Deep House"),
    ("r&s", "Techno"),
    ("ram", "Drum & Bass"),
    ("raster-noton", "Experimental"),
    ("running back", "Disco"),
    ("rushed", "Hard Techno"),
    ("semantica", "Deep Techno"),
    ("shogun audio", "Drum & Bass"),
    ("soma", "Techno"),
    ("soul clap", "Disco"),
    ("south london hi-fi", "Dub Reggae"),
    ("stroboscopic artefacts", "Techno"),
    ("suara", "Tech House"),
    ("subtle audio", "Drum & Bass"),
    ("swamp 81", "UK Bass"),
    ("tectonic", "UK Bass"),
    ("tempa", "Dubstep"),
    ("timedance", "UK Bass"),
    ("toolroom", "Tech House"),
    ("tresor", "Techno"),
    ("truesoul", "Techno"),
    ("type", "Ambient"),
    ("upsammy", "Experimental"),
    ("visionquest", "Deep House"),
];

fn build_alias_map(aliases: &[(&str, &'static str)]) -> HashMap<String, &'static str> {
    let mut map = HashMap::with_capacity(aliases.len());
    for &(alias, canonical) in aliases {
        assert_eq!(
            alias,
            alias.trim(),
            "alias '{}' has leading/trailing whitespace",
            alias
        );
        assert!(alias.is_ascii(), "alias '{}' must be ASCII", alias);
        assert_eq!(
            alias,
            alias.to_ascii_lowercase(),
            "alias '{}' must be lowercase ASCII",
            alias
        );
        let key = alias.to_string();
        let previous = map.insert(key.clone(), canonical);
        assert!(
            previous.is_none(),
            "duplicate alias key '{}' (case-insensitive)",
            key
        );
    }
    map
}

/// Static alias map built once via OnceLock. Maps lowercase ASCII alias → canonical genre.
pub fn genre_alias_map() -> &'static HashMap<String, &'static str> {
    static MAP: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| build_alias_map(ALIASES))
}

/// Returns the canonical genre if the input is a known alias, `None` if already canonical or unknown.
pub fn canonical_genre_from_alias(genre: &str) -> Option<&'static str> {
    genre_alias_map()
        .get(&genre.trim().to_ascii_lowercase())
        .copied()
}

/// Static label-genre map built once via OnceLock. Maps lowercase ASCII label name → canonical genre.
pub fn label_genre_map() -> &'static HashMap<String, &'static str> {
    static MAP: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| build_alias_map(LABEL_GENRES))
}

const LABEL_SUFFIXES: &[&str] = &[
    " records",
    " recordings",
    " music",
    " audio",
    " sound",
    " label",
];

/// Returns the canonical genre for a label name, if known.
/// Tries exact match first, then strips common suffixes.
/// Uses ASCII case-folding only; non-ASCII characters are compared byte-for-byte.
pub fn label_genre(label: &str) -> Option<&'static str> {
    let normalized = label.trim().to_ascii_lowercase();
    if let Some(&genre) = label_genre_map().get(&normalized) {
        return Some(genre);
    }
    for suffix in LABEL_SUFFIXES {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            if let Some(&genre) = label_genre_map().get(stripped) {
                return Some(genre);
            }
        }
    }
    None
}

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

/// Returns the typical BPM range for a canonical genre, if BPM is diagnostic for that genre.
/// Returns `None` for genres where BPM spread is too wide to be useful (e.g. IDM, Jazz, Experimental).
pub fn genre_bpm_range(canonical: &str) -> Option<BpmRange> {
    match canonical {
        "Acid" => Some(BpmRange::new(120.0, 145.0)),
        "Afro House" => Some(BpmRange::new(118.0, 128.0)),
        "Ambient Techno" => Some(BpmRange::new(110.0, 130.0)),
        "Bassline" => Some(BpmRange::new(130.0, 142.0)),
        "Dancehall" => Some(BpmRange::new(85.0, 108.0)),
        "Deep House" => Some(BpmRange::new(118.0, 126.0)),
        "Deep Techno" => Some(BpmRange::new(120.0, 132.0)),
        "Disco" => Some(BpmRange::new(110.0, 130.0)),
        "Downtempo" => Some(BpmRange::new(80.0, 115.0)),
        "Drone Techno" => Some(BpmRange::new(115.0, 135.0)),
        "Drum & Bass" => Some(BpmRange::new(168.0, 180.0)),
        "Dub" => Some(BpmRange::new(60.0, 90.0)),
        "Dub Reggae" => Some(BpmRange::new(60.0, 90.0)),
        "Dub Techno" => Some(BpmRange::new(118.0, 132.0)),
        "Dubstep" => Some(BpmRange::new(136.0, 144.0)),
        "Garage" => Some(BpmRange::new(130.0, 138.0)),
        "Gospel House" => Some(BpmRange::new(120.0, 128.0)),
        "Grime" => Some(BpmRange::new(138.0, 145.0)),
        "Hard Techno" => Some(BpmRange::new(145.0, 160.0)),
        "House" => Some(BpmRange::new(120.0, 130.0)),
        "Jungle" => Some(BpmRange::new(160.0, 175.0)),
        "Minimal" => Some(BpmRange::new(120.0, 132.0)),
        "Progressive House" => Some(BpmRange::new(122.0, 132.0)),
        "Psytrance" => Some(BpmRange::new(138.0, 148.0)),
        "Speed Garage" => Some(BpmRange::new(130.0, 140.0)),
        "Tech House" => Some(BpmRange::new(124.0, 132.0)),
        "Techno" => Some(BpmRange::new(128.0, 140.0)),
        "Trance" => Some(BpmRange::new(136.0, 145.0)),
        "UK Bass" => Some(BpmRange::new(130.0, 145.0)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenreFamily {
    House,
    Techno,
    Bass,
    Downtempo,
    Other,
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
    match canonical {
        // House family — 1 = most energetic/driving, 5 = deepest/darkest
        "Disco" | "Speed Garage" => 1,
        "Gospel House" | "Garage" | "Afro House" => 2,
        "House" | "Tech House" => 3,
        "Progressive House" => 4,
        "Deep House" => 5,

        // Techno family
        "Hard Techno" | "Psytrance" => 1,
        "Acid" | "Electro" | "Trance" => 2,
        "Techno" => 3,
        "Minimal" => 4,
        "Deep Techno" => 5,
        "Dub Techno" => 6,
        "Ambient Techno" => 7,
        "Drone Techno" => 8,

        // Bass family
        "Grime" | "Bassline" => 1,
        "Drum & Bass" => 2,
        "Jungle" | "Breakbeat" => 3,
        "Dubstep" | "UK Bass" => 4,
        "Broken Beat" => 5,

        // Downtempo family
        "IDM" | "Experimental" => 2,
        "Downtempo" => 3,
        "Dub" | "Dub Reggae" => 4,
        "Ambient" => 5,

        // Other / unknown — no depth comparison possible
        _ => 0,
    }
}

/// Map a canonical genre name to its family. Input should be canonical
/// (via `canonical_genre_name` or `canonical_genre_from_alias`); non-canonical names
/// fall through to `Other`.
pub fn genre_family(canonical: &str) -> GenreFamily {
    match canonical {
        "House" | "Deep House" | "Tech House" | "Afro House" | "Gospel House"
        | "Progressive House" | "Garage" | "Speed Garage" | "Disco" => GenreFamily::House,

        "Techno" | "Deep Techno" | "Minimal" | "Dub Techno" | "Ambient Techno" | "Hard Techno"
        | "Drone Techno" | "Acid" | "Electro" | "Trance" | "Psytrance" => GenreFamily::Techno,

        "Drum & Bass" | "Jungle" | "Dubstep" | "Breakbeat" | "UK Bass" | "Grime" | "Bassline"
        | "Broken Beat" => GenreFamily::Bass,

        "Ambient" | "Downtempo" | "Dub" | "Dub Reggae" | "IDM" | "Experimental" => {
            GenreFamily::Downtempo
        }

        _ => GenreFamily::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn taxonomy_sorted() {
        let mut sorted = GENRES.to_vec();
        sorted.sort_by_key(|a| a.to_lowercase());
        assert_eq!(
            GENRES,
            sorted.as_slice(),
            "GENRES array must be sorted alphabetically"
        );
    }

    #[test]
    fn taxonomy_has_reasonable_size() {
        assert!(
            GENRES.len() >= 30,
            "taxonomy seems too small: {}",
            GENRES.len()
        );
    }

    #[test]
    fn known_genre_case_insensitive() {
        assert!(is_known_genre("deep house"));
        assert!(is_known_genre("Deep House"));
        assert!(is_known_genre("TECHNO"));
        assert!(is_known_genre("uk bass"));
        assert!(is_known_genre("R&B"));
        assert!(!is_known_genre("Polka"));
    }

    #[test]
    fn known_genre_trims_whitespace() {
        assert!(is_known_genre(" Techno"));
        assert!(is_known_genre("Techno "));
        assert!(is_known_genre("\tDeep House\t"));
    }

    #[test]
    fn normalize_known_aliases() {
        assert_eq!(canonical_genre_from_alias("Hip-Hop"), Some("Hip Hop"));
        assert_eq!(canonical_genre_from_alias("DnB"), Some("Drum & Bass"));
        assert_eq!(canonical_genre_from_alias("Gabber"), Some("Hard Techno"));
        assert_eq!(canonical_genre_from_alias("Glitch"), Some("IDM"));
        assert_eq!(
            canonical_genre_from_alias("140 / Deep Dubstep / Grime"),
            Some("Dubstep")
        );
    }

    #[test]
    fn normalize_case_insensitive() {
        assert_eq!(canonical_genre_from_alias("hip-hop"), Some("Hip Hop"));
        assert_eq!(canonical_genre_from_alias("HIP-HOP"), Some("Hip Hop"));
        assert_eq!(canonical_genre_from_alias("Hip-Hop"), Some("Hip Hop"));
        assert_eq!(canonical_genre_from_alias("dnb"), Some("Drum & Bass"));
        assert_eq!(canonical_genre_from_alias("DNB"), Some("Drum & Bass"));
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(canonical_genre_from_alias(" hip-hop"), Some("Hip Hop"));
        assert_eq!(canonical_genre_from_alias("HIP-HOP "), Some("Hip Hop"));
        assert_eq!(canonical_genre_from_alias("\tdnb\t"), Some("Drum & Bass"));
    }

    #[test]
    fn normalize_canonical_returns_none() {
        assert_eq!(canonical_genre_from_alias("Techno"), None);
        assert_eq!(canonical_genre_from_alias("Deep House"), None);
        assert_eq!(canonical_genre_from_alias("Drum & Bass"), None);
        assert_eq!(canonical_genre_from_alias("Hip Hop"), None);
        assert_eq!(canonical_genre_from_alias("Rock"), None);
        assert_eq!(canonical_genre_from_alias("Pop"), None);
    }

    #[test]
    fn normalize_unknown_returns_none() {
        assert_eq!(canonical_genre_from_alias("Polka"), None);
        assert_eq!(canonical_genre_from_alias("Anti-music"), None);
        assert_eq!(canonical_genre_from_alias("Zydeco"), None);
    }

    #[test]
    fn alias_map_not_empty() {
        let aliases = genre_alias_map();
        assert!(
            aliases.len() >= 23,
            "expected at least 23 aliases, got {}",
            aliases.len()
        );
    }

    #[test]
    fn aliases_sorted() {
        for w in ALIASES.windows(2) {
            assert!(
                w[0].0 <= w[1].0,
                "ALIASES not sorted: {:?} > {:?}",
                w[0].0,
                w[1].0
            );
        }
    }

    #[test]
    fn aliases_are_lowercase_and_casefold_unique() {
        let mut seen = HashSet::new();
        for &(alias, _) in ALIASES {
            assert!(alias.is_ascii(), "alias '{}' must be ASCII", alias);
            assert_eq!(
                alias,
                alias.to_ascii_lowercase(),
                "alias '{}' must be lowercase ASCII",
                alias
            );
            let inserted = seen.insert(alias.to_ascii_lowercase());
            assert!(
                inserted,
                "duplicate alias key '{}' (case-insensitive)",
                alias
            );
        }
    }

    #[test]
    fn non_ascii_aliases_are_rejected() {
        let result = std::panic::catch_unwind(|| {
            let _ = build_alias_map(&[("Électro", "Electro")]);
        });
        assert!(result.is_err(), "expected non-ASCII alias to panic");
    }

    #[test]
    fn all_alias_targets_are_canonical() {
        for &(alias, canonical) in ALIASES {
            assert!(
                is_known_genre(canonical),
                "alias '{}' maps to '{}' which is not in taxonomy",
                alias,
                canonical
            );
        }
    }

    #[test]
    fn no_alias_shadows_canonical() {
        for &(alias, target) in ALIASES {
            assert!(
                !is_known_genre(alias),
                "alias '{}' (-> '{}') shadows a canonical genre — remove it",
                alias,
                target
            );
        }
    }

    #[test]
    fn all_taxonomy_genres_have_family() {
        // Verify key family assignments to catch mapping errors
        assert_eq!(genre_family("House"), GenreFamily::House);
        assert_eq!(genre_family("Deep House"), GenreFamily::House);
        assert_eq!(genre_family("Techno"), GenreFamily::Techno);
        assert_eq!(genre_family("Hard Techno"), GenreFamily::Techno);
        assert_eq!(genre_family("Trance"), GenreFamily::Techno);
        assert_eq!(genre_family("Psytrance"), GenreFamily::Techno);
        assert_eq!(genre_family("Drum & Bass"), GenreFamily::Bass);
        assert_eq!(genre_family("Dubstep"), GenreFamily::Bass);
        assert_eq!(genre_family("Ambient"), GenreFamily::Downtempo);
        assert_eq!(genre_family("Downtempo"), GenreFamily::Downtempo);

        // Every taxonomy genre must resolve without panicking
        for g in GENRES {
            let _ = genre_family(g);
        }
    }

    #[test]
    fn bpm_ranges_are_valid() {
        for g in GENRES {
            if let Some(range) = genre_bpm_range(g) {
                assert!(
                    range.typical_min < range.typical_max,
                    "genre '{}' has invalid BPM range: {} >= {}",
                    g,
                    range.typical_min,
                    range.typical_max
                );
                assert!(
                    range.typical_min > 0.0,
                    "genre '{}' has non-positive typical_min: {}",
                    g,
                    range.typical_min
                );
            }
        }
    }

    #[test]
    fn label_genres_sorted() {
        for w in LABEL_GENRES.windows(2) {
            assert!(
                w[0].0 <= w[1].0,
                "LABEL_GENRES not sorted: {:?} > {:?}",
                w[0].0,
                w[1].0
            );
        }
    }

    #[test]
    fn label_genres_are_lowercase() {
        for &(label, _) in LABEL_GENRES {
            assert!(label.is_ascii(), "label '{}' must be ASCII", label);
            assert_eq!(
                label,
                label.to_ascii_lowercase(),
                "label '{}' must be lowercase ASCII",
                label
            );
        }
    }

    #[test]
    fn all_label_genre_targets_are_canonical() {
        for &(label, canonical) in LABEL_GENRES {
            assert!(
                is_known_genre(canonical),
                "label '{}' maps to '{}' which is not in taxonomy",
                label,
                canonical
            );
        }
    }

    #[test]
    fn no_label_shadows_alias() {
        let alias_map = genre_alias_map();
        for &(label, _) in LABEL_GENRES {
            assert!(
                !alias_map.contains_key(label),
                "label '{}' shadows an alias key",
                label
            );
        }
    }

    #[test]
    fn label_genre_exact_match() {
        assert_eq!(label_genre("mord"), Some("Hard Techno"));
        assert_eq!(label_genre("hospital"), Some("Drum & Bass"));
        assert_eq!(label_genre("kompakt"), Some("Minimal"));
    }

    #[test]
    fn label_genre_suffix_stripping() {
        assert_eq!(label_genre("Tresor Records"), Some("Techno"));
        assert_eq!(label_genre("hospital records"), Some("Drum & Bass"));
        assert_eq!(label_genre("cocoon recordings"), Some("Techno"));
    }

    #[test]
    fn label_genre_case_insensitive() {
        assert_eq!(label_genre("MORD"), Some("Hard Techno"));
        assert_eq!(label_genre("Hyperdub"), Some("UK Bass"));
        assert_eq!(label_genre("TRESOR"), Some("Techno"));
    }

    #[test]
    fn suffix_stripped_labels_are_consistent() {
        let map = label_genre_map();
        for &(label, genre) in LABEL_GENRES {
            for suffix in LABEL_SUFFIXES {
                if let Some(prefix) = label.strip_suffix(suffix) {
                    if let Some(&prefix_genre) = map.get(prefix) {
                        assert_eq!(
                            genre, prefix_genre,
                            "label '{}' maps to '{}' but prefix '{}' maps to '{}'",
                            label, genre, prefix, prefix_genre
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn label_genre_unknown_returns_none() {
        assert_eq!(label_genre("warp"), None);
        assert_eq!(label_genre("xl recordings"), None);
        assert_eq!(label_genre(""), None);
    }

    #[test]
    fn all_taxonomy_genres_have_depth() {
        for g in GENRES {
            let family = genre_family(g);
            let depth = genre_depth(g);
            if family != GenreFamily::Other {
                assert!(
                    depth > 0,
                    "genre '{}' (family {:?}) has depth 0 — add it to genre_depth()",
                    g,
                    family,
                );
            }
        }
    }

    #[test]
    fn depth_ordering_house() {
        assert!(genre_depth("Deep House") > genre_depth("House"));
        assert!(genre_depth("House") > genre_depth("Disco"));
    }

    #[test]
    fn depth_ordering_techno() {
        assert!(genre_depth("Drone Techno") > genre_depth("Deep Techno"));
        assert!(genre_depth("Deep Techno") > genre_depth("Techno"));
        assert!(genre_depth("Techno") > genre_depth("Hard Techno"));
        assert!(genre_depth("Ambient Techno") > genre_depth("Dub Techno"));
    }

    #[test]
    fn depth_ordering_bass() {
        assert!(genre_depth("Broken Beat") > genre_depth("Drum & Bass"));
        assert!(genre_depth("Dubstep") > genre_depth("Drum & Bass"));
    }

    #[test]
    fn depth_ordering_downtempo() {
        assert!(genre_depth("Ambient") > genre_depth("Downtempo"));
        assert!(genre_depth("Downtempo") > genre_depth("IDM"));
    }

    #[test]
    fn known_bpm_ranges() {
        let deep_techno = genre_bpm_range("Deep Techno").unwrap();
        assert_eq!(deep_techno.typical_min, 120.0);
        assert_eq!(deep_techno.typical_max, 132.0);

        let dnb = genre_bpm_range("Drum & Bass").unwrap();
        assert_eq!(dnb.typical_min, 168.0);
        assert_eq!(dnb.typical_max, 180.0);

        // Genres without diagnostic BPM ranges return None
        assert!(genre_bpm_range("IDM").is_none());
        assert!(genre_bpm_range("Experimental").is_none());
        assert!(genre_bpm_range("Jazz").is_none());
        assert!(genre_bpm_range("Polka").is_none());
    }
}
