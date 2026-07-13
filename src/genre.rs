//! Compatibility façade for the classification taxonomy package.

#![allow(unused_imports)]

pub(crate) use crate::domain::classification::taxonomy::{
    ALIASES, BpmRange, GENRES, GenreFamily, LABEL_GENRES, canonical_genre_from_alias,
    canonical_genre_name, extract_genre_tokens, extract_parenthetical_base, genre_alias_map,
    genre_bpm_range, genre_depth, genre_family, init_overrides, is_known_genre, label_genre,
    label_genre_map, map_genre_through_taxonomy, resolve_genre,
};
