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
    "Drum & Bass",
    "Dub",
    "Dub Techno",
    "Dubstep",
    "Electro",
    "Experimental",
    "Garage",
    "Grime",
    "Hard Techno",
    "Hip Hop",
    "House",
    "IDM",
    "Jungle",
    "Minimal",
    "Psytrance",
    "R&B",
    "Reggae",
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

pub fn is_known_genre(genre: &str) -> bool {
    GENRES.iter().any(|g| g.eq_ignore_ascii_case(genre))
}

/// Static alias map built once via OnceLock. Maps lowercase alias → canonical genre.
fn alias_map() -> &'static HashMap<String, &'static str> {
    static MAP: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let entries: &[(&str, &str)] = &[
            ("hip-hop", "Hip Hop"),
            ("loop (hip-hop)", "Hip Hop"),
            ("electronica", "Techno"),
            ("techno (peak time / driving)", "Hard Techno"),
            ("dnb", "Drum & Bass"),
            ("bass", "UK Bass"),
            ("r & b", "R&B"),
            ("techno (raw / deep / hypnotic)", "Deep Techno"),
            ("drone techno", "Deep Techno"),
            ("trance (main floor)", "Trance"),
            ("uk garage", "Garage"),
            ("gospel house", "House"),
            ("highlife", "Afro House"),
            ("progressive house", "House"),
            ("minimal / deep tech", "Minimal"),
            ("melodic house & techno", "Deep Techno"),
            ("italodance", "Disco"),
            ("dance-pop", "Synth-pop"),
            ("breaks / breakbeat / uk bass", "Breakbeat"),
            ("trance (raw / deep / hypnotic)", "Trance"),
            ("soundtrack", "Ambient"),
            ("rock", "Experimental"),
            ("mainstage", "Trance"),
            ("loop (trance)", "Trance"),
            ("indie dance", "House"),
            ("hard trance", "Trance"),
            ("hard dance", "Hard Techno"),
            ("glitch", "IDM"),
            ("gabber", "Hard Techno"),
            ("electronic", "Experimental"),
            ("dub reggae", "Dub"),
            ("drone", "Ambient"),
            ("dance / electro pop", "Synth-pop"),
            ("chill dnb", "Drum & Bass"),
            ("ballad", "Downtempo"),
            ("afrobeat", "Afro House"),
            ("140 / deep dubstep / grime", "Dubstep"),
        ];
        let mut map = HashMap::with_capacity(entries.len());
        for &(alias, canonical) in entries {
            map.insert(alias.to_lowercase(), canonical);
        }
        map
    })
}

/// Returns the canonical genre if the input is a known alias, `None` if already canonical or unknown.
pub fn normalize_genre(genre: &str) -> Option<&'static str> {
    alias_map().get(&genre.to_lowercase()).copied()
}

/// Returns the full alias map as (alias, canonical) pairs for display.
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
    fn taxonomy_has_35_genres() {
        assert_eq!(GENRES.len(), 35);
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
        assert_eq!(normalize_genre("Drone Techno"), Some("Deep Techno"));
        assert_eq!(normalize_genre("Gospel House"), Some("House"));
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
    }

    #[test]
    fn normalize_unknown_returns_none() {
        assert_eq!(normalize_genre("Polka"), None);
        assert_eq!(normalize_genre("Anti-music"), None);
        assert_eq!(normalize_genre("Pop"), None);
    }

    #[test]
    fn alias_map_not_empty() {
        let aliases = get_alias_map();
        assert!(
            aliases.len() >= 37,
            "expected at least 37 aliases, got {}",
            aliases.len()
        );
    }

    #[test]
    fn alias_map_sorted() {
        let aliases = get_alias_map();
        for w in aliases.windows(2) {
            assert!(
                w[0].0 <= w[1].0,
                "alias map not sorted: {:?} > {:?}",
                w[0].0,
                w[1].0
            );
        }
    }

    #[test]
    fn all_alias_targets_are_canonical() {
        let aliases = get_alias_map();
        for (alias, canonical) in &aliases {
            assert!(
                is_known_genre(canonical),
                "alias '{}' maps to '{}' which is not in taxonomy",
                alias,
                canonical
            );
        }
    }
}
