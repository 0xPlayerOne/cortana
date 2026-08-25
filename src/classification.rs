//! Deterministic, review-only classification for bounded memory observations.
//!
//! Deterministic classification is always available without a provider. An
//! optional provider adapter may refine a result, but invalid or failed model
//! output degrades to a bounded review-required deterministic outcome.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{memory::MemoryRecord, observation::ObservationCandidate};

const SEMANTIC_DUPLICATE_THRESHOLD: f64 = 0.72;
const REINFORCEMENT_THRESHOLD: f64 = 0.45;
const MAX_EXPLANATION_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ClassificationKind {
    New,
    ExactDuplicate,
    SemanticDuplicate,
    Reinforcement,
    Contradiction,
    Supersession,
    TemporaryWorking,
    Discard,
}

impl ClassificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::ExactDuplicate => "exact-duplicate",
            Self::SemanticDuplicate => "semantic-duplicate",
            Self::Reinforcement => "reinforcement",
            Self::Contradiction => "contradiction",
            Self::Supersession => "supersession",
            Self::TemporaryWorking => "temporary-working",
            Self::Discard => "discard",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProposedAction {
    RetainForReview,
    MergeWithExisting,
    ReinforceExisting,
    ReplaceExistingAfterReview,
    KeepTemporary,
    Discard,
}

impl ProposedAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::RetainForReview => "retain-for-review",
            Self::MergeWithExisting => "merge-with-existing",
            Self::ReinforceExisting => "reinforce-existing",
            Self::ReplaceExistingAfterReview => "replace-existing-after-review",
            Self::KeepTemporary => "keep-temporary",
            Self::Discard => "discard",
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CandidateClassification {
    pub candidate_id: String,
    pub classification: String,
    pub supporting_memory_ids: Vec<String>,
    pub explanation: String,
    pub confidence: f32,
    pub proposed_action: String,
    pub unresolved_ambiguity: Option<String>,
    pub compared_memory_count: usize,
}

pub trait ClassificationProvider: Send + Sync {
    fn classify(
        &self,
        candidate: &ObservationCandidate,
        memories: &[MemoryRecord],
    ) -> std::result::Result<CandidateClassification, String>;
}

pub fn classify_with_optional_provider(
    candidate: &ObservationCandidate,
    memories: &[MemoryRecord],
    provider: Option<&dyn ClassificationProvider>,
) -> CandidateClassification {
    let deterministic = classify(candidate, memories);
    let Some(provider) = provider else {
        return deterministic;
    };
    match provider.classify(candidate, memories) {
        Ok(result) if provider_result_is_valid(candidate, memories, &result) => result,
        Ok(_) | Err(_) => review_required_provider_fallback(deterministic),
    }
}

fn provider_result_is_valid(
    candidate: &ObservationCandidate,
    memories: &[MemoryRecord],
    result: &CandidateClassification,
) -> bool {
    const KINDS: &[&str] = &[
        "new",
        "exact-duplicate",
        "semantic-duplicate",
        "reinforcement",
        "contradiction",
        "supersession",
        "temporary-working",
        "discard",
    ];
    const ACTIONS: &[&str] = &[
        "retain-for-review",
        "merge-with-existing",
        "reinforce-existing",
        "replace-existing-after-review",
        "keep-temporary",
        "discard",
    ];
    let visible_ids = memories
        .iter()
        .filter(|memory| memory_is_active(memory))
        .filter(|memory| memory.project == candidate.project && memory.acl == candidate.acl)
        .map(|memory| memory.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    result.candidate_id == candidate.id
        && KINDS.contains(&result.classification.as_str())
        && ACTIONS.contains(&result.proposed_action.as_str())
        && result.confidence.is_finite()
        && (0.0..=1.0).contains(&result.confidence)
        && result.explanation.len() <= MAX_EXPLANATION_BYTES
        && result
            .unresolved_ambiguity
            .as_ref()
            .is_none_or(|value| value.len() <= MAX_EXPLANATION_BYTES)
        && result.supporting_memory_ids.len() <= memories.len().min(32)
        && result
            .supporting_memory_ids
            .iter()
            .all(|id| visible_ids.contains(id.as_str()))
}

fn review_required_provider_fallback(
    mut deterministic: CandidateClassification,
) -> CandidateClassification {
    deterministic.proposed_action = ProposedAction::RetainForReview.as_str().into();
    deterministic.unresolved_ambiguity = Some(
        "model-assisted comparison unavailable or invalid; deterministic review required".into(),
    );
    deterministic
}

#[derive(Clone, Debug)]
struct Match<'a> {
    memory: &'a MemoryRecord,
    similarity: f64,
    exact: bool,
    contradiction: bool,
    supersession: bool,
}

/// Classify one candidate against a bounded, already ACL/project/axis-scoped
/// set of canonical records.  The caller owns the visibility boundary.
pub fn classify(
    candidate: &ObservationCandidate,
    memories: &[MemoryRecord],
) -> CandidateClassification {
    let eligible_memories = memories
        .iter()
        .filter(|memory| memory_is_active(memory))
        .filter(|memory| memory.project == candidate.project)
        .filter(|memory| memory.acl == candidate.acl)
        .filter(|memory| memory.content_type == candidate.content_type)
        .filter(|memory| memory.retention_tier == candidate.retention_tier)
        .filter(|memory| memory.scope == candidate.scope)
        .collect::<Vec<_>>();
    let compared_memory_count = eligible_memories.len();
    if candidate.status != "pending" {
        return report(
            candidate,
            ClassificationKind::Discard,
            Vec::new(),
            "candidate is no longer pending and cannot be promoted",
            1.0,
            ProposedAction::Discard,
            None,
            compared_memory_count,
        );
    }
    if candidate.confidence < 0.2 || candidate.content.trim().len() < 3 {
        return report(
            candidate,
            ClassificationKind::Discard,
            Vec::new(),
            "candidate confidence or content is below the bounded retention floor",
            0.98,
            ProposedAction::Discard,
            None,
            compared_memory_count,
        );
    }

    let mut matches: Vec<Match<'_>> = eligible_memories
        .into_iter()
        .filter_map(|memory| {
            let candidate_text = normalized_text(&candidate.title, &candidate.content);
            let memory_text = normalized_text(&memory.title, &memory.content);
            let exact = candidate_text == memory_text;
            let similarity = token_similarity(&candidate_text, &memory_text);
            let contradiction = polarity_conflict(&candidate_text, &memory_text)
                || preference_value_conflict(candidate, memory, similarity);
            let supersession = contradiction
                && (has_change_marker(&candidate_text) || has_change_marker(&memory_text));
            (exact || similarity >= REINFORCEMENT_THRESHOLD || contradiction).then_some(Match {
                memory,
                similarity,
                exact,
                contradiction,
                supersession,
            })
        })
        .collect();
    matches.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.memory.id.cmp(&right.memory.id))
    });

    let Some(best) = matches.first() else {
        if candidate.retention_tier == "working" {
            return report(
                candidate,
                ClassificationKind::TemporaryWorking,
                Vec::new(),
                "no visible canonical match; retain only within the candidate working window",
                0.9,
                ProposedAction::KeepTemporary,
                None,
                compared_memory_count,
            );
        }
        return report(
            candidate,
            ClassificationKind::New,
            Vec::new(),
            "no visible canonical memory matched within the same project, ACL, and memory axes",
            0.9,
            ProposedAction::RetainForReview,
            None,
            compared_memory_count,
        );
    };

    let supporting = vec![best.memory.id.clone()];
    if best.exact {
        return report(
            candidate,
            ClassificationKind::ExactDuplicate,
            supporting,
            "normalized title and content exactly match visible canonical memory",
            0.99,
            ProposedAction::MergeWithExisting,
            None,
            compared_memory_count,
        );
    }
    if best.supersession {
        return report(
            candidate,
            ClassificationKind::Supersession,
            supporting,
            "candidate conflicts with a visible memory and contains a change marker; replacement requires review",
            (0.65 + best.similarity * 0.3) as f32,
            ProposedAction::ReplaceExistingAfterReview,
            Some("conflicting decisions are never auto-committed"),
            compared_memory_count,
        );
    }
    if best.contradiction {
        return report(
            candidate,
            ClassificationKind::Contradiction,
            supporting,
            "candidate conflicts with visible canonical memory; retain for explicit review",
            (0.55 + best.similarity * 0.35) as f32,
            ProposedAction::RetainForReview,
            Some("polarity or preference values conflict"),
            compared_memory_count,
        );
    }
    if best.similarity >= SEMANTIC_DUPLICATE_THRESHOLD {
        return report(
            candidate,
            ClassificationKind::SemanticDuplicate,
            supporting,
            "high deterministic token similarity indicates a paraphrase of visible canonical memory",
            (0.7 + best.similarity * 0.25) as f32,
            ProposedAction::MergeWithExisting,
            None,
            compared_memory_count,
        );
    }
    if candidate.retention_tier == "working" {
        return report(
            candidate,
            ClassificationKind::TemporaryWorking,
            supporting,
            "working candidate is related to visible memory but remains temporary and review-only",
            0.7,
            ProposedAction::KeepTemporary,
            None,
            compared_memory_count,
        );
    }
    report(
        candidate,
        ClassificationKind::Reinforcement,
        supporting,
        "candidate provides related evidence without an exact or semantic duplicate",
        (0.55 + best.similarity * 0.35) as f32,
        ProposedAction::ReinforceExisting,
        None,
        compared_memory_count,
    )
}

fn memory_is_stale(memory: &MemoryRecord) -> bool {
    memory.valid_until.as_deref().is_some_and(|valid_until| {
        DateTime::parse_from_rfc3339(valid_until)
            .map(|value| value.with_timezone(&Utc) <= Utc::now())
            .unwrap_or(false)
    })
}

fn memory_is_active(memory: &MemoryRecord) -> bool {
    if memory.status != "active" || memory_is_stale(memory) {
        return false;
    }
    DateTime::parse_from_rfc3339(&memory.valid_from)
        .map(|value| value.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn report(
    candidate: &ObservationCandidate,
    classification: ClassificationKind,
    supporting_memory_ids: Vec<String>,
    explanation: &str,
    confidence: f32,
    proposed_action: ProposedAction,
    unresolved_ambiguity: Option<&str>,
    compared_memory_count: usize,
) -> CandidateClassification {
    let explanation = explanation.chars().take(MAX_EXPLANATION_BYTES).collect();
    CandidateClassification {
        candidate_id: candidate.id.clone(),
        classification: classification.as_str().into(),
        supporting_memory_ids,
        explanation,
        confidence: confidence.clamp(0.0, 1.0),
        proposed_action: proposed_action.as_str().into(),
        unresolved_ambiguity: unresolved_ambiguity.map(str::to_owned),
        compared_memory_count,
    }
}

fn normalized_text(title: &str, content: &str) -> String {
    format!("{title} {content}")
        .to_ascii_lowercase()
        .split_whitespace()
        .map(|word| word.trim_matches(|character: char| !character.is_ascii_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn token_similarity(left: &str, right: &str) -> f64 {
    let left: std::collections::HashSet<&str> = left.split_whitespace().collect();
    let right: std::collections::HashSet<&str> = right.split_whitespace().collect();
    let shared = left.intersection(&right).count();
    let total = left.len() + right.len();
    if total == 0 {
        0.0
    } else {
        // Dice overlap is less sensitive to harmless stop-word/order changes
        // than Jaccard while remaining deterministic and provider-free.
        (2 * shared) as f64 / total as f64
    }
}

fn polarity_conflict(left: &str, right: &str) -> bool {
    let left_negative = has_negation(left);
    let right_negative = has_negation(right);
    left_negative != right_negative && token_similarity(left, right) >= 0.35
}

fn has_negation(value: &str) -> bool {
    value.split_whitespace().any(|word| {
        matches!(
            word,
            "no" | "not" | "never" | "dont" | "don't" | "cannot" | "can't"
        )
    })
}

fn has_change_marker(value: &str) -> bool {
    value.split_whitespace().any(|word| {
        matches!(
            word,
            "changed"
                | "change"
                | "now"
                | "instead"
                | "replace"
                | "replaced"
                | "supersede"
                | "supersedes"
                | "updated"
        )
    })
}

fn preference_value_conflict(
    candidate: &ObservationCandidate,
    memory: &MemoryRecord,
    similarity: f64,
) -> bool {
    let candidate_text = normalized_text(&candidate.title, &candidate.content);
    let memory_text = normalized_text(&memory.title, &memory.content);
    candidate.content_type == "preference"
        && similarity >= 0.35
        && (has_change_marker(&candidate_text) || has_change_marker(&memory_text))
        && candidate_text != memory_text
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;

    fn candidate(content: &str, content_type: &str, retention_tier: &str) -> ObservationCandidate {
        ObservationCandidate {
            id: "candidate-1".into(),
            observation_kind: "user-authored".into(),
            content_type: content_type.into(),
            retention_tier: retention_tier.into(),
            scope: "workspace".into(),
            created_by: "agent-a".into(),
            project: "work".into(),
            title: "Decision".into(),
            content: content.into(),
            source: "test".into(),
            source_id: "source-1".into(),
            dedupe_key: None,
            confidence: 0.9,
            importance: 0.6,
            sensitivity: "normal".into(),
            status: "pending".into(),
            acl: vec!["work".into()],
            provenance: json!({}),
            expires_at: (Utc::now() + Duration::hours(1)).to_rfc3339(),
            rejection_reason: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn memory(content: &str, content_type: &str, retention_tier: &str) -> MemoryRecord {
        MemoryRecord {
            id: "memory-1".into(),
            kind: content_type.into(),
            content_type: content_type.into(),
            retention_tier: retention_tier.into(),
            scope: "workspace".into(),
            project: "work".into(),
            title: "Decision".into(),
            content: content.into(),
            source: "test".into(),
            source_id: "memory-source".into(),
            dedupe_key: None,
            confidence: 0.8,
            importance: 0.5,
            status: "active".into(),
            acl: vec!["work".into()],
            provenance: json!({}),
            observed_at: Utc::now().to_rfc3339(),
            valid_from: Utc::now().to_rfc3339(),
            valid_until: None,
            supersedes_id: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn classifies_new_and_paraphrase_without_provider() {
        let result = classify(&candidate("Deploy on Friday", "semantic", "durable"), &[]);
        assert_eq!(result.classification, "new");
        let result = classify(
            &candidate("Deploy Friday instead", "semantic", "durable"),
            &[memory("Deploy on Friday", "semantic", "durable")],
        );
        assert_eq!(result.classification, "semantic-duplicate");
    }

    #[test]
    fn classifies_conflict_and_changed_decision_as_review_only() {
        let mut candidate = candidate("Changed: deploy on Monday instead", "preference", "durable");
        let old = memory("Deploy on Friday", "preference", "durable");
        assert_eq!(classify(&candidate, &[old]).classification, "supersession");
        candidate.content = "Do not deploy on Friday".into();
        assert_eq!(
            classify(
                &candidate,
                &[memory("Deploy on Friday", "preference", "durable")]
            )
            .classification,
            "contradiction"
        );
    }

    #[test]
    fn preference_paraphrases_are_not_contradictions() {
        let candidate = candidate("Prefer concise release notes", "preference", "durable");
        let existing = memory("Prefer short release notes", "preference", "durable");
        let result = classify(&candidate, &[existing]);
        assert_ne!(result.classification, "contradiction");
        assert_ne!(result.classification, "supersession");
    }

    #[test]
    fn retired_expired_future_and_disjoint_acl_memories_are_not_compared() {
        let candidate = candidate("Deploy on Friday", "semantic", "durable");
        let mut retired = memory("Deploy on Friday", "semantic", "durable");
        retired.status = "superseded".into();
        let mut expired = memory("Deploy on Friday", "semantic", "durable");
        expired.id = "expired".into();
        expired.valid_until = Some((Utc::now() - Duration::hours(1)).to_rfc3339());
        let mut future = memory("Deploy on Friday", "semantic", "durable");
        future.id = "future".into();
        future.valid_from = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let mut disjoint = memory("Deploy on Friday", "semantic", "durable");
        disjoint.id = "disjoint".into();
        disjoint.acl = vec!["private".into()];
        let result = classify(&candidate, &[retired, expired, future, disjoint]);
        assert_eq!(result.classification, "new");
        assert_eq!(result.compared_memory_count, 0);
    }

    struct FailingProvider;

    impl ClassificationProvider for FailingProvider {
        fn classify(
            &self,
            _candidate: &ObservationCandidate,
            _memories: &[MemoryRecord],
        ) -> std::result::Result<CandidateClassification, String> {
            Err("provider unavailable".into())
        }
    }

    #[test]
    fn optional_provider_failure_is_bounded_and_review_required() {
        let result = classify_with_optional_provider(
            &candidate("Deploy Friday", "semantic", "durable"),
            &[],
            Some(&FailingProvider),
        );
        assert_eq!(result.proposed_action, "retain-for-review");
        let ambiguity = result.unresolved_ambiguity.expect("review reason");
        assert!(ambiguity.len() <= MAX_EXPLANATION_BYTES);
        assert!(!ambiguity.contains("provider unavailable"));
    }

    #[test]
    fn working_candidates_stay_temporary_and_low_quality_is_discarded() {
        let result = classify(
            &candidate("Short-lived context", "semantic", "working"),
            &[],
        );
        assert_eq!(result.classification, "temporary-working");
        let mut stale = memory("Short-lived context", "semantic", "working");
        stale.valid_until = Some("2000-01-01T00:00:00Z".into());
        let result = classify(
            &candidate("Short-lived context", "semantic", "working"),
            &[stale],
        );
        assert_eq!(result.classification, "temporary-working");
        assert_eq!(result.compared_memory_count, 0);
        assert!(result.supporting_memory_ids.is_empty());
        let mut low = candidate("x", "semantic", "durable");
        low.confidence = 0.1;
        assert_eq!(classify(&low, &[]).classification, "discard");
    }

    #[test]
    fn scoped_duplicates_are_not_compared_across_axes() {
        let result = classify(
            &candidate("Same fact", "semantic", "durable"),
            &[
                memory("Same fact", "preference", "durable"),
                memory("Same fact", "semantic", "working"),
            ],
        );
        assert_eq!(result.classification, "new");
        assert!(result.supporting_memory_ids.is_empty());
    }
}
