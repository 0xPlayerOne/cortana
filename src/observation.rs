//! Bounded, reviewable observations proposed for native memory.
//!
//! Candidates are deliberately separate from `memories`: accepting an
//! observation never makes it recallable and never advances `memory_revision`.
//! A later review/promotion step may turn an approved candidate into an
//! explicit memory.

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::memory::{MemoryContentType, MemoryRetentionTier, MemoryScope, normalize_acl};

pub const MAX_CANDIDATE_TITLE_BYTES: usize = 256;
pub const MAX_CANDIDATE_CONTENT_BYTES: usize = 8 * 1024;
pub const MAX_CANDIDATE_SOURCE_BYTES: usize = 128;
pub const MAX_CANDIDATE_SOURCE_ID_BYTES: usize = 512;
pub const MAX_CANDIDATE_DEDUPE_KEY_BYTES: usize = 512;
pub const MAX_CANDIDATE_PROVENANCE_BYTES: usize = 4 * 1024;
pub const MAX_CANDIDATES_PER_PROJECT: usize = 1_000;
pub const MAX_CANDIDATE_EXPORT_LIMIT: usize = 10_000;
pub const MAX_CANDIDATE_TTL: Duration = Duration::days(7);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ObservationKind {
    HarnessScratchpad,
    ExecutionEvent,
    EvidenceBacked,
    UserAuthored,
}

impl ObservationKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "harness-scratchpad" | "scratchpad" => Ok(Self::HarnessScratchpad),
            "execution-event" | "execution" => Ok(Self::ExecutionEvent),
            "evidence-backed" | "evidence" => Ok(Self::EvidenceBacked),
            "user-authored" | "user" => Ok(Self::UserAuthored),
            other => bail!(
                "unsupported observation kind `{other}`; expected harness-scratchpad, execution-event, evidence-backed, or user-authored"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::HarnessScratchpad => "harness-scratchpad",
            Self::ExecutionEvent => "execution-event",
            Self::EvidenceBacked => "evidence-backed",
            Self::UserAuthored => "user-authored",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CandidateSensitivity {
    Normal,
    Sensitive,
    Restricted,
}

impl CandidateSensitivity {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "normal" | "standard" => Ok(Self::Normal),
            "sensitive" => Ok(Self::Sensitive),
            "restricted" => Ok(Self::Restricted),
            other => bail!(
                "unsupported candidate sensitivity `{other}`; expected normal, sensitive, or restricted"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Sensitive => "sensitive",
            Self::Restricted => "restricted",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObservationCandidateInput {
    pub observation_kind: String,
    pub content_type: String,
    pub retention_tier: String,
    pub scope: String,
    pub project: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_id: String,
    pub dedupe_key: Option<String>,
    pub confidence: f32,
    pub importance: f32,
    pub sensitivity: String,
    pub acl: Vec<String>,
    pub provenance: Value,
    pub expires_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObservationCandidate {
    pub id: String,
    pub observation_kind: String,
    pub content_type: String,
    pub retention_tier: String,
    pub scope: String,
    pub project: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_id: String,
    pub dedupe_key: Option<String>,
    pub confidence: f32,
    pub importance: f32,
    pub sensitivity: String,
    pub status: String,
    pub acl: Vec<String>,
    pub provenance: Value,
    pub expires_at: String,
    pub rejection_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct ValidatedCandidate {
    pub observation_kind: ObservationKind,
    pub content_type: MemoryContentType,
    pub retention_tier: MemoryRetentionTier,
    pub scope: MemoryScope,
    pub sensitivity: CandidateSensitivity,
    pub acl: Vec<String>,
    pub provenance: Value,
    pub expires_at: String,
}

pub fn validate_input(input: &ObservationCandidateInput) -> Result<ValidatedCandidate> {
    let observation_kind = ObservationKind::parse(&input.observation_kind)?;
    let content_type = MemoryContentType::parse(&input.content_type)?;
    let retention_tier = MemoryRetentionTier::parse(&input.retention_tier)?;
    let scope = MemoryScope::parse(&input.scope)?;
    let sensitivity = CandidateSensitivity::parse(&input.sensitivity)?;
    validate_text("project", &input.project, 256)?;
    validate_text("title", &input.title, MAX_CANDIDATE_TITLE_BYTES)?;
    validate_text("content", &input.content, MAX_CANDIDATE_CONTENT_BYTES)?;
    validate_text("source", &input.source, MAX_CANDIDATE_SOURCE_BYTES)?;
    validate_text("source_id", &input.source_id, MAX_CANDIDATE_SOURCE_ID_BYTES)?;
    if let Some(key) = &input.dedupe_key {
        validate_text("dedupe_key", key, MAX_CANDIDATE_DEDUPE_KEY_BYTES)?;
    }
    anyhow::ensure!(
        input.confidence.is_finite() && (0.0..=1.0).contains(&input.confidence),
        "confidence must be a finite number between 0 and 1"
    );
    anyhow::ensure!(
        input.importance.is_finite() && (0.0..=1.0).contains(&input.importance),
        "importance must be a finite number between 0 and 1"
    );
    anyhow::ensure!(
        input.provenance.is_object(),
        "candidate provenance must be a JSON object"
    );
    let provenance_json = serde_json::to_string(&input.provenance)?;
    anyhow::ensure!(
        provenance_json.len() <= MAX_CANDIDATE_PROVENANCE_BYTES,
        "candidate provenance exceeds {MAX_CANDIDATE_PROVENANCE_BYTES} bytes"
    );
    let acl = normalize_acl(&input.project, &input.acl)?;
    let expires_at = DateTime::parse_from_rfc3339(&input.expires_at)
        .map_err(|error| anyhow::anyhow!("expires_at must be RFC3339: {error}"))?
        .with_timezone(&Utc);
    let now = Utc::now();
    anyhow::ensure!(expires_at > now, "expires_at must be in the future");
    anyhow::ensure!(
        expires_at <= now + MAX_CANDIDATE_TTL,
        "candidate expiry cannot exceed 7 days"
    );
    Ok(ValidatedCandidate {
        observation_kind,
        content_type,
        retention_tier,
        scope,
        sensitivity,
        acl,
        provenance: input.provenance.clone(),
        expires_at: expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ObservationCandidateInput {
        ObservationCandidateInput {
            observation_kind: "evidence-backed".into(),
            content_type: "semantic".into(),
            retention_tier: "working".into(),
            scope: "workspace".into(),
            project: "work".into(),
            title: "bounded observation".into(),
            content: "A short proposal".into(),
            source: "mcp".into(),
            source_id: "source-1".into(),
            dedupe_key: Some("retry-1".into()),
            confidence: 0.8,
            importance: 0.4,
            sensitivity: "normal".into(),
            acl: vec!["work".into()],
            provenance: serde_json::json!({"citation":"doc-1"}),
            expires_at: (Utc::now() + Duration::hours(1)).to_rfc3339(),
        }
    }

    #[test]
    fn rejects_unbounded_or_sensitive_observations() {
        let mut candidate = input();
        candidate.content = "x".repeat(MAX_CANDIDATE_CONTENT_BYTES + 1);
        assert!(validate_input(&candidate).is_err());
        let mut candidate = input();
        candidate.sensitivity = "restricted".into();
        assert_eq!(
            validate_input(&candidate).unwrap().sensitivity,
            CandidateSensitivity::Restricted
        );
    }

    #[test]
    fn canonicalizes_supported_aliases() {
        let mut candidate = input();
        candidate.observation_kind = "evidence".into();
        let result = validate_input(&candidate).expect("candidate");
        assert_eq!(result.observation_kind, ObservationKind::EvidenceBacked);
        assert_eq!(result.acl, vec!["work"]);
    }
}
