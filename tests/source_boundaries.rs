//! Mechanical checks for the source-layer dependency direction in `src/README.md`.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
struct LayerRule {
    directory: &'static str,
    forbidden_roots: &'static [&'static str],
}

#[derive(Debug, PartialEq, Eq)]
struct CrateRootReference {
    root: String,
    byte_offset: usize,
}

const LAYER_RULES: &[LayerRule] = &[
    LayerRule {
        directory: "domain",
        forbidden_roots: &["adapters", "application", "bootstrap", "cli", "mcp"],
    },
    LayerRule {
        directory: "application",
        forbidden_roots: &["bootstrap", "cli", "mcp"],
    },
    LayerRule {
        directory: "adapters",
        forbidden_roots: &["application", "bootstrap", "cli", "mcp"],
    },
    LayerRule {
        directory: "cli",
        forbidden_roots: &["mcp"],
    },
    LayerRule {
        directory: "mcp",
        forbidden_roots: &["cli"],
    },
];

#[test]
fn explicit_source_paths_follow_dependency_direction() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();

    for rule in LAYER_RULES {
        let layer_root = source_root.join(rule.directory);
        for path in rust_files(&layer_root) {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            if let Some(byte_offset) = crate_alias_offset(&source) {
                let relative = path.strip_prefix(&source_root).unwrap_or(&path);
                violations.push(format!(
                    "{}:{} defines a crate alias that can bypass layer checks",
                    relative.display(),
                    line_number(&source, byte_offset),
                ));
            }
            for reference in explicit_root_references(&source) {
                if rule.forbidden_roots.contains(&reference.root.as_str()) {
                    let relative = path.strip_prefix(&source_root).unwrap_or(&path);
                    violations.push(format!(
                        "{}:{} references forbidden crate::{} from {}",
                        relative.display(),
                        line_number(&source, reference.byte_offset),
                        reference.root,
                        rule.directory,
                    ));
                }
            }
        }
    }

    violations.sort();
    assert!(
        violations.is_empty(),
        "explicit source-path dependency direction violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn hydration_cache_and_outcome_coordination_stays_out_of_cli() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cli_root = manifest.join("src/cli/hydrate");
    let cli_source = rust_files(&cli_root)
        .into_iter()
        .map(|path| {
            std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        })
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "get_enrichment(",
        "set_enrichment(",
        "serialize_cache_payload(",
        "run_hydrate_cache_writer(",
        "match_quality: Some(\"error\"",
    ] {
        assert!(
            !cli_source.contains(forbidden),
            "CLI hydrate must not own {forbidden:?} coordination"
        );
    }
    for application_call in [
        "select_hydration_work(",
        "hydrate_discogs_track(",
        "HydrateCacheWriterSession::start(",
        "HydrationApplicationReport::assemble(",
        ".final_accounting()",
    ] {
        assert!(
            cli_source.contains(application_call),
            "CLI hydrate should delegate through {application_call:?}"
        );
    }

    let application_source =
        std::fs::read_to_string(manifest.join("src/application/enrichment/hydrate.rs"))
            .expect("application hydration source should read");
    for owner in [
        "pub(crate) fn select_hydration_work(",
        "pub(crate) async fn hydrate_discogs_track",
        "pub(crate) struct HydrateCacheWriterSession",
        "pub(crate) struct HydrationApplicationReport",
        "pub(crate) fn final_accounting(&self)",
    ] {
        assert!(
            application_source.contains(owner),
            "application hydration should own {owner:?}"
        );
    }
}

#[test]
fn platform_process_group_stays_synchronous_and_transport_free() {
    let source = std::fs::read_to_string("src/adapters/platform/process_group.rs")
        .expect("platform process-group source should be readable");
    for forbidden in [
        "tokio::",
        "tokio_",
        "Child",
        "Command",
        "AsyncRead",
        "crate::adapters::audio",
        "crate::adapters::rekordbox",
        "crate::cli",
        "crate::mcp",
    ] {
        assert!(
            !source.contains(forbidden),
            "platform process-group primitive must not import or own {forbidden}"
        );
    }
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    fn visit(directory: &Path, files: &mut Vec<PathBuf>) {
        let mut entries = std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
            .map(|entry| entry.expect("source directory entry should be readable"))
            .collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::file_name);

        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(root, &mut files);
    files
}

fn crate_root_references(source: &str) -> Vec<CrateRootReference> {
    let code = code_only(source);
    let mut references = Vec::new();
    let mut cursor = 0;

    while cursor < code.len() {
        if !is_identifier_start(code[cursor]) {
            cursor += 1;
            continue;
        }

        let identifier_start = cursor;
        cursor = identifier_end(&code, cursor);
        if &code[identifier_start..cursor] != b"crate" {
            continue;
        }

        let mut path_cursor = skip_whitespace(&code, cursor);
        if code.get(path_cursor..path_cursor + 2) != Some(b"::") {
            continue;
        }
        path_cursor = skip_whitespace(&code, path_cursor + 2);

        if code.get(path_cursor) == Some(&b'{') {
            references.extend(grouped_crate_roots(&code, path_cursor + 1));
        } else if code
            .get(path_cursor)
            .copied()
            .is_some_and(is_identifier_start)
        {
            let end = identifier_end(&code, path_cursor);
            references.push(CrateRootReference {
                root: String::from_utf8_lossy(&code[path_cursor..end]).into_owned(),
                byte_offset: path_cursor,
            });
        }
    }

    references
}

fn explicit_root_references(source: &str) -> Vec<CrateRootReference> {
    let mut references = crate_root_references(source);
    references.extend(upward_root_references(source));
    references.sort_by(|left, right| {
        (left.byte_offset, left.root.as_str()).cmp(&(right.byte_offset, right.root.as_str()))
    });
    references.dedup();
    references
}

fn upward_root_references(source: &str) -> Vec<CrateRootReference> {
    let code = code_only(source);
    let mut references = Vec::new();
    let mut cursor = 0;

    while cursor < code.len() {
        if !is_identifier_start(code[cursor]) {
            cursor += 1;
            continue;
        }

        let identifier_start = cursor;
        cursor = identifier_end(&code, cursor);
        if &code[identifier_start..cursor] != b"super" {
            continue;
        }

        let mut path_cursor = skip_whitespace(&code, cursor);
        if code.get(path_cursor..path_cursor + 2) != Some(b"::") {
            continue;
        }
        path_cursor = skip_whitespace(&code, path_cursor + 2);

        loop {
            if code.get(path_cursor) == Some(&b'{') {
                references.extend(grouped_crate_roots(&code, path_cursor + 1));
                break;
            }
            if !code
                .get(path_cursor)
                .copied()
                .is_some_and(is_identifier_start)
            {
                break;
            }
            let end = identifier_end(&code, path_cursor);
            if &code[path_cursor..end] != b"super" {
                references.push(CrateRootReference {
                    root: String::from_utf8_lossy(&code[path_cursor..end]).into_owned(),
                    byte_offset: path_cursor,
                });
                break;
            }
            path_cursor = skip_whitespace(&code, end);
            if code.get(path_cursor..path_cursor + 2) != Some(b"::") {
                break;
            }
            path_cursor = skip_whitespace(&code, path_cursor + 2);
        }
    }

    references
}

fn crate_alias_offset(source: &str) -> Option<usize> {
    let code = code_only(source);
    let mut identifiers = Vec::new();
    let mut cursor = 0;
    while cursor < code.len() {
        if is_identifier_start(code[cursor]) {
            let start = cursor;
            cursor = identifier_end(&code, cursor);
            identifiers.push((&code[start..cursor], start));
        } else {
            cursor += 1;
        }
    }

    for window in identifiers.windows(3) {
        if window[0].0 == b"use" && window[1].0 == b"crate" && window[2].0 == b"as" {
            return Some(window[1].1);
        }
    }
    for window in identifiers.windows(4) {
        let extern_alias = window[0].0 == b"extern"
            && window[1].0 == b"crate"
            && window[2].0 == b"self"
            && window[3].0 == b"as";
        let grouped_use_alias = window[0].0 == b"use"
            && window[1].0 == b"crate"
            && window[2].0 == b"self"
            && window[3].0 == b"as";
        if extern_alias || grouped_use_alias {
            return Some(window[1].1);
        }
    }
    None
}

fn grouped_crate_roots(code: &[u8], mut cursor: usize) -> Vec<CrateRootReference> {
    let mut references = Vec::new();
    let mut depth = 1_u32;
    let mut expecting_root = true;

    while cursor < code.len() && depth > 0 {
        cursor = skip_whitespace(code, cursor);
        let Some(byte) = code.get(cursor).copied() else {
            break;
        };

        match byte {
            b'{' => {
                depth += 1;
                cursor += 1;
            }
            b'}' => {
                depth -= 1;
                cursor += 1;
            }
            b',' if depth == 1 => {
                expecting_root = true;
                cursor += 1;
            }
            _ if depth == 1 && expecting_root && is_identifier_start(byte) => {
                let end = identifier_end(code, cursor);
                references.push(CrateRootReference {
                    root: String::from_utf8_lossy(&code[cursor..end]).into_owned(),
                    byte_offset: cursor,
                });
                expecting_root = false;
                cursor = end;
            }
            _ => cursor += 1,
        }
    }

    references
}

fn code_only(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut code = bytes.to_vec();
    let mut cursor = 0;

    while cursor < bytes.len() {
        if bytes.get(cursor..cursor + 2) == Some(b"//") {
            let end = bytes[cursor..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| cursor + offset);
            blank(&mut code, cursor, end);
            cursor = end;
        } else if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            let end = block_comment_end(bytes, cursor);
            blank_preserving_newlines(&mut code, cursor, end);
            cursor = end;
        } else if let Some((content_start, hashes)) = raw_string_start(bytes, cursor) {
            let end = raw_string_end(bytes, content_start, hashes);
            blank_preserving_newlines(&mut code, cursor, end);
            cursor = end;
        } else if let Some(end) = character_literal_end(bytes, cursor) {
            blank_preserving_newlines(&mut code, cursor, end);
            cursor = end;
        } else if bytes[cursor] == b'"'
            || bytes.get(cursor..cursor + 2) == Some(b"b\"")
            || bytes.get(cursor..cursor + 2) == Some(b"c\"")
        {
            let quote = if bytes[cursor] == b'"' {
                cursor
            } else {
                cursor + 1
            };
            let end = quoted_string_end(bytes, quote);
            blank_preserving_newlines(&mut code, cursor, end);
            cursor = end;
        } else {
            cursor += 1;
        }
    }

    code
}

fn character_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = if bytes.get(start) == Some(&b'\'') {
        start
    } else if bytes.get(start..start + 2) == Some(b"b'") {
        start + 1
    } else {
        return None;
    };

    let mut cursor = quote + 1;
    match bytes.get(cursor).copied()? {
        b'\\' => {
            cursor += 1;
            match bytes.get(cursor).copied()? {
                b'x' => {
                    cursor += 1;
                    if !bytes.get(cursor).is_some_and(u8::is_ascii_hexdigit)
                        || !bytes.get(cursor + 1).is_some_and(u8::is_ascii_hexdigit)
                    {
                        return None;
                    }
                    cursor += 2;
                }
                b'u' => {
                    cursor += 1;
                    if bytes.get(cursor) != Some(&b'{') {
                        return None;
                    }
                    cursor += 1;
                    let digits_start = cursor;
                    while bytes.get(cursor).is_some_and(u8::is_ascii_hexdigit) {
                        cursor += 1;
                    }
                    if cursor == digits_start || bytes.get(cursor) != Some(&b'}') {
                        return None;
                    }
                    cursor += 1;
                }
                _ => cursor += 1,
            }
        }
        b'\n' | b'\r' | b'\'' => return None,
        _ => {
            let character = std::str::from_utf8(&bytes[cursor..]).ok()?.chars().next()?;
            cursor += character.len_utf8();
        }
    }

    (bytes.get(cursor) == Some(&b'\'')).then_some(cursor + 1)
}

fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut cursor = start + 2;
    let mut depth = 1_u32;
    while cursor < bytes.len() && depth > 0 {
        if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            depth += 1;
            cursor += 2;
        } else if bytes.get(cursor..cursor + 2) == Some(b"*/") {
            depth -= 1;
            cursor += 2;
        } else {
            cursor += 1;
        }
    }
    cursor
}

fn raw_string_start(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut cursor = start;
    if bytes.get(cursor) == Some(&b'b') || bytes.get(cursor) == Some(&b'c') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'r') {
        return None;
    }
    cursor += 1;

    let hash_start = cursor;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    Some((cursor + 1, cursor - hash_start))
}

fn raw_string_end(bytes: &[u8], mut cursor: usize, hashes: usize) -> usize {
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes
                .get(cursor + 1..cursor + 1 + hashes)
                .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
        {
            return (cursor + 1 + hashes).min(bytes.len());
        }
        cursor += 1;
    }
    bytes.len()
}

fn quoted_string_end(bytes: &[u8], quote: usize) -> usize {
    let mut cursor = quote + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor = (cursor + 2).min(bytes.len()),
            b'"' => return cursor + 1,
            _ => cursor += 1,
        }
    }
    bytes.len()
}

fn blank(bytes: &mut [u8], start: usize, end: usize) {
    bytes[start..end].fill(b' ');
}

fn blank_preserving_newlines(bytes: &mut [u8], start: usize, end: usize) {
    for byte in &mut bytes[start..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn skip_whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    cursor
}

fn identifier_end(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes
        .get(cursor)
        .copied()
        .is_some_and(is_identifier_continue)
    {
        cursor += 1;
    }
    cursor
}

fn is_identifier_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit()
}

fn line_number(source: &str, byte_offset: usize) -> usize {
    source.as_bytes()[..byte_offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

#[test]
fn scanner_handles_grouped_paths_and_ignores_non_code_text() {
    let source = r###"
        use crate::application::workflow;
        use crate::{adapters, domain::{self, Rule}, mcp as transport};
        use super::super::{bootstrap, cli};
        // crate::cli must not count
        /* crate::bootstrap must not count */
        const TEXT: &str = "crate::mcp must not count";
        const RAW: &str = r#"crate::adapters must not count"#;
    "###;

    let references = explicit_root_references(source)
        .into_iter()
        .map(|reference| reference.root)
        .collect::<Vec<_>>();
    assert_eq!(
        references,
        [
            "application",
            "adapters",
            "domain",
            "mcp",
            "bootstrap",
            "cli"
        ]
    );
    assert_eq!(crate_alias_offset("pub use crate as root;"), Some(8));
    assert_eq!(crate_alias_offset("use crate::{self as root};"), Some(4));
    assert_eq!(crate_alias_offset("extern crate self as root;"), Some(7));
    assert_eq!(crate_alias_offset("// use crate as root;"), None);

    let character_literals = r###"
        let quote = '"';
        let byte_quote = b'"';
        let escaped_quote = '\'';
        use crate::mcp::after_character_literals;
    "###;
    let references = crate_root_references(character_literals)
        .into_iter()
        .map(|reference| reference.root)
        .collect::<Vec<_>>();
    assert_eq!(references, ["mcp"]);
}
