use std::collections::HashMap;
use std::sync::OnceLock;

/// The starter genre taxonomy. Not a closed list — arbitrary genres are accepted.
/// This list serves as a reference for consistency and auto-complete suggestions.
pub const GENRES: &[&str] = &[
    "2-Step Garage",
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
    "EBM",
    "Electro",
    "Experimental",
    "Footwork",
    "Future Garage",
    "Gabber",
    "Garage",
    "Gospel House",
    "Grime",
    "Happy Hardcore",
    "Hard Techno",
    "Hard Trance",
    "Hardcore",
    "Hardstyle",
    "Highlife",
    "Hip Hop",
    "House",
    "IDM",
    "Italo Disco",
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
    "Trip-Hop",
    "UK Funky",
];

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
    ("electronic body music", "EBM"),
    ("garage house", "House"),
    ("glitch", "IDM"),
    ("hip-hop", "Hip Hop"),
    ("industrial hardcore", "Hardcore"),
    ("italo", "Italo Disco"),
    ("italodance", "Italo Disco"),
    ("juke", "Footwork"),
    ("juke/footwork", "Footwork"),
    ("loop (hip-hop)", "Hip Hop"),
    ("loop (trance)", "Trance"),
    ("mainstage", "Trance"),
    ("mainstream hardcore", "Hardcore"),
    ("melodic house & techno", "Deep Techno"),
    ("minimal / deep tech", "Minimal"),
    ("minimal techno", "Minimal"),
    ("post-dubstep", "Future Garage"),
    ("r & b", "R&B"),
    ("tech trance", "Trance"),
    ("techno (peak time / driving)", "Techno"),
    ("techno (raw / deep / hypnotic)", "Deep Techno"),
    ("terror", "Hardcore"),
    ("trance (main floor)", "Trance"),
    ("trance (raw / deep / hypnotic)", "Trance"),
    ("uk / happy hardcore", "Happy Hardcore"),
    ("uk bass", "Future Garage"),
    ("uk garage", "Garage"),
    ("uk hardcore", "Happy Hardcore"),
    ("uptempo", "Hardcore"),
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
    ("dirty workz", "Hardstyle"),
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
    ("hessle audio", "Techno"),
    ("hospital", "Drum & Bass"),
    ("hotflush", "Dubstep"),
    ("hyperdub", "Future Garage"),
    ("ilian tape", "Techno"),
    ("industrial strength", "Gabber"),
    ("klockworks", "Techno"),
    ("kniteforce", "Happy Hardcore"),
    ("kompakt", "Minimal"),
    ("lobster theremin", "House"),
    ("mac ii", "Drum & Bass"),
    ("mala", "Dubstep"),
    ("masters of hardcore", "Hardcore"),
    ("metalheadz", "Drum & Bass"),
    ("mokum", "Gabber"),
    ("mord", "Hard Techno"),
    ("mote-evolver", "Deep Techno"),
    ("mute", "Synth-pop"),
    ("neophyte", "Hardcore"),
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
    ("rotterdam", "Gabber"),
    ("running back", "Disco"),
    ("rushed", "Hard Techno"),
    ("scantraxx", "Hardstyle"),
    ("semantica", "Deep Techno"),
    ("shogun audio", "Drum & Bass"),
    ("soma", "Techno"),
    ("soul clap", "Disco"),
    ("south london hi-fi", "Dub Reggae"),
    ("stroboscopic artefacts", "Techno"),
    ("suara", "Tech House"),
    ("subtle audio", "Drum & Bass"),
    ("swamp 81", "Dubstep"),
    ("tectonic", "Dubstep"),
    ("teklife", "Footwork"),
    ("tempa", "Dubstep"),
    ("thunderdome", "Hardcore"),
    ("timedance", "Techno"),
    ("toolroom", "Tech House"),
    ("tracid traxxx", "Hard Trance"),
    ("traxtorm", "Hardcore"),
    ("tresor", "Techno"),
    ("truesoul", "Techno"),
    ("type", "Ambient"),
    ("upsammy", "Experimental"),
    ("visionquest", "Deep House"),
    ("wax trax", "EBM"),
];

fn build_alias_map(aliases: &[(&str, &'static str)]) -> HashMap<String, &'static str> {
    let mut map = HashMap::with_capacity(aliases.len());
    for &(alias, canonical) in aliases {
        assert_eq!(
            alias,
            alias.trim(),
            "alias '{alias}' has leading/trailing whitespace"
        );
        assert!(alias.is_ascii(), "alias '{alias}' must be ASCII");
        assert_eq!(
            alias,
            alias.to_ascii_lowercase(),
            "alias '{alias}' must be lowercase ASCII"
        );
        let key = alias.to_string();
        let previous = map.insert(key.clone(), canonical);
        assert!(
            previous.is_none(),
            "duplicate alias key '{key}' (case-insensitive)"
        );
    }
    map
}

pub fn genre_alias_map() -> &'static HashMap<String, &'static str> {
    static MAP: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| build_alias_map(ALIASES))
}

static OVERRIDES: OnceLock<HashMap<String, &'static str>> = OnceLock::new();

/// Load user-defined genre overrides from config. Must be called from main()
/// before classification runs. Validates that all targets are canonical genres;
/// invalid entries are logged and skipped. Overrides take priority over compiled aliases.
pub fn init_overrides(raw: HashMap<String, String>) {
    let mut map = HashMap::with_capacity(raw.len());
    for (alias, target) in raw {
        let key = alias.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        if let Some(canonical) = canonical_genre_name(&target) {
            map.insert(key, canonical);
        } else {
            tracing::warn!(
                alias = alias.as_str(),
                target = target.as_str(),
                "Genre override target is not a canonical genre, ignoring"
            );
        }
    }
    OVERRIDES
        .set(map)
        .expect("BUG: init_overrides() must only be called once");
}

/// Returns the canonical genre if the input is a known alias, `None` if already canonical or unknown.
/// Checks user overrides (from config) first, then compiled aliases.
pub fn canonical_genre_from_alias(genre: &str) -> Option<&'static str> {
    let key = genre.trim().to_ascii_lowercase();
    if let Some(overrides) = OVERRIDES.get()
        && let Some(&canonical) = overrides.get(&key) {
            return Some(canonical);
        }
    genre_alias_map().get(&key).copied()
}

/// Extracts the base genre from a parenthetical pattern like "Techno (Peak Time / Driving)"
/// and resolves it through canonical/alias lookup. Returns None if the string has no
/// parenthetical, or if the base doesn't resolve.
pub fn extract_parenthetical_base(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    let paren_idx = trimmed.find('(')?;
    let base = trimmed[..paren_idx].trim();
    if base.is_empty() {
        return None;
    }
    canonical_genre_name(base).or_else(|| canonical_genre_from_alias(base))
}

/// Token-to-genre map for extracting genre signals from non-canonical genre strings.
/// Used as low-weight classification evidence, not for normalization.
/// Multi-word entries are checked first against slash/comma-delimited parts.
/// Single-word entries are checked against individual words.
/// Vague terms ("electronic", "electronica", "music", "dance") are intentionally omitted.
const GENRE_TOKENS: &[(&str, &[&str])] = &[
    // Multi-word (checked against slash/comma-delimited parts before word splitting)
    ("hard dance", &["Hard Techno", "Hardstyle"]),
    ("hip hop", &["Hip Hop"]),
    // Single-word (checked via word splitting)
    ("acid", &["Acid"]),
    ("ambient", &["Ambient"]),
    ("bass", &["Dubstep", "Drum & Bass", "Breakbeat"]),
    ("breakbeat", &["Breakbeat"]),
    ("breaks", &["Breakbeat"]),
    ("chill", &["Downtempo"]),
    ("dancehall", &["Dancehall"]),
    ("disco", &["Disco"]),
    ("downtempo", &["Downtempo"]),
    ("drone", &["Drone Techno", "Ambient"]),
    ("dubstep", &["Dubstep"]),
    ("ebm", &["EBM"]),
    ("electro", &["Electro"]),
    ("experimental", &["Experimental"]),
    ("footwork", &["Footwork"]),
    ("gabber", &["Gabber"]),
    ("garage", &["Garage"]),
    ("grime", &["Grime"]),
    ("hardcore", &["Hardcore"]),
    ("highlife", &["Highlife"]),
    ("house", &["House"]),
    ("idm", &["IDM"]),
    ("industrial", &["Hard Techno", "EBM"]),
    ("italo", &["Italo Disco"]),
    ("jazz", &["Jazz"]),
    ("jungle", &["Jungle"]),
    ("lounge", &["Downtempo"]),
    ("minimal", &["Minimal"]),
    ("pop", &["Pop"]),
    ("psytrance", &["Psytrance"]),
    ("r&b", &["R&B"]),
    ("rap", &["Hip Hop"]),
    ("reggae", &["Reggae"]),
    ("rnb", &["R&B"]),
    ("rock", &["Rock"]),
    ("soul", &["R&B"]),
    ("techno", &["Techno"]),
    ("trance", &["Trance"]),
    ("trip-hop", &["Trip-Hop"]),
];

fn genre_token_map() -> &'static HashMap<&'static str, &'static [&'static str]> {
    static MAP: OnceLock<HashMap<&'static str, &'static [&'static str]>> = OnceLock::new();
    MAP.get_or_init(|| {
        GENRE_TOKENS
            .iter()
            .filter(|(token, _)| !token.contains(' '))
            .map(|&(token, genres)| (token, genres))
            .collect()
    })
}

/// Extract genre candidates from a non-canonical genre string by tokenizing
/// and matching against known genre keywords. Returns canonical genre names.
/// Only fires for non-canonical, non-alias strings (avoids circular evidence).
pub fn extract_genre_tokens(genre_str: &str) -> Vec<&'static str> {
    if genre_str.is_empty()
        || is_known_genre(genre_str)
        || canonical_genre_from_alias(genre_str).is_some()
    {
        return vec![];
    }

    let lower = genre_str.trim().to_ascii_lowercase();
    let mut matched: Vec<&'static str> = Vec::new();

    let mut push_unique = |genres: &[&str]| {
        for &g in genres {
            if let Some(c) = canonical_genre_name(g)
                && !matched.contains(&c) {
                    matched.push(c);
                }
        }
    };

    // Split on / , ( ) to get parts
    let parts: Vec<&str> = lower
        .split(['/', ',', '(', ')'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    // Multi-word tokens: check against each part
    for &(token, genres) in GENRE_TOKENS {
        if !token.contains(' ') {
            continue;
        }
        if parts.iter().any(|part| part.contains(token)) {
            push_unique(genres);
        }
    }

    // Single-word tokens
    let map = genre_token_map();
    for part in &parts {
        for word in part.split_whitespace() {
            if let Some(genres) = map.get(word) {
                push_unique(genres);
            }
        }
    }

    matched
}

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

/// Tries exact match first, then strips common suffixes like " records".
/// ASCII case-folding only.
pub fn label_genre(label: &str) -> Option<&'static str> {
    let normalized = label.trim().to_ascii_lowercase();
    if let Some(&genre) = label_genre_map().get(&normalized) {
        return Some(genre);
    }
    for suffix in LABEL_SUFFIXES {
        if let Some(stripped) = normalized.strip_suffix(suffix)
            && let Some(&genre) = label_genre_map().get(stripped)
        {
            return Some(genre);
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
        "2-Step Garage" => Some(BpmRange::new(128.0, 135.0)),
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
        "EBM" => Some(BpmRange::new(110.0, 140.0)),
        "Footwork" => Some(BpmRange::new(155.0, 165.0)),
        "Future Garage" => Some(BpmRange::new(125.0, 138.0)),
        "Gabber" => Some(BpmRange::new(160.0, 190.0)),
        "Garage" => Some(BpmRange::new(130.0, 138.0)),
        "Gospel House" => Some(BpmRange::new(120.0, 128.0)),
        "Grime" => Some(BpmRange::new(138.0, 145.0)),
        "Happy Hardcore" => Some(BpmRange::new(165.0, 180.0)),
        "Hard Techno" => Some(BpmRange::new(145.0, 160.0)),
        "Hard Trance" => Some(BpmRange::new(138.0, 150.0)),
        "Hardcore" => Some(BpmRange::new(160.0, 180.0)),
        "Hardstyle" => Some(BpmRange::new(148.0, 160.0)),
        "House" => Some(BpmRange::new(120.0, 130.0)),
        "Italo Disco" => Some(BpmRange::new(118.0, 135.0)),
        "Jungle" => Some(BpmRange::new(160.0, 175.0)),
        "Minimal" => Some(BpmRange::new(120.0, 132.0)),
        "Progressive House" => Some(BpmRange::new(122.0, 132.0)),
        "Psytrance" => Some(BpmRange::new(138.0, 148.0)),
        "Speed Garage" => Some(BpmRange::new(130.0, 140.0)),
        "Tech House" => Some(BpmRange::new(124.0, 132.0)),
        "Techno" => Some(BpmRange::new(128.0, 140.0)),
        "Trance" => Some(BpmRange::new(136.0, 145.0)),
        "Trip-Hop" => Some(BpmRange::new(80.0, 100.0)),
        "UK Funky" => Some(BpmRange::new(125.0, 135.0)),
        _ => None,
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
        "Disco" | "Speed Garage" | "Italo Disco" => 1,
        "Gospel House" | "Garage" | "Afro House" | "2-Step Garage" | "UK Funky" => 2,
        "House" | "Tech House" => 3,
        "Progressive House" => 4,
        "Deep House" => 5,

        // Hardcore family — 1 = most commercial/euphoric, 5 = most raw/aggressive
        "Hardstyle" => 1,
        "Happy Hardcore" => 2,
        "Hard Trance" => 3,
        "Hardcore" => 4,
        "Gabber" => 5,

        // Techno family
        "Hard Techno" | "Psytrance" => 1,
        "Acid" | "EBM" | "Electro" | "Trance" => 2,
        "Techno" => 3,
        "Minimal" => 4,
        "Deep Techno" => 5,
        "Dub Techno" => 6,
        "Ambient Techno" => 7,
        "Drone Techno" => 8,

        // Bass family
        "Grime" | "Bassline" => 1,
        "Drum & Bass" | "Footwork" => 2,
        "Jungle" | "Breakbeat" => 3,
        "Dubstep" => 4,
        "Future Garage" | "Broken Beat" => 5,

        // Downtempo family
        "IDM" | "Experimental" => 2,
        "Downtempo" | "Trip-Hop" => 3,
        "Dub" | "Dub Reggae" => 4,
        "Ambient" => 5,

        // Other / unknown — no depth comparison possible
        _ => 0,
    }
}

/// Non-canonical names fall through to `Other`.
pub fn genre_family(canonical: &str) -> GenreFamily {
    match canonical {
        "House" | "Deep House" | "Tech House" | "Afro House" | "Gospel House"
        | "Progressive House" | "Garage" | "Speed Garage" | "Disco" | "Italo Disco"
        | "2-Step Garage" | "UK Funky" => GenreFamily::House,

        "Techno" | "Deep Techno" | "Minimal" | "Dub Techno" | "Ambient Techno" | "Hard Techno"
        | "Drone Techno" | "Acid" | "EBM" | "Electro" | "Trance" | "Psytrance" => {
            GenreFamily::Techno
        }

        "Hardcore" | "Gabber" | "Hardstyle" | "Happy Hardcore" | "Hard Trance" => {
            GenreFamily::Hardcore
        }

        "Drum & Bass" | "Jungle" | "Dubstep" | "Breakbeat" | "Footwork" | "Future Garage"
        | "Grime" | "Bassline" | "Broken Beat" => GenreFamily::Bass,

        "Ambient" | "Downtempo" | "Trip-Hop" | "Dub" | "Dub Reggae" | "IDM"
        | "Experimental" => GenreFamily::Downtempo,

        _ => GenreFamily::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn all_genre_token_targets_are_canonical() {
        for &(token, genres) in GENRE_TOKENS {
            for &g in genres {
                assert!(
                    canonical_genre_name(g).is_some(),
                    "GENRE_TOKENS entry '{token}' references non-canonical genre '{g}'"
                );
            }
        }
    }

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
        assert!(is_known_genre("uk funky"));
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
        assert_eq!(canonical_genre_from_alias("Glitch"), Some("IDM"));
        assert_eq!(canonical_genre_from_alias("Terror"), Some("Hardcore"));
        assert_eq!(canonical_genre_from_alias("Uptempo"), Some("Hardcore"));
        assert_eq!(
            canonical_genre_from_alias("UK / Happy Hardcore"),
            Some("Happy Hardcore")
        );
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
            aliases.len() >= 26,
            "expected at least 26 aliases, got {}",
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
            assert!(alias.is_ascii(), "alias '{alias}' must be ASCII");
            assert_eq!(
                alias,
                alias.to_ascii_lowercase(),
                "alias '{alias}' must be lowercase ASCII"
            );
            let inserted = seen.insert(alias.to_ascii_lowercase());
            assert!(
                inserted,
                "duplicate alias key '{alias}' (case-insensitive)"
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
                "alias '{alias}' maps to '{canonical}' which is not in taxonomy"
            );
        }
    }

    #[test]
    fn no_alias_shadows_canonical() {
        for &(alias, target) in ALIASES {
            assert!(
                !is_known_genre(alias),
                "alias '{alias}' (-> '{target}') shadows a canonical genre — remove it"
            );
        }
    }

    #[test]
    fn all_taxonomy_genres_have_family() {
        assert_eq!(genre_family("House"), GenreFamily::House);
        assert_eq!(genre_family("Deep House"), GenreFamily::House);
        assert_eq!(genre_family("Techno"), GenreFamily::Techno);
        assert_eq!(genre_family("Hard Techno"), GenreFamily::Techno);
        assert_eq!(genre_family("Trance"), GenreFamily::Techno);
        assert_eq!(genre_family("Psytrance"), GenreFamily::Techno);
        assert_eq!(genre_family("Drum & Bass"), GenreFamily::Bass);
        assert_eq!(genre_family("Dubstep"), GenreFamily::Bass);
        assert_eq!(genre_family("Hardcore"), GenreFamily::Hardcore);
        assert_eq!(genre_family("Gabber"), GenreFamily::Hardcore);
        assert_eq!(genre_family("Hardstyle"), GenreFamily::Hardcore);
        assert_eq!(genre_family("Happy Hardcore"), GenreFamily::Hardcore);
        assert_eq!(genre_family("Hard Trance"), GenreFamily::Hardcore);
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
    fn token_extraction_specific_match() {
        let tokens = extract_genre_tokens("Electronic Techno");
        assert_eq!(tokens, vec!["Techno"]);
    }

    #[test]
    fn token_extraction_compound_string() {
        let mut tokens = extract_genre_tokens("Electro Chill Out/Trip-Hop/Lounge");
        tokens.sort();
        assert!(tokens.contains(&"Downtempo"));
        assert!(tokens.contains(&"Electro"));
        assert!(tokens.contains(&"Trip-Hop"));
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn token_extraction_multi_word() {
        let tokens = extract_genre_tokens("Dance/Rap/Hip Hop");
        assert_eq!(tokens, vec!["Hip Hop"]);
    }

    #[test]
    fn token_extraction_vague_returns_empty() {
        assert!(extract_genre_tokens("Electronica").is_empty());
        assert!(extract_genre_tokens("Electronic").is_empty());
        assert!(extract_genre_tokens("Anti-music").is_empty());
        assert!(extract_genre_tokens("").is_empty());
    }

    #[test]
    fn token_extraction_skips_canonical() {
        assert!(extract_genre_tokens("Techno").is_empty());
        assert!(extract_genre_tokens("Deep House").is_empty());
    }

    #[test]
    fn token_extraction_skips_aliases() {
        assert!(extract_genre_tokens("Hip-Hop").is_empty());
        assert!(extract_genre_tokens("DnB").is_empty());
    }

    #[test]
    fn parenthetical_extracts_base_genre() {
        assert_eq!(
            extract_parenthetical_base("Techno (Peak Time / Driving)"),
            Some("Techno")
        );
        assert_eq!(
            extract_parenthetical_base("House (Progressive)"),
            Some("House")
        );
        // Base resolves through alias
        assert_eq!(
            extract_parenthetical_base("DnB (Liquid)"),
            Some("Drum & Bass")
        );
    }

    #[test]
    fn parenthetical_returns_none_for_non_matching() {
        assert_eq!(extract_parenthetical_base("Techno"), None);
        assert_eq!(extract_parenthetical_base(""), None);
        assert_eq!(extract_parenthetical_base("(nothing before paren)"), None);
        assert_eq!(extract_parenthetical_base("Polka (Fast)"), None);
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
            assert!(label.is_ascii(), "label '{label}' must be ASCII");
            assert_eq!(
                label,
                label.to_ascii_lowercase(),
                "label '{label}' must be lowercase ASCII"
            );
        }
    }

    #[test]
    fn all_label_genre_targets_are_canonical() {
        for &(label, canonical) in LABEL_GENRES {
            assert!(
                is_known_genre(canonical),
                "label '{label}' maps to '{canonical}' which is not in taxonomy"
            );
        }
    }

    #[test]
    fn no_label_shadows_alias() {
        let alias_map = genre_alias_map();
        for &(label, _) in LABEL_GENRES {
            assert!(
                !alias_map.contains_key(label),
                "label '{label}' shadows an alias key"
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
        assert_eq!(label_genre("Hyperdub"), Some("Future Garage"));
        assert_eq!(label_genre("TRESOR"), Some("Techno"));
    }

    #[test]
    fn suffix_stripped_labels_are_consistent() {
        let map = label_genre_map();
        for &(label, genre) in LABEL_GENRES {
            for suffix in LABEL_SUFFIXES {
                if let Some(prefix) = label.strip_suffix(suffix)
                    && let Some(&prefix_genre) = map.get(prefix)
                {
                    assert_eq!(
                        genre, prefix_genre,
                        "label '{label}' maps to '{genre}' but prefix '{prefix}' maps to '{prefix_genre}'"
                    );
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
                    "genre '{g}' (family {family:?}) has depth 0 — add it to genre_depth()",
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
    fn depth_ordering_hardcore() {
        assert!(genre_depth("Gabber") > genre_depth("Hardcore"));
        assert!(genre_depth("Hardcore") > genre_depth("Hard Trance"));
        assert!(genre_depth("Hard Trance") > genre_depth("Happy Hardcore"));
        assert!(genre_depth("Happy Hardcore") > genre_depth("Hardstyle"));
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

        let hardcore = genre_bpm_range("Hardcore").unwrap();
        assert_eq!(hardcore.typical_min, 160.0);
        assert_eq!(hardcore.typical_max, 180.0);

        let gabber = genre_bpm_range("Gabber").unwrap();
        assert_eq!(gabber.typical_min, 160.0);
        assert_eq!(gabber.typical_max, 190.0);

        let hardstyle = genre_bpm_range("Hardstyle").unwrap();
        assert_eq!(hardstyle.typical_min, 148.0);
        assert_eq!(hardstyle.typical_max, 160.0);

        let happy_hc = genre_bpm_range("Happy Hardcore").unwrap();
        assert_eq!(happy_hc.typical_min, 165.0);
        assert_eq!(happy_hc.typical_max, 180.0);

        let hard_trance = genre_bpm_range("Hard Trance").unwrap();
        assert_eq!(hard_trance.typical_min, 138.0);
        assert_eq!(hard_trance.typical_max, 150.0);

        assert!(genre_bpm_range("IDM").is_none());
        assert!(genre_bpm_range("Experimental").is_none());
        assert!(genre_bpm_range("Jazz").is_none());
        assert!(genre_bpm_range("Polka").is_none());
    }
}
