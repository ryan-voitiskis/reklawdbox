use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};

pub struct CachedAudioAnalysis {
    pub file_path: String,
    #[allow(dead_code)]
    pub analyzer: String,
    #[allow(dead_code)]
    pub file_size: i64,
    #[allow(dead_code)]
    pub file_mtime: i64,
    pub analysis_version: String,
    pub input_fingerprint: String,
    pub features_json: String,
    #[allow(dead_code)]
    pub created_at: String,
}

#[derive(Debug)]
pub struct TimbralSourceRow {
    pub file_path: String,
    pub file_size: i64,
    pub file_mtime: i64,
    pub input_fingerprint: String,
    pub features_json: String,
}

pub fn load_timbral_source_rows(
    conn: &Connection,
    analyzer: &str,
    analysis_version: &str,
) -> Result<Vec<TimbralSourceRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT file_path, file_size, file_mtime, input_fingerprint, features_json
         FROM audio_analysis_cache
         WHERE analyzer = ?1 AND analysis_version = ?2
         ORDER BY file_path",
    )?;
    stmt.query_map(params![analyzer, analysis_version], |row| {
        Ok(TimbralSourceRow {
            file_path: row.get(0)?,
            file_size: row.get(1)?,
            file_mtime: row.get(2)?,
            input_fingerprint: row.get(3)?,
            features_json: row.get(4)?,
        })
    })?
    .collect()
}

pub fn is_audio_analysis_fresh(
    cached: Option<&CachedAudioAnalysis>,
    analysis_version: &str,
    file_size: i64,
    file_mtime: i64,
    input_fingerprint: &str,
) -> bool {
    matches!(
        cached,
        Some(entry)
            if entry.analysis_version == analysis_version
                && entry.file_size == file_size
                && entry.file_mtime == file_mtime
                && entry.input_fingerprint == input_fingerprint
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioAnalysisIdentity<'a> {
    pub file_path: &'a str,
    pub file_size: i64,
    pub file_mtime: i64,
    pub input_fingerprint: &'a str,
}

pub fn get_audio_analysis(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
) -> Result<Option<CachedAudioAnalysis>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT file_path, analyzer, file_size, file_mtime, analysis_version, input_fingerprint, features_json, created_at
         FROM audio_analysis_cache
         WHERE file_path = ?1 AND analyzer = ?2",
    )?;
    let mut rows = stmt.query_map(params![file_path, analyzer], |row| {
        Ok(CachedAudioAnalysis {
            file_path: row.get(0)?,
            analyzer: row.get(1)?,
            file_size: row.get(2)?,
            file_mtime: row.get(3)?,
            analysis_version: row.get(4)?,
            input_fingerprint: row.get(5)?,
            features_json: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.next().transpose()
}

/// Batch-load full audio analysis cache entries for a set of file paths,
/// filtered to a single analyzer and schema version.
pub fn batch_get_audio_analysis(
    conn: &Connection,
    file_paths: &[&str],
    analyzer: &str,
    analysis_version: &str,
) -> Result<HashMap<String, CachedAudioAnalysis>, rusqlite::Error> {
    if file_paths.is_empty() {
        return Ok(HashMap::new());
    }
    // 1 bind var per path + 2 for analyzer and version; 896 + 2 = 898, under 999.
    const MAX_PATHS: usize = 896;
    let mut result = HashMap::with_capacity(file_paths.len());
    for chunk in file_paths.chunks(MAX_PATHS) {
        use std::fmt::Write as _;
        let mut placeholders = String::new();
        for i in 0..chunk.len() {
            if i > 0 {
                placeholders.push(',');
            }
            write!(placeholders, "?{}", i + 1).unwrap();
        }
        let analyzer_pos = chunk.len() + 1;
        let version_pos = chunk.len() + 2;
        let sql = format!(
            "SELECT file_path, analyzer, file_size, file_mtime, \
                    analysis_version, input_fingerprint, features_json, created_at \
             FROM audio_analysis_cache \
             WHERE file_path IN ({placeholders}) \
               AND analyzer = ?{analyzer_pos} \
               AND analysis_version = ?{version_pos}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind_values: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() + 2);
        for path in chunk {
            bind_values.push(path);
        }
        bind_values.push(&analyzer);
        bind_values.push(&analysis_version);
        let rows = stmt.query_map(bind_values.as_slice(), |row| {
            Ok(CachedAudioAnalysis {
                file_path: row.get(0)?,
                analyzer: row.get(1)?,
                file_size: row.get(2)?,
                file_mtime: row.get(3)?,
                analysis_version: row.get(4)?,
                input_fingerprint: row.get(5)?,
                features_json: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        for row in rows {
            let entry = row?;
            result.insert(entry.file_path.clone(), entry);
        }
    }
    Ok(result)
}

pub fn batch_get_fresh_audio_analysis(
    conn: &Connection,
    identities: &[AudioAnalysisIdentity<'_>],
    analyzer: &str,
    analysis_version: &str,
) -> Result<HashMap<String, CachedAudioAnalysis>, rusqlite::Error> {
    if identities.is_empty() {
        return Ok(HashMap::new());
    }
    let expected = coalesce_audio_analysis_identities(identities);
    let file_paths: Vec<&str> = expected
        .iter()
        .filter_map(|(&file_path, identity)| identity.is_some().then_some(file_path))
        .collect();
    let mut entries = batch_get_audio_analysis(conn, &file_paths, analyzer, analysis_version)?;
    entries.retain(|file_path, entry| {
        expected
            .get(file_path.as_str())
            .and_then(Option::as_ref)
            .is_some_and(|(file_size, file_mtime, input_fingerprint)| {
                is_audio_analysis_fresh(
                    Some(&*entry),
                    analysis_version,
                    *file_size,
                    *file_mtime,
                    input_fingerprint,
                )
            })
    });
    Ok(entries)
}

pub fn batch_fresh_audio_analysis_existence(
    conn: &Connection,
    identities: &[AudioAnalysisIdentity<'_>],
    analyzer: &str,
    analysis_version: &str,
) -> Result<HashSet<String>, rusqlite::Error> {
    if identities.is_empty() {
        return Ok(HashSet::new());
    }
    let expected = coalesce_audio_analysis_identities(identities);
    let unique_identities: Vec<_> = expected
        .iter()
        .filter_map(|(&file_path, identity)| {
            identity
                .as_ref()
                .map(
                    |&(file_size, file_mtime, input_fingerprint)| AudioAnalysisIdentity {
                        file_path,
                        file_size,
                        file_mtime,
                        input_fingerprint,
                    },
                )
        })
        .collect();
    // 1 bind var per path + 2 for analyzer and version; 896 + 2 = 898, under 999.
    const MAX_PATHS: usize = 896;
    let mut result = HashSet::with_capacity(unique_identities.len());
    for chunk in unique_identities.chunks(MAX_PATHS) {
        use std::fmt::Write as _;
        let mut placeholders = String::new();
        for i in 0..chunk.len() {
            if i > 0 {
                placeholders.push(',');
            }
            write!(placeholders, "?{}", i + 1).unwrap();
        }
        let analyzer_pos = chunk.len() + 1;
        let version_pos = chunk.len() + 2;
        let sql = format!(
            "SELECT file_path, file_size, file_mtime, input_fingerprint \
             FROM audio_analysis_cache \
             WHERE file_path IN ({placeholders}) \
               AND analyzer = ?{analyzer_pos} \
               AND analysis_version = ?{version_pos}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind_values: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() + 2);
        for identity in chunk {
            bind_values.push(&identity.file_path);
        }
        bind_values.push(&analyzer);
        bind_values.push(&analysis_version);
        let rows = stmt.query_map(bind_values.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (file_path, file_size, file_mtime, input_fingerprint) = row?;
            if expected
                .get(file_path.as_str())
                .and_then(Option::as_ref)
                .is_some_and(|expected| {
                    *expected == (file_size, file_mtime, input_fingerprint.as_str())
                })
            {
                result.insert(file_path);
            }
        }
    }
    Ok(result)
}

fn coalesce_audio_analysis_identities<'a>(
    identities: &[AudioAnalysisIdentity<'a>],
) -> HashMap<&'a str, Option<(i64, i64, &'a str)>> {
    let mut expected: HashMap<&'a str, Option<(i64, i64, &'a str)>> =
        HashMap::with_capacity(identities.len());
    for identity in identities {
        let value = (
            identity.file_size,
            identity.file_mtime,
            identity.input_fingerprint,
        );
        expected
            .entry(identity.file_path)
            .and_modify(|current| {
                if current.is_some_and(|existing| existing != value) {
                    *current = None;
                }
            })
            .or_insert(Some(value));
    }
    expected
}

#[cfg(test)]
pub fn set_audio_analysis(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
    file_size: i64,
    file_mtime: i64,
    analysis_version: &str,
    features_json: &str,
) -> Result<(), rusqlite::Error> {
    set_audio_analysis_with_fingerprint(
        conn,
        file_path,
        analyzer,
        file_size,
        file_mtime,
        analysis_version,
        "",
        features_json,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn set_audio_analysis_with_fingerprint(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
    file_size: i64,
    file_mtime: i64,
    analysis_version: &str,
    input_fingerprint: &str,
    features_json: &str,
) -> Result<(), rusqlite::Error> {
    let valid_stratum_fingerprint = input_fingerprint
        == crate::audio::STRATUM_HMM_INPUT_FINGERPRINT
        || input_fingerprint
            .strip_prefix("grid:v1:")
            .is_some_and(|hash| !hash.is_empty());
    if analyzer == crate::audio::ANALYZER_STRATUM && !valid_stratum_fingerprint {
        return Err(rusqlite::Error::InvalidParameterName(
            "Stratum input_fingerprint must use grid:v1:... or hmm:v1".to_string(),
        ));
    }
    conn.execute(
        "INSERT INTO audio_analysis_cache (file_path, analyzer, file_size, file_mtime, analysis_version, input_fingerprint, features_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(file_path, analyzer)
         DO UPDATE SET file_size = ?3, file_mtime = ?4, analysis_version = ?5, input_fingerprint = ?6, features_json = ?7, created_at = datetime('now')",
        params![file_path, analyzer, file_size, file_mtime, analysis_version, input_fingerprint, features_json],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Timbral normalization stats (for pool compatibility z-score normalization)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct TimbralNormStats {
    pub dims: Vec<(f64, f64)>, // (mean, stddev) per dimension
    pub sample_count: i64,
    pub source_fingerprint: String,
    pub analysis_version: String,
    pub vector_schema_version: String,
}

pub fn get_timbral_norm_stats(
    conn: &Connection,
) -> Result<Option<TimbralNormStats>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT dimension_index, mean, stddev, sample_count,
                source_fingerprint, analysis_version, vector_schema_version
         FROM timbral_norm_stats
         ORDER BY dimension_index",
    )?;
    let rows: Vec<(i64, f64, f64, i64, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    if rows.is_empty() {
        return Ok(None);
    }

    let sample_count = rows[0].3;
    let source_fingerprint = rows[0].4.clone();
    let analysis_version = rows[0].5.clone();
    let vector_schema_version = rows[0].6.clone();

    let coherent = rows.iter().enumerate().all(|(expected_index, row)| {
        row.0 == expected_index as i64
            && row.3 == sample_count
            && row.4 == source_fingerprint
            && row.5 == analysis_version
            && row.6 == vector_schema_version
    });
    if !coherent {
        return Ok(None);
    }

    let dims: Vec<(f64, f64)> = rows.iter().map(|r| (r.1, r.2)).collect();

    Ok(Some(TimbralNormStats {
        dims,
        sample_count,
        source_fingerprint,
        analysis_version,
        vector_schema_version,
    }))
}

pub fn save_timbral_norm_stats(
    conn: &Connection,
    stats: &TimbralNormStats,
) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM timbral_norm_stats", [])?;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO timbral_norm_stats (
             dimension_index,
             mean,
             stddev,
             sample_count,
             source_fingerprint,
             analysis_version,
             vector_schema_version
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (i, (mean, stddev)) in stats.dims.iter().enumerate() {
        stmt.execute(params![
            i as i64,
            mean,
            stddev,
            stats.sample_count,
            stats.source_fingerprint,
            stats.analysis_version,
            stats.vector_schema_version,
        ])?;
    }
    drop(stmt);
    tx.commit()?;
    Ok(())
}

pub fn clear_timbral_norm_stats(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM timbral_norm_stats", [])?;
    Ok(())
}
