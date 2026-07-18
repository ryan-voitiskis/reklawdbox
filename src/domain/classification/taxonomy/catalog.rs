//! Canonical genre catalog and resolution entry points.

use super::aliases::canonical_genre_from_alias;

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
    "Drum & Bass",
    "Dub",
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
    "Italodance",
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

/// Resolve a genre string to its canonical form by trying exact match first,
/// then alias lookup. Returns None if the genre is not recognized.
pub fn resolve_genre(genre: &str) -> Option<&'static str> {
    canonical_genre_name(genre).or_else(|| canonical_genre_from_alias(genre))
}

pub fn is_known_genre(genre: &str) -> bool {
    canonical_genre_name(genre).is_some()
}
