//! Native, durable agent memory.
//!
//! Cortana keeps memory in the same SQLite store as indexed knowledge.  The
//! memory table is intentionally separate from documents: source ingestion is
//! evidence capture, while memories are explicit agent- or user-authored
//! conclusions with their own lifecycle, provenance, confidence, and ACL.

use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_MEMORY_TITLE_BYTES: usize = 512;
pub const MAX_MEMORY_CONTENT_BYTES: usize = 64 * 1024;
pub const MAX_MEMORY_PROJECT_BYTES: usize = 256;
pub const MAX_MEMORY_SOURCE_BYTES: usize = 128;
pub const MAX_MEMORY_SOURCE_ID_BYTES: usize = 512;
pub const MAX_MEMORY_DEDUPE_KEY_BYTES: usize = 512;
pub const MAX_MEMORY_PROVENANCE_BYTES: usize = 16 * 1024;
pub const MAX_MEMORY_VALID_UNTIL_BYTES: usize = 64;
pub const MAX_MEMORY_ACL_ENTRIES: usize = 32;
pub const MAX_MEMORY_ACL_BYTES: usize = 256;
pub const MAX_MEMORY_RECALL_LIMIT: usize = 100;
pub const MAX_MEMORY_EXPORT_LIMIT: usize = 100_000;
pub const DEFAULT_MEMORY_MAX_ACTIVE: usize = 100_000;

#[derive(Clone, Copy, Debug)]
pub struct MemoryDefaults {
    pub confidence: f32,
    pub importance: f32,
}

impl Default for MemoryDefaults {
    fn default() -> Self {
        Self {
            confidence: 0.7,
            importance: 0.5,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Procedural,
    Preference,
    Working,
}

impl MemoryKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "episodic" => Ok(Self::Episodic),
            "semantic" => Ok(Self::Semantic),
            "procedural" => Ok(Self::Procedural),
            "preference" => Ok(Self::Preference),
            "working" => Ok(Self::Working),
            other => bail!(
                "unsupported memory kind `{other}`; expected episodic, semantic, procedural, preference, or working"
            ),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
            Self::Preference => "preference",
            Self::Working => "working",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemoryInput {
    pub kind: String,
    pub project: String,
    pub title: String,
    pub content: String,
    #[serde(default = "default_memory_source")]
    pub source: String,
    #[serde(default)]
    pub source_id: String,
    #[serde(default)]
    pub dedupe_key: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default)]
    pub acl: Vec<String>,
    #[serde(default)]
    pub provenance: Value,
    #[serde(default)]
    pub supersedes_id: Option<String>,
    /// Optional RFC3339 expiry for short-lived working context.
    #[serde(default)]
    pub valid_until: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemoryRecord {
    pub id: String,
    pub kind: String,
    pub project: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_id: String,
    pub dedupe_key: Option<String>,
    pub confidence: f32,
    pub importance: f32,
    pub status: String,
    pub acl: Vec<String>,
    pub provenance: Value,
    pub observed_at: String,
    pub valid_from: String,
    pub valid_until: Option<String>,
    pub supersedes_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemorySearchResult {
    #[serde(flatten)]
    pub memory: MemoryRecord,
    pub lexical_score: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MemoryStats {
    /// Active and currently valid memories available to recall.
    pub active: i64,
    /// Active records whose validity window has elapsed. They remain in the
    /// store for export/audit history but are not eligible for recall.
    pub expired: i64,
    pub retracted: i64,
    pub superseded: i64,
    pub total: i64,
}

pub(crate) fn default_memory_source() -> String {
    "agent".into()
}

fn default_confidence() -> f32 {
    0.7
}

fn default_importance() -> f32 {
    0.5
}

pub(crate) fn validate_input(
    input: &MemoryInput,
) -> Result<(MemoryKind, Vec<String>, String, Option<String>)> {
    let kind = MemoryKind::parse(&input.kind)?;
    validate_text("project", &input.project, MAX_MEMORY_PROJECT_BYTES)?;
    validate_text("title", &input.title, MAX_MEMORY_TITLE_BYTES)?;
    validate_text("content", &input.content, MAX_MEMORY_CONTENT_BYTES)?;
    validate_text("source", &input.source, MAX_MEMORY_SOURCE_BYTES)?;
    if !input.source_id.is_empty() {
        validate_text("source_id", &input.source_id, MAX_MEMORY_SOURCE_ID_BYTES)?;
    }
    if let Some(key) = &input.dedupe_key {
        validate_text("dedupe_key", key, MAX_MEMORY_DEDUPE_KEY_BYTES)?;
    }
    if let Some(id) = &input.supersedes_id {
        validate_text("supersedes_id", id, 128)?;
    }
    let valid_until = input
        .valid_until
        .as_deref()
        .map(normalize_valid_until)
        .transpose()?;
    anyhow::ensure!(
        input.confidence.is_finite() && (0.0..=1.0).contains(&input.confidence),
        "confidence must be a finite number between 0 and 1"
    );
    anyhow::ensure!(
        input.importance.is_finite() && (0.0..=1.0).contains(&input.importance),
        "importance must be a finite number between 0 and 1"
    );
    let acl = normalize_acl(&input.project, &input.acl)?;
    let provenance = if input.provenance.is_null() {
        Value::Object(Default::default())
    } else {
        input.provenance.clone()
    };
    let provenance_json = serde_json::to_string(&provenance)?;
    anyhow::ensure!(
        provenance_json.len() <= MAX_MEMORY_PROVENANCE_BYTES,
        "provenance exceeds {MAX_MEMORY_PROVENANCE_BYTES} bytes"
    );
    Ok((kind, acl, provenance_json, valid_until))
}

fn normalize_valid_until(value: &str) -> Result<String> {
    validate_text("valid_until", value, MAX_MEMORY_VALID_UNTIL_BYTES)?;
    let timestamp = DateTime::parse_from_rfc3339(value)
        .map_err(|error| anyhow::anyhow!("valid_until must be RFC3339: {error}"))?;
    Ok(timestamp
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

pub(crate) fn normalize_acl(project: &str, acl: &[String]) -> Result<Vec<String>> {
    let mut values = if acl.is_empty() {
        vec![project.to_string()]
    } else {
        acl.to_vec()
    };
    anyhow::ensure!(
        values.len() <= MAX_MEMORY_ACL_ENTRIES,
        "memory ACL exceeds {MAX_MEMORY_ACL_ENTRIES} entries"
    );
    values.sort();
    values.dedup();
    for value in &values {
        validate_text("memory ACL label", value, MAX_MEMORY_ACL_BYTES)?;
        anyhow::ensure!(
            value != "*",
            "memory ACL cannot contain the reserved `*` label"
        );
    }
    Ok(values)
}

fn validate_text(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{name} must not be empty");
    anyhow::ensure!(value.len() <= max_bytes, "{name} exceeds {max_bytes} bytes");
    anyhow::ensure!(
        !value.chars().any(char::is_control),
        "{name} must not contain control characters"
    );
    Ok(())
}

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339()
}

pub(crate) fn fts_query(query: &str) -> Result<String> {
    let terms = query_terms(query)?;
    Ok(terms.join(" AND "))
}

/// Build a bounded fallback query for natural-language questions. Exact
/// all-term matching remains the primary path; callers may use this OR form
/// only when that precise query returns no memories.
pub(crate) fn fts_query_or(query: &str) -> Result<String> {
    Ok(query_terms(query)?.join(" OR "))
}

fn query_terms(query: &str) -> Result<Vec<String>> {
    anyhow::ensure!(
        query.len() <= MAX_MEMORY_CONTENT_BYTES,
        "memory query is too long"
    );
    let terms = query
        .split_whitespace()
        .map(|term| {
            term.chars()
                .filter(|character| character.is_alphanumeric() || *character == '_')
                .collect::<String>()
        })
        .filter(|term| !term.is_empty())
        .take(16)
        .map(|term| format!("\"{}\"", term.replace('"', "")))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !terms.is_empty(),
        "memory query must contain searchable terms"
    );
    Ok(terms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(valid_until: Option<&str>) -> MemoryInput {
        MemoryInput {
            kind: "working".into(),
            project: "work".into(),
            title: "Current task".into(),
            content: "Finish the release checklist.".into(),
            source: "agent".into(),
            source_id: String::new(),
            dedupe_key: None,
            confidence: 0.7,
            importance: 0.5,
            acl: vec![],
            provenance: serde_json::json!({"test":true}),
            supersedes_id: None,
            valid_until: valid_until.map(str::to_string),
        }
    }

    #[test]
    fn expiry_is_normalized_and_bounded() {
        let (_, _, _, valid_until) =
            validate_input(&input(Some("2030-01-01T12:00:00+02:00"))).expect("valid expiry");
        assert_eq!(valid_until.as_deref(), Some("2030-01-01T10:00:00Z"));
        let error = validate_input(&input(Some("tomorrow"))).expect_err("invalid expiry");
        assert!(error.to_string().contains("valid_until must be RFC3339"));
    }
}
