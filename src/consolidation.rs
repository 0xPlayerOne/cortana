//! Approval-aware promotion of bounded observations into native memory.
//!
//! Consolidation is deliberately a policy boundary, not an implicit part of
//! ingestion or retrieval.  Candidates remain reviewable until a versioned
//! policy explicitly permits a bounded action.  This module is provider-free
//! and deterministic so the same decision can be explained, retried, or
//! audited without retaining candidate content in telemetry.

use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    classification::{CandidateClassification, ClassificationKind},
    observation::{CandidateSensitivity, ObservationCandidate},
};

pub const CONSOLIDATION_POLICY_VERSION: &str = "cortana.memory.consolidation.v1";
pub const DEFAULT_MAX_QUEUE: usize = 1_000;
pub const DEFAULT_MAX_RETRIES: u8 = 3;
pub const DEFAULT_MAX_WORKING_DAYS: i64 = 7;
pub const DEFAULT_MAX_DURABLE_DAYS: i64 = 365;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RetentionCeilings {
    pub max_working_days: i64,
    pub max_durable_days: i64,
    pub max_active: usize,
}

impl Default for RetentionCeilings {
    fn default() -> Self {
        Self {
            max_working_days: DEFAULT_MAX_WORKING_DAYS,
            max_durable_days: DEFAULT_MAX_DURABLE_DAYS,
            max_active: crate::memory::DEFAULT_MEMORY_MAX_ACTIVE,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ConsolidationPolicy {
    pub version: String,
    pub enabled: bool,
    pub auto_retain_min_confidence: f32,
    pub auto_retain_min_importance: f32,
    pub max_queue: usize,
    pub max_retries: u8,
    pub retry_backoff_seconds: u64,
    pub ceilings: RetentionCeilings,
}

impl Default for ConsolidationPolicy {
    fn default() -> Self {
        Self {
            version: CONSOLIDATION_POLICY_VERSION.into(),
            enabled: false,
            auto_retain_min_confidence: 0.9,
            auto_retain_min_importance: 0.65,
            max_queue: DEFAULT_MAX_QUEUE,
            max_retries: DEFAULT_MAX_RETRIES,
            retry_backoff_seconds: 30,
            ceilings: RetentionCeilings::default(),
        }
    }
}

impl ConsolidationPolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version == CONSOLIDATION_POLICY_VERSION,
            "unsupported consolidation policy version"
        );
        anyhow::ensure!(
            self.auto_retain_min_confidence.is_finite()
                && (0.0..=1.0).contains(&self.auto_retain_min_confidence),
            "auto-retain confidence threshold must be between 0 and 1"
        );
        anyhow::ensure!(
            self.auto_retain_min_importance.is_finite()
                && (0.0..=1.0).contains(&self.auto_retain_min_importance),
            "auto-retain importance threshold must be between 0 and 1"
        );
        anyhow::ensure!(
            (1..=DEFAULT_MAX_QUEUE).contains(&self.max_queue),
            "consolidation queue is outside its bound"
        );
        anyhow::ensure!(
            self.max_retries <= 10,
            "consolidation retries are outside their bound"
        );
        anyhow::ensure!(
            self.ceilings.max_working_days > 0
                && self.ceilings.max_working_days <= DEFAULT_MAX_WORKING_DAYS,
            "working retention exceeds its ceiling"
        );
        anyhow::ensure!(
            self.ceilings.max_durable_days > 0 && self.ceilings.max_durable_days <= 3650,
            "durable retention exceeds its ceiling"
        );
        anyhow::ensure!(
            self.ceilings.max_active > 0 && self.ceilings.max_active <= 1_000_000,
            "active retention capacity is outside its bound"
        );
        Ok(())
    }

    /// Stable identity for the complete policy, not only its schema version.
    /// Threshold or ceiling changes therefore cannot reuse an earlier job.
    pub fn identity(&self) -> anyhow::Result<String> {
        self.validate()?;
        let digest = crate::contracts::stable_json_digest(self);
        Ok(format!("{}:{}", self.version, &digest[..16]))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConsolidationDecision {
    AutoRetain,
    Approve,
    Review,
    Reject,
    Working,
}

impl ConsolidationDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AutoRetain => "auto-retain",
            Self::Approve => "approve",
            Self::Review => "review",
            Self::Reject => "reject",
            Self::Working => "working",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PolicyContext {
    pub explicit_approval: bool,
    pub same_scope: bool,
    pub reviewer: Option<String>,
}

impl Default for PolicyContext {
    fn default() -> Self {
        Self {
            explicit_approval: false,
            same_scope: true,
            reviewer: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ConsolidationDecisionReport {
    pub candidate_id: String,
    pub policy_version: String,
    pub decision: ConsolidationDecision,
    pub classification: String,
    pub reason_code: String,
    pub explanation: String,
    pub queue_priority: u8,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ConsolidationOutcome {
    pub candidate_id: String,
    pub status: String,
    pub decision: ConsolidationDecisionReport,
    pub memory_id: Option<String>,
    pub attempts: u8,
}

/// Evaluate a candidate without mutating a store.  Sensitive, contradictory,
/// low-confidence, and cross-scope candidates are never auto-committed.
pub fn evaluate(
    candidate: &ObservationCandidate,
    classification: &CandidateClassification,
    policy: &ConsolidationPolicy,
    context: &PolicyContext,
) -> anyhow::Result<ConsolidationDecisionReport> {
    policy.validate()?;
    let now = Utc::now();
    let expires_at = DateTime::parse_from_rfc3339(&candidate.expires_at)
        .ok()
        .map(|value| value.with_timezone(&Utc));
    let mut decision = ConsolidationDecision::Review;
    let mut reason_code = "review-required";
    let mut explanation = "candidate requires explicit review before canonical retention";
    let mut priority = candidate
        .importance
        .mul_add(4.0, candidate.confidence * 4.0)
        .round() as u8;
    priority = priority.clamp(1, 8);

    if !policy.enabled {
        decision = ConsolidationDecision::Review;
        reason_code = "disabled";
        explanation =
            "consolidation is disabled; explicit memory writes and retrieval remain available";
    } else if candidate.status != "pending" {
        decision = ConsolidationDecision::Reject;
        reason_code = "candidate-not-pending";
        explanation = "only pending candidates can be consolidated";
    } else if expires_at.is_some_and(|value| value <= now) {
        decision = ConsolidationDecision::Reject;
        reason_code = "candidate-expired";
        explanation = "candidate expiry has elapsed";
    } else if candidate.sensitivity != CandidateSensitivity::Normal.as_str() {
        decision = ConsolidationDecision::Review;
        reason_code = "sensitive";
        explanation = "sensitive candidates require explicit user approval and cannot auto-commit";
        priority = 8;
    } else if !context.same_scope {
        decision = ConsolidationDecision::Review;
        reason_code = "cross-scope";
        explanation = "cross-scope candidates require an owner-approved review";
        priority = 8;
    } else if matches!(
        classification.classification.as_str(),
        "contradiction" | "supersession"
    ) {
        decision = ConsolidationDecision::Review;
        reason_code = "conflict";
        explanation = "contradictory or superseding candidates never auto-commit";
        priority = 8;
    } else if classification.classification == ClassificationKind::Discard.as_str() {
        decision = ConsolidationDecision::Reject;
        reason_code = "classification-discard";
        explanation = "classification marked the candidate as unsafe or below the retention floor";
    } else if candidate.confidence < policy.auto_retain_min_confidence {
        decision = if context.explicit_approval {
            ConsolidationDecision::Approve
        } else {
            ConsolidationDecision::Review
        };
        reason_code = "low-confidence";
        explanation = "confidence is below the automatic retention threshold";
    } else if candidate.retention_tier == "working" {
        decision = ConsolidationDecision::Working;
        reason_code = "working-retention";
        explanation = "working memory remains temporary and bounded even when approved";
    } else if matches!(
        classification.classification.as_str(),
        "new" | "reinforcement" | "semantic-duplicate" | "exact-duplicate"
    ) && candidate.importance >= policy.auto_retain_min_importance
    {
        decision = ConsolidationDecision::AutoRetain;
        reason_code = "thresholds-met";
        explanation = "candidate is non-sensitive, in-scope, non-conflicting, and meets automatic retention thresholds";
    } else if context.explicit_approval {
        decision = ConsolidationDecision::Approve;
        reason_code = "explicit-approval";
        explanation = "an authorized reviewer explicitly approved canonical retention";
    }

    let expiry = match decision {
        ConsolidationDecision::Working => {
            Some((now + Duration::days(policy.ceilings.max_working_days)).to_rfc3339())
        }
        ConsolidationDecision::AutoRetain | ConsolidationDecision::Approve => {
            Some((now + Duration::days(policy.ceilings.max_durable_days)).to_rfc3339())
        }
        _ => expires_at.map(|value| value.to_rfc3339()),
    };
    Ok(ConsolidationDecisionReport {
        candidate_id: candidate.id.clone(),
        policy_version: policy.identity()?,
        decision,
        classification: classification.classification.clone(),
        reason_code: reason_code.into(),
        explanation: explanation.into(),
        queue_priority: priority,
        expires_at: expiry,
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QueueStatus {
    Queued,
    Running,
    Retry,
    DeadLetter,
    Paused,
    Cancelled,
    Complete,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct QueueItem {
    pub candidate_id: String,
    pub policy_version: String,
    pub priority: u8,
    pub attempts: u8,
    pub status: QueueStatus,
}

/// A bounded deterministic queue useful to schedule consolidation without
/// coupling the policy to a runtime worker. Persistence is supplied by Store.
#[derive(Clone, Debug)]
pub struct ConsolidationQueue {
    max_items: usize,
    max_retries: u8,
    paused: bool,
    items: VecDeque<QueueItem>,
    seen: HashMap<String, QueueStatus>,
}

impl ConsolidationQueue {
    pub fn new(policy: &ConsolidationPolicy) -> anyhow::Result<Self> {
        policy.validate()?;
        Ok(Self {
            max_items: policy.max_queue,
            max_retries: policy.max_retries,
            paused: false,
            items: VecDeque::new(),
            seen: HashMap::new(),
        })
    }

    pub fn enqueue(
        &mut self,
        candidate_id: &str,
        policy_version: &str,
        priority: u8,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.paused, "consolidation queue is paused");
        let key = format!("{policy_version}:{candidate_id}");
        if self.seen.get(&key).is_some_and(|status| {
            matches!(
                status,
                QueueStatus::Queued | QueueStatus::Running | QueueStatus::Complete
            )
        }) {
            return Ok(());
        }
        anyhow::ensure!(self.len() < self.max_items, "consolidation queue is full");
        let item = QueueItem {
            candidate_id: candidate_id.into(),
            policy_version: policy_version.into(),
            priority: priority.clamp(1, 8),
            attempts: 0,
            status: QueueStatus::Queued,
        };
        self.seen.insert(key, QueueStatus::Queued);
        self.items.push_back(item);
        self.items.make_contiguous().sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        Ok(())
    }

    pub fn pause(&mut self) {
        self.paused = true;
        for item in &mut self.items {
            if matches!(item.status, QueueStatus::Queued | QueueStatus::Retry) {
                item.status = QueueStatus::Paused;
                self.seen.insert(
                    format!("{}:{}", item.policy_version, item.candidate_id),
                    QueueStatus::Paused,
                );
            }
        }
    }
    pub fn resume(&mut self) {
        for item in &mut self.items {
            if item.status == QueueStatus::Paused {
                item.status = QueueStatus::Queued;
                self.seen.insert(
                    format!("{}:{}", item.policy_version, item.candidate_id),
                    QueueStatus::Queued,
                );
            }
        }
        self.paused = false;
    }
    pub fn cancel(&mut self, candidate_id: &str) -> bool {
        if let Some(item) = self.items.iter_mut().find(|item| {
            item.candidate_id == candidate_id
                && matches!(
                    item.status,
                    QueueStatus::Queued | QueueStatus::Retry | QueueStatus::Paused
                )
        }) {
            item.status = QueueStatus::Cancelled;
            self.seen.insert(
                format!("{}:{candidate_id}", item.policy_version),
                QueueStatus::Cancelled,
            );
            return true;
        }
        false
    }
    pub fn pop(&mut self) -> Option<QueueItem> {
        if self.paused {
            return None;
        }
        while let Some(mut item) = self.items.pop_front() {
            if item.status == QueueStatus::Cancelled {
                continue;
            }
            item.status = QueueStatus::Running;
            item.attempts = item.attempts.saturating_add(1);
            return Some(item);
        }
        None
    }
    pub fn retry(&mut self, mut item: QueueItem) {
        item.status = if item.attempts > self.max_retries {
            QueueStatus::DeadLetter
        } else {
            QueueStatus::Retry
        };
        self.seen.insert(
            format!("{}:{}", item.policy_version, item.candidate_id),
            item.status,
        );
        if item.status == QueueStatus::Retry {
            self.items.push_back(item);
            self.items.make_contiguous().sort_by(|left, right| {
                right
                    .priority
                    .cmp(&left.priority)
                    .then_with(|| left.candidate_id.cmp(&right.candidate_id))
            });
        }
    }
    pub fn len(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status != QueueStatus::Cancelled)
            .count()
    }
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|item| {
            matches!(
                item.status,
                QueueStatus::Cancelled | QueueStatus::DeadLetter
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate() -> ObservationCandidate {
        ObservationCandidate {
            id: "candidate-1".into(),
            created_by: "agent-a".into(),
            observation_kind: "evidence-backed".into(),
            content_type: "semantic".into(),
            retention_tier: "durable".into(),
            scope: "workspace".into(),
            project: "work".into(),
            title: "fact".into(),
            content: "useful fact".into(),
            source: "test".into(),
            source_id: "1".into(),
            dedupe_key: Some("d1".into()),
            confidence: 0.95,
            importance: 0.9,
            sensitivity: "normal".into(),
            status: "pending".into(),
            acl: vec!["work".into()],
            provenance: serde_json::json!({"source":"test"}),
            expires_at: (Utc::now() + Duration::hours(1)).to_rfc3339(),
            rejection_reason: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }
    fn classification(kind: &str) -> CandidateClassification {
        CandidateClassification {
            candidate_id: "candidate-1".into(),
            classification: kind.into(),
            supporting_memory_ids: vec![],
            explanation: "test".into(),
            confidence: 0.95,
            proposed_action: "retain-for-review".into(),
            unresolved_ambiguity: None,
            compared_memory_count: 0,
        }
    }

    #[test]
    fn safe_high_confidence_candidate_auto_retains_only_when_enabled() {
        let mut policy = ConsolidationPolicy {
            enabled: true,
            ..Default::default()
        };
        let result = evaluate(
            &candidate(),
            &classification("new"),
            &policy,
            &PolicyContext::default(),
        )
        .unwrap();
        assert_eq!(result.decision, ConsolidationDecision::AutoRetain);
        policy.enabled = false;
        assert_eq!(
            evaluate(
                &candidate(),
                &classification("new"),
                &policy,
                &PolicyContext::default()
            )
            .unwrap()
            .decision,
            ConsolidationDecision::Review
        );
    }

    #[test]
    fn sensitive_conflicting_and_cross_scope_never_auto_commit() {
        let policy = ConsolidationPolicy {
            enabled: true,
            ..Default::default()
        };
        let mut sensitive = candidate();
        sensitive.sensitivity = "sensitive".into();
        assert_eq!(
            evaluate(
                &sensitive,
                &classification("new"),
                &policy,
                &PolicyContext::default()
            )
            .unwrap()
            .decision,
            ConsolidationDecision::Review
        );
        assert_eq!(
            evaluate(
                &candidate(),
                &classification("contradiction"),
                &policy,
                &PolicyContext::default()
            )
            .unwrap()
            .decision,
            ConsolidationDecision::Review
        );
        assert_eq!(
            evaluate(
                &candidate(),
                &classification("new"),
                &policy,
                &PolicyContext {
                    same_scope: false,
                    ..Default::default()
                }
            )
            .unwrap()
            .decision,
            ConsolidationDecision::Review
        );
    }

    #[test]
    fn bounded_queue_prioritizes_and_dead_letters_after_retries() {
        let policy = ConsolidationPolicy {
            enabled: true,
            max_retries: 1,
            ..Default::default()
        };
        let mut queue = ConsolidationQueue::new(&policy).unwrap();
        queue.enqueue("low", &policy.version, 1).unwrap();
        queue.enqueue("high", &policy.version, 8).unwrap();
        let high = queue.pop().unwrap();
        assert_eq!(high.candidate_id, "high");
        queue.retry(high.clone());
        let retry = queue.pop().unwrap();
        assert_eq!(retry.attempts, 2);
        queue.retry(retry);
        assert!(queue.cancel("low"));
        assert!(queue.is_empty());
    }

    #[test]
    fn policy_identity_covers_thresholds_and_queue_dedupes_before_capacity() {
        let policy = ConsolidationPolicy {
            enabled: true,
            max_queue: 1,
            ..Default::default()
        };
        let mut changed = policy.clone();
        changed.auto_retain_min_importance = 0.8;
        assert_ne!(policy.identity().unwrap(), changed.identity().unwrap());

        let mut queue = ConsolidationQueue::new(&policy).unwrap();
        let identity = policy.identity().unwrap();
        queue.enqueue("candidate", &identity, 1).unwrap();
        queue.enqueue("candidate", &identity, 8).unwrap();
        assert_eq!(queue.len(), 1);
        queue.pause();
        assert!(queue.pop().is_none());
        queue.resume();
        assert_eq!(queue.pop().unwrap().candidate_id, "candidate");
    }
}
