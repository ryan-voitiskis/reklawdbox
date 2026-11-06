//! Translate Rekordbox `FolderPath` values between the machine that owns the
//! library and the machine running reklawdbox.
//!
//! Rekordbox stores absolute paths from its own host (for example
//! `N:/media/music/shared/...` on Windows). When reklawdbox runs elsewhere —
//! WSL, a Linux box, a container — the same files are reachable under another
//! prefix (`/mnt/data/media/music/shared/...`). `REKLAWDBOX_PATH_MAP` declares
//! those prefixes:
//!
//! ```text
//! REKLAWDBOX_PATH_MAP="N:/media/music=/mnt/data/media/music;D:/Samples=/srv/samples"
//! ```
//!
//! Each rule is `DB_PREFIX=LOCAL_PREFIX`; rules are separated by `;` or new
//! lines. The longest matching prefix wins. DB prefixes match ASCII
//! case-insensitively and accept `\` or `/` (Windows paths); local prefixes
//! match exactly. Prefixes only match on whole path segments.
//!
//! Paths read from the database go through [`to_local`]; anything written
//! back or compared against `FolderPath` (XML `Location`, SQL filters, ANLZ
//! lookups) goes through [`to_db`]. With no rules configured both are the
//! identity, so macOS behaviour is unchanged.

use std::sync::OnceLock;

pub(crate) const PATH_MAP_ENV: &str = "REKLAWDBOX_PATH_MAP";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    db_prefix: String,
    local_prefix: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PathMap {
    rules: Vec<Rule>,
}

impl PathMap {
    pub(crate) fn parse(spec: &str) -> Result<Self, String> {
        let mut rules = Vec::new();
        for raw in spec.split([';', '\n']) {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let (db, local) = raw.split_once('=').ok_or_else(|| {
                format!("{PATH_MAP_ENV}: rule {raw:?} must look like DB_PREFIX=LOCAL_PREFIX")
            })?;
            let db_prefix = normalize_db(db.trim()).trim_end_matches('/').to_string();
            let local_prefix = local.trim().trim_end_matches('/').to_string();
            if db_prefix.is_empty() || local_prefix.is_empty() {
                return Err(format!(
                    "{PATH_MAP_ENV}: rule {raw:?} has an empty prefix (use a full directory)"
                ));
            }
            rules.push(Rule {
                db_prefix,
                local_prefix,
            });
        }
        // Longest prefix first so nested rules win over their parents.
        rules.sort_by_key(|rule| std::cmp::Reverse(rule.db_prefix.len()));
        Ok(Self { rules })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Database path → path usable on this machine. Unmapped paths are returned unchanged.
    pub(crate) fn to_local(&self, db_path: &str) -> String {
        if self.rules.is_empty() {
            return db_path.to_string();
        }
        let normalized = normalize_db(db_path);
        for rule in &self.rules {
            if let Some(rest) = strip_prefix_ascii_ci(&normalized, &rule.db_prefix) {
                return format!("{}{rest}", rule.local_prefix);
            }
        }
        db_path.to_string()
    }

    /// Path on this machine → the form Rekordbox stores. Unmapped paths are returned unchanged.
    pub(crate) fn to_db(&self, local_path: &str) -> String {
        let mut best: Option<&Rule> = None;
        for rule in &self.rules {
            if strip_prefix_exact(local_path, &rule.local_prefix).is_some()
                && best.is_none_or(|b| rule.local_prefix.len() > b.local_prefix.len())
            {
                best = Some(rule);
            }
        }
        match best {
            Some(rule) => {
                let rest = strip_prefix_exact(local_path, &rule.local_prefix).unwrap_or_default();
                format!("{}{rest}", rule.db_prefix)
            }
            None => local_path.to_string(),
        }
    }
}

fn normalize_db(path: &str) -> String {
    path.replace('\\', "/")
}

/// Returns the remainder (starting with `/`, or empty) when `prefix` matches a
/// whole leading path segment of `path`, ignoring ASCII case.
fn strip_prefix_ascii_ci<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let head = path.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = &path[prefix.len()..];
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

fn strip_prefix_exact<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

/// Process-wide map loaded once from `REKLAWDBOX_PATH_MAP`.
///
/// An invalid value is logged and ignored rather than aborting startup: the
/// server still works, it just cannot reach the audio files.
pub(crate) fn global() -> &'static PathMap {
    static MAP: OnceLock<PathMap> = OnceLock::new();
    MAP.get_or_init(|| match std::env::var(PATH_MAP_ENV) {
        Ok(spec) => match PathMap::parse(&spec) {
            Ok(map) => {
                if !map.is_empty() {
                    tracing::info!("{PATH_MAP_ENV}: {} rule(s) loaded", map.rules.len());
                }
                map
            }
            Err(error) => {
                tracing::error!("{error}; ignoring {PATH_MAP_ENV}");
                PathMap::default()
            }
        },
        Err(_) => PathMap::default(),
    })
}

pub(crate) fn to_local(db_path: &str) -> String {
    global().to_local(db_path)
}

pub(crate) fn to_db(local_path: &str) -> String {
    global().to_db(local_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> PathMap {
        PathMap::parse(
            "N:/media/music=/mnt/data/media/music/; N:/media/music/shared=/musica/shared\nD:\\Samples=/srv/samples",
        )
        .unwrap()
    }

    #[test]
    fn empty_map_is_identity() {
        let m = PathMap::parse("").unwrap();
        assert!(m.is_empty());
        assert_eq!(m.to_local("N:/a/b.mp3"), "N:/a/b.mp3");
        assert_eq!(m.to_db("/x/b.mp3"), "/x/b.mp3");
    }

    #[test]
    fn longest_db_prefix_wins() {
        let m = map();
        assert_eq!(
            m.to_local("N:/media/music/shared/A/01 - x.flac"),
            "/musica/shared/A/01 - x.flac"
        );
        assert_eq!(
            m.to_local("N:/media/music/biblioteca/B/02.mp3"),
            "/mnt/data/media/music/biblioteca/B/02.mp3"
        );
    }

    #[test]
    fn db_prefix_is_case_insensitive_and_accepts_backslashes() {
        let m = map();
        assert_eq!(
            m.to_local(r"n:\Media\Music\x.mp3"),
            "/mnt/data/media/music/x.mp3"
        );
        assert_eq!(m.to_local(r"D:\Samples\kick.wav"), "/srv/samples/kick.wav");
    }

    #[test]
    fn prefix_only_matches_whole_segments() {
        let m = map();
        assert_eq!(m.to_local("N:/media/musicX/a.mp3"), "N:/media/musicX/a.mp3");
        assert_eq!(m.to_db("/musica/sharedX/a.mp3"), "/musica/sharedX/a.mp3");
    }

    #[test]
    fn unmapped_and_streaming_paths_pass_through() {
        let m = map();
        assert_eq!(m.to_local("/v4/catalog/tracks/1/"), "/v4/catalog/tracks/1/");
        assert_eq!(m.to_local(""), "");
    }

    #[test]
    fn round_trip_restores_db_form() {
        let m = map();
        for db in [
            "N:/media/music/shared/Á/ção.flac",
            "N:/media/music/biblioteca/B/02.mp3",
            "D:/Samples/kick.wav",
        ] {
            assert_eq!(m.to_db(&m.to_local(db)), db);
        }
    }

    #[test]
    fn to_db_maps_directory_prefixes_for_sql_filters() {
        let m = map();
        assert_eq!(m.to_db("/musica/shared"), "N:/media/music/shared");
        assert_eq!(m.to_db("/musica/shared/"), "N:/media/music/shared/");
    }

    #[test]
    fn rejects_malformed_rules() {
        assert!(PathMap::parse("N:/media").is_err());
        assert!(PathMap::parse("=/mnt/x").is_err());
        assert!(PathMap::parse("N:/x=").is_err());
    }
}
