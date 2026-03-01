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
    ("afrobeat", "Afro House"),
    ("bass", "UK Bass"),
    ("breaks / breakbeat / uk bass", "Breakbeat"),
    ("chill dnb", "Drum & Bass"),
    ("dance / electro pop", "Synth-pop"),
    ("dance-pop", "Synth-pop"),
    ("dnb", "Drum & Bass"),
    ("drone", "Ambient"),
    ("electronic", "Experimental"),
    ("electronica", "Techno"),
    ("gabber", "Hard Techno"),
    ("glitch", "IDM"),
    ("hard dance", "Hard Techno"),
    ("hard trance", "Trance"),
    ("hip-hop", "Hip Hop"),
    ("indie dance", "House"),
    ("italodance", "Disco"),
    ("loop (hip-hop)", "Hip Hop"),
    ("loop (trance)", "Trance"),
    ("mainstage", "Trance"),
    ("melodic house & techno", "Deep Techno"),
    ("minimal / deep tech", "Minimal"),
    ("r & b", "R&B"),
    ("soundtrack", "Ambient"),
    ("techno (peak time / driving)", "Techno"),
    ("techno (raw / deep / hypnotic)", "Deep Techno"),
    ("trance (main floor)", "Trance"),
    ("trance (raw / deep / hypnotic)", "Trance"),
    ("uk garage", "Garage"),
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
        let key = alias.to_ascii_lowercase();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenreFamily {
    House,
    Techno,
    Bass,
    Downtempo,
    Other,
}

/// Map a canonical genre name to its family. Input should be canonical
/// (via `canonical_genre_name` or `canonical_genre_from_alias`); non-canonical names
/// fall through to `Other`.
pub fn genre_family(canonical: &str) -> GenreFamily {
    match canonical {
        "House" | "Deep House" | "Tech House" | "Afro House" | "Gospel House"
        | "Progressive House" | "Garage" | "Speed Garage" | "Disco" => GenreFamily::House,

        "Techno" | "Deep Techno" | "Minimal" | "Dub Techno" | "Ambient Techno" | "Hard Techno"
        | "Drone Techno" | "Acid" | "Electro" => GenreFamily::Techno,

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
    fn taxonomy_not_empty() {
        assert!(!GENRES.is_empty());
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
        assert_eq!(canonical_genre_from_alias("Electronica"), Some("Techno"));
        assert_eq!(canonical_genre_from_alias("Bass"), Some("UK Bass"));
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
            aliases.len() >= 30,
            "expected at least 30 aliases, got {}",
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
        for g in GENRES {
            let _ = genre_family(g); // should not panic
        }
    }
}
