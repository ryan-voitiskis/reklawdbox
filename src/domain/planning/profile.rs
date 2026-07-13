//! Track-profile taxonomy policy.

use crate::domain::classification::taxonomy::{GenreFamily, genre_family, resolve_genre};

pub(crate) fn canonicalize_genre(raw_genre: &str) -> Option<String> {
    let trimmed = raw_genre.trim();
    if trimmed.is_empty() {
        return None;
    }
    resolve_genre(trimmed).map(str::to_string)
}

pub(crate) fn genre_family_for(canonical_genre: &str) -> GenreFamily {
    genre_family(canonical_genre)
}
