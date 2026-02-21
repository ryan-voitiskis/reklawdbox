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

pub fn get_taxonomy() -> Vec<String> {
    GENRES.iter().map(|s| s.to_string()).collect()
}

/// Returns the canonical casing of a genre if it's in the taxonomy.
pub fn canonical_casing(genre: &str) -> Option<&'static str> {
    GENRES
        .iter()
        .find(|g| g.eq_ignore_ascii_case(genre))
        .copied()
}

pub fn is_known_genre(genre: &str) -> bool {
    canonical_casing(genre).is_some()
}

/// Alias entries mapping non-canonical genre strings to canonical genres.
/// Keys must be lowercase. Sorted alphabetically by key.
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

/// Static alias map built once via OnceLock. Maps lowercase alias → canonical genre.
fn alias_map() -> &'static HashMap<String, &'static str> {
    static MAP: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::with_capacity(ALIASES.len());
        for &(alias, canonical) in ALIASES {
            map.insert(alias.to_lowercase(), canonical);
        }
        map
    })
}

/// Returns the canonical genre if the input is a known alias, `None` if already canonical or unknown.
pub fn normalize_genre(genre: &str) -> Option<&'static str> {
    alias_map().get(&genre.to_lowercase()).copied()
}

pub fn get_alias_map() -> Vec<(String, String)> {
    let map = alias_map();
    let mut pairs: Vec<(String, String)> = map
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string()))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taxonomy_not_empty() {
        assert!(!get_taxonomy().is_empty());
    }

    #[test]
    fn taxonomy_sorted() {
        let genres = get_taxonomy();
        let mut sorted = genres.clone();
        sorted.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        assert_eq!(genres, sorted, "GENRES array must be sorted alphabetically");
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
    fn normalize_known_aliases() {
        assert_eq!(normalize_genre("Hip-Hop"), Some("Hip Hop"));
        assert_eq!(normalize_genre("DnB"), Some("Drum & Bass"));
        assert_eq!(normalize_genre("Electronica"), Some("Techno"));
        assert_eq!(normalize_genre("Bass"), Some("UK Bass"));
        assert_eq!(normalize_genre("Gabber"), Some("Hard Techno"));
        assert_eq!(normalize_genre("Glitch"), Some("IDM"));
        assert_eq!(
            normalize_genre("140 / Deep Dubstep / Grime"),
            Some("Dubstep")
        );
    }

    #[test]
    fn normalize_case_insensitive() {
        assert_eq!(normalize_genre("hip-hop"), Some("Hip Hop"));
        assert_eq!(normalize_genre("HIP-HOP"), Some("Hip Hop"));
        assert_eq!(normalize_genre("Hip-Hop"), Some("Hip Hop"));
        assert_eq!(normalize_genre("dnb"), Some("Drum & Bass"));
        assert_eq!(normalize_genre("DNB"), Some("Drum & Bass"));
    }

    #[test]
    fn normalize_canonical_returns_none() {
        assert_eq!(normalize_genre("Techno"), None);
        assert_eq!(normalize_genre("Deep House"), None);
        assert_eq!(normalize_genre("Drum & Bass"), None);
        assert_eq!(normalize_genre("Hip Hop"), None);
        assert_eq!(normalize_genre("Rock"), None);
        assert_eq!(normalize_genre("Pop"), None);
    }

    #[test]
    fn normalize_unknown_returns_none() {
        assert_eq!(normalize_genre("Polka"), None);
        assert_eq!(normalize_genre("Anti-music"), None);
        assert_eq!(normalize_genre("Zydeco"), None);
    }

    #[test]
    fn alias_map_not_empty() {
        let aliases = get_alias_map();
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
}
