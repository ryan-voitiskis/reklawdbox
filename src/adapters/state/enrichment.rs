use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;

use rusqlite::{Connection, params};

/// (provider, query_artist, query_title, query_album)
pub type EnrichmentKey = (String, String, String, String);

pub struct EnrichmentCacheEntry {
    pub provider: String,
    pub query_artist: String,
    pub query_title: String,
    pub query_album: String,
    pub match_quality: Option<String>,
    pub response_json: Option<String>,
    pub created_at: String,
}

pub fn get_enrichment(
    conn: &Connection,
    provider: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
    include_errors: bool,
) -> Result<Option<EnrichmentCacheEntry>, rusqlite::Error> {
    let album = album.unwrap_or("");
    let sql = if include_errors {
        "SELECT provider, query_artist, query_title, query_album, match_quality, response_json, created_at
         FROM enrichment_cache
         WHERE provider = ?1
           AND query_artist = ?2
           AND query_title = ?3
           AND query_album = ?4"
    } else {
        "SELECT provider, query_artist, query_title, query_album, match_quality, response_json, created_at
         FROM enrichment_cache
         WHERE provider = ?1
           AND query_artist = ?2
           AND query_title = ?3
           AND query_album = ?4
           AND COALESCE(match_quality, '') != 'error'"
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let mut rows = stmt.query_map(params![provider, artist, title, album], |row| {
        Ok(EnrichmentCacheEntry {
            provider: row.get(0)?,
            query_artist: row.get(1)?,
            query_title: row.get(2)?,
            query_album: row.get(3)?,
            match_quality: row.get(4)?,
            response_json: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    rows.next().transpose()
}

/// Shared helper for batch enrichment queries that differ only in WHERE suffix.
/// Over-fetches by artist (all titles for matched artists), so the caller
/// filters via `HashSet::contains`. This collapses album identity and must not
/// be used for classification readiness.
#[cfg(test)]
fn batch_enrichment_query(
    conn: &Connection,
    provider: &str,
    artists: &[&str],
    extra_where: &str,
) -> Result<HashSet<(String, String)>, rusqlite::Error> {
    if artists.is_empty() {
        return Ok(HashSet::new());
    }
    const MAX_IN_VARS: usize = 899;
    let mut result = HashSet::new();
    for chunk in artists.chunks(MAX_IN_VARS) {
        let placeholders: Vec<String> = (2..=chunk.len() + 1).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT DISTINCT query_artist, query_title FROM enrichment_cache \
             WHERE provider = ?1 AND query_artist IN ({}) \
               AND COALESCE(match_quality, '') != 'error'{extra_where}",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind_values: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() + 1);
        bind_values.push(&provider);
        for artist in chunk {
            bind_values.push(artist);
        }
        let rows = stmt.query_map(bind_values.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            result.insert(row?);
        }
    }
    Ok(result)
}

#[cfg(test)]
pub fn batch_enrichment_existence(
    conn: &Connection,
    provider: &str,
    artists: &[&str],
) -> Result<HashSet<(String, String)>, rusqlite::Error> {
    batch_enrichment_query(conn, provider, artists, "")
}

/// Batch existence check filtered to entries that have actual results
/// (match_quality = "exact" or "fuzzy", i.e. response_json is non-null).
/// Used by cache_coverage to distinguish "searched" from "has_result".
#[cfg(test)]
pub fn batch_enrichment_with_results(
    conn: &Connection,
    provider: &str,
    artists: &[&str],
) -> Result<HashSet<(String, String)>, rusqlite::Error> {
    batch_enrichment_query(
        conn,
        provider,
        artists,
        " AND match_quality IN ('exact', 'fuzzy')",
    )
}

/// Batch check for enrichment entries that have label data.
/// Returns (artist, title) pairs where the cached response has a non-empty label field.
#[cfg(test)]
pub fn batch_enrichment_with_label(
    conn: &Connection,
    provider: &str,
    artists: &[&str],
) -> Result<HashSet<(String, String)>, rusqlite::Error> {
    batch_enrichment_query(
        conn,
        provider,
        artists,
        " AND match_quality IN ('exact', 'fuzzy') \
         AND json_extract(response_json, '$.label') IS NOT NULL \
         AND json_extract(response_json, '$.label') != ''",
    )
}

/// Batch-load full enrichment cache entries for a set of exact primary-key tuples.
///
/// Keys are `(provider, query_artist, query_title, query_album)`. Entries with
/// `match_quality = 'error'` are excluded, matching `get_enrichment` semantics.
pub fn batch_get_enrichment(
    conn: &Connection,
    keys: &[(&str, &str, &str, &str)],
) -> Result<HashMap<EnrichmentKey, EnrichmentCacheEntry>, rusqlite::Error> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    // 4 bind vars per key; 200 × 4 = 800, well under SQLite's 999 limit.
    const CHUNK_KEYS: usize = 200;
    let mut result = HashMap::with_capacity(keys.len());
    for chunk in keys.chunks(CHUNK_KEYS) {
        use std::fmt::Write as _;
        let mut sql = String::from(
            "SELECT provider, query_artist, query_title, query_album, \
                    match_quality, response_json, created_at \
             FROM enrichment_cache \
             WHERE (provider, query_artist, query_title, query_album) IN (VALUES ",
        );
        for (i, _) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            let b = i * 4 + 1;
            write!(sql, "(?{},?{},?{},?{})", b, b + 1, b + 2, b + 3).unwrap();
        }
        sql.push_str(") AND COALESCE(match_quality, '') != 'error'");

        let mut stmt = conn.prepare(&sql)?;
        let mut bind_values: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() * 4);
        for key in chunk {
            bind_values.push(&key.0);
            bind_values.push(&key.1);
            bind_values.push(&key.2);
            bind_values.push(&key.3);
        }
        let rows = stmt.query_map(bind_values.as_slice(), |row| {
            Ok(EnrichmentCacheEntry {
                provider: row.get(0)?,
                query_artist: row.get(1)?,
                query_title: row.get(2)?,
                query_album: row.get(3)?,
                match_quality: row.get(4)?,
                response_json: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        for row in rows {
            let entry = row?;
            let key = (
                entry.provider.clone(),
                entry.query_artist.clone(),
                entry.query_title.clone(),
                entry.query_album.clone(),
            );
            result.insert(key, entry);
        }
    }
    Ok(result)
}

pub fn set_enrichment(
    conn: &Connection,
    provider: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
    match_quality: Option<&str>,
    response_json: Option<&str>,
) -> Result<(), rusqlite::Error> {
    let album = album.unwrap_or("");
    conn.execute(
        "INSERT INTO enrichment_cache (
             provider,
             query_artist,
             query_title,
             query_album,
             match_quality,
             response_json
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(provider, query_artist, query_title, query_album)
         DO UPDATE SET match_quality = ?5, response_json = ?6, created_at = datetime('now')",
        params![provider, artist, title, album, match_quality, response_json],
    )?;
    Ok(())
}
