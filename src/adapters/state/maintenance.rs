use rusqlite::Connection;

/// Clear all caches except broker sessions. Returns row counts per table.
pub fn clear_caches(conn: &Connection) -> Result<ClearCachesResult, rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    let enrichment = tx.execute("DELETE FROM enrichment_cache", [])?;
    let audio_analysis = tx.execute("DELETE FROM audio_analysis_cache", [])?;
    // audit_issues before audit_files: child rows first to get accurate counts
    // (ON DELETE CASCADE would handle it, but then audit_issues count would be 0)
    let audit_issues = tx.execute("DELETE FROM audit_issues", [])?;
    let audit_files = tx.execute("DELETE FROM audit_files", [])?;
    tx.execute("DELETE FROM timbral_norm_stats", [])?;
    tx.commit()?;
    Ok(ClearCachesResult {
        enrichment,
        audio_analysis,
        audit_issues,
        audit_files,
    })
}

pub struct ClearCachesResult {
    pub enrichment: usize,
    pub audio_analysis: usize,
    pub audit_issues: usize,
    pub audit_files: usize,
}
