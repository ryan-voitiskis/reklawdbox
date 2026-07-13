use rusqlite::{Connection, params};

// ---------------------------------------------------------------------------
// Weight presets (user-saved scoring weight configurations)
// ---------------------------------------------------------------------------

pub struct WeightPresetEntry {
    pub name: String,
    pub scorer_type: String,
    pub weights_json: String,
}

pub fn save_weight_preset(
    conn: &Connection,
    name: &str,
    scorer_type: &str,
    weights_json: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO weight_presets (name, scorer_type, weights_json)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(name, scorer_type)
         DO UPDATE SET weights_json = ?3, updated_at = datetime('now')",
        params![name, scorer_type, weights_json],
    )?;
    Ok(())
}

pub fn list_weight_presets(
    conn: &Connection,
    scorer_type: Option<&str>,
) -> Result<Vec<WeightPresetEntry>, rusqlite::Error> {
    if let Some(st) = scorer_type {
        let mut stmt = conn.prepare_cached(
            "SELECT name, scorer_type, weights_json FROM weight_presets
             WHERE scorer_type = ?1 ORDER BY name",
        )?;
        let rows = stmt
            .query_map(params![st], |row| {
                Ok(WeightPresetEntry {
                    name: row.get(0)?,
                    scorer_type: row.get(1)?,
                    weights_json: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    } else {
        let mut stmt = conn.prepare_cached(
            "SELECT name, scorer_type, weights_json FROM weight_presets ORDER BY scorer_type, name",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(WeightPresetEntry {
                    name: row.get(0)?,
                    scorer_type: row.get(1)?,
                    weights_json: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}

pub fn get_weight_preset(
    conn: &Connection,
    name: &str,
    scorer_type: &str,
) -> Result<Option<String>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT weights_json FROM weight_presets
         WHERE name = ?1 AND scorer_type = ?2",
    )?;
    let mut rows = stmt.query_map(params![name, scorer_type], |row| row.get::<_, String>(0))?;
    rows.next().transpose()
}

pub fn delete_weight_preset(
    conn: &Connection,
    name: &str,
    scorer_type: &str,
) -> Result<bool, rusqlite::Error> {
    let deleted = conn.execute(
        "DELETE FROM weight_presets WHERE name = ?1 AND scorer_type = ?2",
        params![name, scorer_type],
    )?;
    Ok(deleted > 0)
}
