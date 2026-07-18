//! Aliases, overrides, token extraction, and label mappings.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::catalog::{canonical_genre_name, is_known_genre, resolve_genre};

/// Alias entries mapping non-canonical genre strings to canonical genres.
/// Keys must be lowercase ASCII. Sorted alphabetically by key.
pub const ALIASES: &[(&str, &str)] = &[
    ("chill dnb", "Drum & Bass"),
    ("dnb", "Drum & Bass"),
    ("electronic body music", "EBM"),
    ("garage house", "House"),
    ("hip-hop", "Hip Hop"),
    ("industrial hardcore", "Hardcore"),
    ("italo", "Italo Disco"),
    ("juke", "Footwork"),
    ("juke/footwork", "Footwork"),
    ("loop (hip-hop)", "Hip Hop"),
    ("loop (trance)", "Trance"),
    ("mainstream hardcore", "Hardcore"),
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

pub(super) fn build_alias_map(aliases: &[(&str, &'static str)]) -> HashMap<String, &'static str> {
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
        && let Some(&canonical) = overrides.get(&key)
    {
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
    resolve_genre(base)
}

/// Token-to-genre map for extracting genre signals from non-canonical genre strings.
/// Used as low-weight classification evidence, not for normalization.
/// Multi-word entries are checked first against slash/comma-delimited parts.
/// Single-word entries are checked against individual words.
/// Vague terms ("electronic", "electronica", "music", "dance") are intentionally omitted.
pub(super) const GENRE_TOKENS: &[(&str, &[&str])] = &[
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
    ("drone", &["Ambient"]),
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
                && !matched.contains(&c)
            {
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

pub(super) const LABEL_SUFFIXES: &[&str] = &[
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

/// Map an external genre string through the canonical taxonomy without
/// importing planning or transport code.
pub(crate) fn map_genre_through_taxonomy(style: &str) -> (Option<String>, &'static str) {
    if let Some(canonical) = canonical_genre_name(style) {
        (Some(canonical.to_string()), "exact")
    } else if let Some(canonical) = canonical_genre_from_alias(style) {
        (Some(canonical.to_string()), "alias")
    } else if let Some(canonical) = extract_parenthetical_base(style) {
        (Some(canonical.to_string()), "parenthetical")
    } else {
        (None, "unknown")
    }
}
