use std::fmt;
use std::fs;
use std::path::{Component, Path};
use std::sync::OnceLock;

use serde::Deserialize;

pub const REKORDBOX_DOCS_ROOT: &str = "docs/rekordbox";
pub const REKORDBOX_MANIFEST_PATH: &str = "docs/rekordbox/manifest.yaml";

static REKORDBOX_INDEX: OnceLock<Result<CorpusIndex, String>> = OnceLock::new();

#[derive(Debug)]
pub enum CorpusError {
    Io(std::io::Error),
    Yaml(serde_yaml::Error),
    InvalidManifest(String),
    Load(String),
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "manifest read failed: {error}"),
            Self::Yaml(error) => write!(f, "manifest parse failed: {error}"),
            Self::InvalidManifest(message) => write!(f, "invalid manifest: {message}"),
            Self::Load(message) => write!(f, "corpus load failed: {message}"),
        }
    }
}

impl std::error::Error for CorpusError {}

impl From<std::io::Error> for CorpusError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_yaml::Error> for CorpusError {
    fn from(value: serde_yaml::Error) -> Self {
        Self::Yaml(value)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CorpusManifest {
    pub schema_version: u32,
    pub corpus: String,
    pub description: Option<String>,
    pub software_version: Option<String>,
    pub last_updated: Option<String>,
    pub source_documents: Option<SourceDocuments>,
    pub taxonomy: Option<Taxonomy>,
    #[serde(default)]
    pub documents: Vec<ManifestDocument>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceDocuments {
    pub pdfs: Option<usize>,
    pub faq_items: Option<usize>,
    pub web_pages: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Taxonomy {
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub modes: Vec<String>,
    #[serde(default)]
    pub types: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestDocument {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub doc_type: String,
    pub path: String,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub modes: Vec<String>,
    pub confidence: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CorpusQuery<'a> {
    pub topic: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub doc_type: Option<&'a str>,
    pub search_text: Option<&'a str>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusHit {
    pub id: String,
    pub title: String,
    pub doc_type: String,
    pub path: String,
    pub score: i32,
}

#[derive(Debug, Clone)]
pub struct CorpusIndex {
    #[cfg(test)]
    manifest: CorpusManifest,
    documents: Vec<IndexedDocument>,
}

#[derive(Debug, Clone)]
struct IndexedDocument {
    manifest_order: usize,
    document: ManifestDocument,
    repo_relative_path: String,
    searchable_text: String,
    id_lower: String,
    title_lower: String,
    doc_type_lower: String,
    path_lower: String,
    topics_lower: Vec<String>,
    modes_lower: Vec<String>,
}

#[derive(Debug, Clone)]
struct NormalizedQuery {
    topic: Option<String>,
    mode: Option<String>,
    doc_type: Option<String>,
    search_phrase: Option<String>,
    search_terms: Vec<String>,
    limit: Option<usize>,
}

impl<'a> From<CorpusQuery<'a>> for NormalizedQuery {
    fn from(value: CorpusQuery<'a>) -> Self {
        let topic = normalize_optional_filter(value.topic);
        let mode = normalize_optional_filter(value.mode);
        let doc_type = normalize_optional_filter(value.doc_type);
        let search_phrase = normalize_optional_filter(value.search_text);
        let search_terms = search_phrase
            .as_deref()
            .map(|phrase| {
                phrase
                    .split_whitespace()
                    .filter(|segment| !segment.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let limit = value.limit.filter(|limit| *limit > 0);

        Self {
            topic,
            mode,
            doc_type,
            search_phrase,
            search_terms,
            limit,
        }
    }
}

impl CorpusIndex {
    pub fn from_manifest_path(path: impl AsRef<Path>) -> Result<Self, CorpusError> {
        let raw = fs::read_to_string(path)?;
        Self::from_manifest_str(&raw)
    }

    pub fn from_manifest_str(raw: &str) -> Result<Self, CorpusError> {
        let manifest: CorpusManifest = serde_yaml::from_str(raw)?;
        Self::from_manifest(manifest)
    }

    pub fn load_rekordbox() -> Result<Self, CorpusError> {
        Self::from_manifest_path(REKORDBOX_MANIFEST_PATH)
    }

    #[cfg(test)]
    pub fn manifest(&self) -> &CorpusManifest {
        &self.manifest
    }

    pub fn query(&self, query: CorpusQuery<'_>) -> Vec<CorpusHit> {
        let query = NormalizedQuery::from(query);
        let mut scored: Vec<(&IndexedDocument, i32)> = self
            .documents
            .iter()
            .filter_map(|document| {
                if !document_matches_filters(document, &query) {
                    return None;
                }
                let score = score_document(document, &query);
                Some((document, score))
            })
            .collect();

        scored.sort_by(|(left_doc, left_score), (right_doc, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| {
                    left_doc
                        .repo_relative_path
                        .cmp(&right_doc.repo_relative_path)
                })
                .then_with(|| left_doc.document.id.cmp(&right_doc.document.id))
                .then_with(|| left_doc.manifest_order.cmp(&right_doc.manifest_order))
        });

        if let Some(limit) = query.limit {
            scored.truncate(limit);
        }

        scored
            .into_iter()
            .map(|(document, score)| CorpusHit {
                id: document.document.id.clone(),
                title: document.document.title.clone(),
                doc_type: document.document.doc_type.clone(),
                path: document.repo_relative_path.clone(),
                score,
            })
            .collect()
    }

    pub fn consulted_paths(&self, query: CorpusQuery<'_>) -> Vec<String> {
        self.query(query).into_iter().map(|hit| hit.path).collect()
    }

    fn from_manifest(manifest: CorpusManifest) -> Result<Self, CorpusError> {
        if manifest.documents.is_empty() {
            return Err(CorpusError::InvalidManifest(
                "manifest has no documents".to_string(),
            ));
        }
        touch_manifest_metadata(&manifest);

        let mut documents = Vec::with_capacity(manifest.documents.len());
        for (manifest_order, document) in manifest.documents.iter().cloned().enumerate() {
            let repo_relative_path =
                to_repo_relative_doc_path(&document.path).ok_or_else(|| {
                    CorpusError::InvalidManifest(format!(
                        "document '{}' has invalid relative path '{}'",
                        document.id, document.path
                    ))
                })?;
            let id_lower = document.id.to_ascii_lowercase();
            let title_lower = document.title.to_ascii_lowercase();
            let doc_type_lower = document.doc_type.to_ascii_lowercase();
            let path_lower = document.path.to_ascii_lowercase();
            let topics_lower = document
                .topics
                .iter()
                .map(|topic| topic.to_ascii_lowercase())
                .collect::<Vec<_>>();
            let modes_lower = document
                .modes
                .iter()
                .map(|mode| mode.to_ascii_lowercase())
                .collect::<Vec<_>>();
            let searchable_text = [
                id_lower.as_str(),
                title_lower.as_str(),
                doc_type_lower.as_str(),
                path_lower.as_str(),
                &topics_lower.join(" "),
                &modes_lower.join(" "),
            ]
            .join(" ");

            documents.push(IndexedDocument {
                manifest_order,
                document,
                repo_relative_path,
                searchable_text,
                id_lower,
                title_lower,
                doc_type_lower,
                path_lower,
                topics_lower,
                modes_lower,
            });
        }

        Ok(Self {
            #[cfg(test)]
            manifest,
            documents,
        })
    }
}

pub fn rekordbox_index() -> Result<&'static CorpusIndex, CorpusError> {
    let loaded = REKORDBOX_INDEX
        .get_or_init(|| CorpusIndex::load_rekordbox().map_err(|error| format!("{error}")));

    match loaded {
        Ok(index) => Ok(index),
        Err(message) => Err(CorpusError::Load(message.clone())),
    }
}

fn normalize_optional_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn to_repo_relative_doc_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Reject Windows-style absolute/path-separator forms even on Unix hosts.
    if trimmed.contains('\\') || looks_like_windows_drive_path(trimmed) {
        return None;
    }

    let input = Path::new(trimmed);
    if input.is_absolute() {
        return None;
    }

    let mut parts = Vec::new();
    for component in input.components() {
        match component {
            Component::Normal(segment) => parts.push(segment.to_str()?.to_string()),
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir => return None,
            _ => return None,
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(format!("{REKORDBOX_DOCS_ROOT}/{}", parts.join("/")))
}

fn looks_like_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn touch_manifest_metadata(manifest: &CorpusManifest) {
    let _ = (
        manifest.schema_version,
        manifest.corpus.as_str(),
        manifest.description.as_deref(),
        manifest.software_version.as_deref(),
        manifest.last_updated.as_deref(),
    );

    if let Some(source_documents) = manifest.source_documents.as_ref() {
        let _ = (
            source_documents.pdfs,
            source_documents.faq_items,
            source_documents.web_pages,
        );
    }

    if let Some(taxonomy) = manifest.taxonomy.as_ref() {
        let _ = (&taxonomy.topics, &taxonomy.modes, &taxonomy.types);
    }

    for document in &manifest.documents {
        let _ = document.confidence.as_deref();
    }
}

fn document_matches_filters(document: &IndexedDocument, query: &NormalizedQuery) -> bool {
    if let Some(topic) = query.topic.as_deref()
        && !document.topics_lower.iter().any(|item| item == topic)
    {
        return false;
    }

    if let Some(mode) = query.mode.as_deref()
        && !document.modes_lower.iter().any(|item| item == mode)
    {
        return false;
    }

    if let Some(doc_type) = query.doc_type.as_deref()
        && document.doc_type_lower != doc_type
    {
        return false;
    }

    if !query.search_terms.is_empty()
        && !query
            .search_terms
            .iter()
            .all(|term| document.searchable_text.contains(term))
    {
        return false;
    }

    true
}

fn score_document(document: &IndexedDocument, query: &NormalizedQuery) -> i32 {
    let mut score = 0;

    if query.topic.is_some() {
        score += 50;
    }
    if query.mode.is_some() {
        score += 40;
    }
    if query.doc_type.is_some() {
        score += 35;
    }

    if let Some(search_phrase) = query.search_phrase.as_deref() {
        if document.title_lower.contains(search_phrase) {
            score += 30;
        }
        if document.id_lower.contains(search_phrase) {
            score += 20;
        }
        if document.path_lower.contains(search_phrase) {
            score += 20;
        }
    }

    for term in &query.search_terms {
        if document.id_lower == *term {
            score += 60;
        }
        if document
            .title_lower
            .split_whitespace()
            .any(|token| token == term)
        {
            score += 30;
        }
        if document.title_lower.contains(term) {
            score += 18;
        }
        if document.doc_type_lower == *term {
            score += 24;
        }
        if document.topics_lower.iter().any(|topic| topic == term) {
            score += 20;
        }
        if document.modes_lower.iter().any(|mode| mode == term) {
            score += 16;
        }
        if document.path_lower.contains(term) {
            score += 10;
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_manifest() -> &'static str {
        r#"
schema_version: 1
corpus: rekordbox
documents:
  - id: beta-sync
    title: Sync Guide
    type: guide
    path: guides/b-sync.md
    topics: [cloud]
    modes: [common]
  - id: alpha-sync
    title: Sync Guide
    type: guide
    path: guides/a-sync.md
    topics: [cloud]
    modes: [common]
  - id: xml-reference
    title: XML Import Export Reference
    type: reference
    path: reference/xml-import-export.md
    topics: [xml, export]
    modes: [common, export]
  - id: xml-guide
    title: XML Format Spec
    type: guide
    path: guides/xml-format-spec.md
    topics: [xml, export]
    modes: [common, export]
  - id: library-faq
    title: Library Collection FAQ
    type: faq
    path: faq/library-and-collection.md
    topics: [library]
    modes: [common]
"#
    }

    #[test]
    fn optional_filters_are_safe_and_paths_are_repo_relative() {
        let index = CorpusIndex::from_manifest_str(fixture_manifest()).expect("fixture parses");

        let hits = index.query(CorpusQuery::default());

        assert_eq!(hits.len(), 5);
        assert_eq!(
            hits.iter().map(|hit| hit.id.as_str()).collect::<Vec<_>>(),
            vec![
                "library-faq",
                "alpha-sync",
                "beta-sync",
                "xml-guide",
                "xml-reference"
            ]
        );
        assert!(
            hits.iter()
                .all(|hit| hit.path.starts_with("docs/rekordbox/"))
        );

        let paths = index.consulted_paths(CorpusQuery::default());
        assert_eq!(paths.len(), hits.len());
        assert!(paths.iter().all(|path| path.starts_with("docs/rekordbox/")));
    }

    #[test]
    fn topic_mode_type_and_search_filters_work_together() {
        let index = CorpusIndex::from_manifest_str(fixture_manifest()).expect("fixture parses");

        let hits = index.query(CorpusQuery {
            topic: Some("xml"),
            mode: Some("export"),
            doc_type: Some("reference"),
            search_text: Some("import"),
            limit: None,
        });

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "xml-reference");
        assert_eq!(
            hits[0].path,
            "docs/rekordbox/reference/xml-import-export.md"
        );
    }

    #[test]
    fn ranking_is_deterministic_for_equal_scores() {
        let index = CorpusIndex::from_manifest_str(fixture_manifest()).expect("fixture parses");
        let query = CorpusQuery {
            topic: Some("cloud"),
            mode: Some("common"),
            doc_type: Some("guide"),
            search_text: Some("sync"),
            limit: None,
        };

        let first = index.query(query.clone());
        assert_eq!(
            first.iter().map(|hit| hit.id.as_str()).collect::<Vec<_>>(),
            vec!["alpha-sync", "beta-sync"]
        );

        for _ in 0..10 {
            let rerun = index.query(query.clone());
            assert_eq!(rerun, first);
        }
    }

    #[test]
    fn rejects_paths_outside_docs_root() {
        let manifest = r#"
schema_version: 1
corpus: rekordbox
documents:
  - id: bad
    title: Bad
    type: guide
    path: ../outside.md
    topics: [xml]
    modes: [common]
"#;

        let error =
            CorpusIndex::from_manifest_str(manifest).expect_err("must fail path validation");
        assert!(format!("{error}").contains("invalid relative path"));
    }

    #[test]
    fn rejects_windows_style_drive_paths() {
        for path in ["C:\\docs\\guide.md", "C:/docs/guide.md", "C:docs/guide.md"] {
            let manifest = format!(
                r#"
schema_version: 1
corpus: rekordbox
documents:
  - id: win-abs
    title: Windows Absolute
    type: guide
    path: '{path}'
    topics: [xml]
    modes: [common]
"#
            );

            let error =
                CorpusIndex::from_manifest_str(&manifest).expect_err("must fail path validation");
            assert!(format!("{error}").contains("invalid relative path"));
        }
    }
}
