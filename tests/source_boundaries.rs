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
fn source_layers_follow_dependency_direction() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();

    for rule in LAYER_RULES {
        let layer_root = source_root.join(rule.directory);
        for path in rust_files(&layer_root) {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            for reference in crate_root_references(&source) {
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
        "source dependency direction violations:\n{}",
        violations.join("\n")
    );
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
        // crate::cli must not count
        /* crate::bootstrap must not count */
        const TEXT: &str = "crate::mcp must not count";
        const RAW: &str = r#"crate::adapters must not count"#;
    "###;

    let references = crate_root_references(source)
        .into_iter()
        .map(|reference| reference.root)
        .collect::<Vec<_>>();
    assert_eq!(references, ["application", "adapters", "domain", "mcp"]);
}
