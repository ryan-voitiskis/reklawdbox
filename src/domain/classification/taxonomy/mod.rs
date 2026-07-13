//! Canonical genre vocabulary and transport-neutral taxonomy resolution.
//!
//! Catalog membership, aliases, label mappings, and genre metadata are owned
//! here so classification and planning callers share one dependency-free
//! taxonomy implementation.

mod aliases;
mod catalog;
mod metadata;

pub(crate) use aliases::{
    ALIASES, LABEL_GENRES, canonical_genre_from_alias, extract_genre_tokens,
    extract_parenthetical_base, genre_alias_map, init_overrides, label_genre, label_genre_map,
    map_genre_through_taxonomy,
};
pub(crate) use catalog::{GENRES, canonical_genre_name, is_known_genre, resolve_genre};
pub(crate) use metadata::{BpmRange, GenreFamily, genre_bpm_range, genre_depth, genre_family};

#[cfg(test)]
use aliases::{GENRE_TOKENS, LABEL_SUFFIXES, build_alias_map};
#[cfg(test)]
mod tests;
