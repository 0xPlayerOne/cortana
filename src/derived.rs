//! Bounded, provenance-bearing projections over canonical memory.
//!
//! Derived representations are computed from the currently authorized memory
//! page. They are never written to `memories`, never become citation
//! authority, and carry the exact memory revision and support IDs that made
//! them valid. A lifecycle change advances `memory_revision`; the next read
//! therefore recomputes the projection without stale materialized state.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::contracts::stable_json_digest;
use crate::{memory::MemoryRecord, store::Store};

pub const DERIVED_MEMORY_CONTRACT_VERSION: &str = "cortana.memory-derived.v1";
pub const DERIVATION_ENGINE_VERSION: &str = "native-derived-v1";
pub const MAX_DERIVED_INPUTS: usize = 100;
pub const MAX_DERIVED_REPRESENTATIONS: usize = 64;
pub const MAX_DERIVED_RELATIONS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum DerivedKind {
    Experience,
    Observation,
    MentalModel,
    Belief,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RelationKind {
    SubjectPredicateObject,
    Temporal,
    Causal,
    Reinforcement,
    Contradiction,
    Supersession,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DerivationProvenance {
    pub engine_version: String,
    pub input_revision: u64,
    pub support_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DerivedRepresentation {
    pub id: String,
    pub contract_version: String,
    pub kind: DerivedKind,
    pub project: String,
    pub scope: String,
    pub statement: String,
    pub confidence: f32,
    pub supporting_memory_ids: Vec<String>,
    pub contradicting_memory_ids: Vec<String>,
    /// Exact intersection of supporting record ACLs; a join with no common
    /// ACL label is omitted rather than widened.
    pub acl: Vec<String>,
    pub memory_revision: u64,
    pub freshness: String,
    pub citation_authority: bool,
    pub provenance: DerivationProvenance,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct MemoryRelation {
    pub id: String,
    pub contract_version: String,
    pub kind: RelationKind,
    pub project: String,
    pub scope: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    pub supporting_memory_ids: Vec<String>,
    pub acl: Vec<String>,
    pub memory_revision: u64,
    pub freshness: String,
    pub citation_authority: bool,
    pub provenance: DerivationProvenance,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DerivedMemoryResponse {
    pub contract_version: String,
    pub derivation_version: String,
    pub memory_revision: u64,
    pub canonical_memory_mutated: bool,
    pub recomputed: bool,
    pub inputs_considered: usize,
    pub representations: Vec<DerivedRepresentation>,
    pub relations: Vec<MemoryRelation>,
}

/// Export an authorized memory page and derive it against a stable revision.
/// Retry one concurrent lifecycle write, then fail closed instead of attaching
/// a newer revision to older support records.
pub fn derive_authorized_memory(
    store: &Store,
    project: Option<&str>,
    limit: usize,
    principal_acl: &[String],
    owner: bool,
) -> Result<DerivedMemoryResponse> {
    let limit = limit.clamp(1, MAX_DERIVED_INPUTS);
    derive_consistent(
        limit,
        || store.memory_revision(),
        || {
            if owner {
                store.export_memories_with_axes_as_owner(project, None, None, None, None, limit)
            } else {
                store.export_memories_with_axes(
                    project,
                    None,
                    None,
                    None,
                    None,
                    limit,
                    principal_acl,
                )
            }
        },
    )
}

fn derive_consistent(
    limit: usize,
    mut revision: impl FnMut() -> Result<u64>,
    mut export: impl FnMut() -> Result<Vec<MemoryRecord>>,
) -> Result<DerivedMemoryResponse> {
    for _ in 0..2 {
        let before = revision()?;
        let memories = export()?;
        if revision()? == before {
            return derive_memory(&memories, before, limit);
        }
    }
    anyhow::bail!("memory changed while derived inputs were being collected; retry")
}

/// Compute a bounded page of higher-order memory projections. Callers must
/// supply only records already authorized for the current principal.
pub fn derive_memory(
    memories: &[MemoryRecord],
    memory_revision: u64,
    limit: usize,
) -> Result<DerivedMemoryResponse> {
    let now = Utc::now();
    let input_limit = limit.clamp(1, MAX_DERIVED_INPUTS);
    let mut visible = memories.iter().take(input_limit).collect::<Vec<_>>();
    visible.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let active = visible
        .iter()
        .copied()
        .filter(|memory| is_active(memory, now))
        .collect::<Vec<_>>();

    let mut representations = Vec::new();
    let mut relations = Vec::new();
    let mut projects = BTreeMap::<&str, Vec<&MemoryRecord>>::new();
    for memory in &active {
        projects.entry(&memory.project).or_default().push(memory);
    }

    for (project, records) in projects {
        for memory in records
            .iter()
            .copied()
            .filter(|memory| memory.content_type == "episodic")
        {
            push_representation(
                &mut representations,
                DerivedKind::Experience,
                project,
                format!("Experience: {}", safe_excerpt(&memory.title, 480)),
                memory.confidence,
                vec![memory],
                Vec::new(),
                memory_revision,
            );
        }

        let mut by_type_scope_and_acl =
            BTreeMap::<(String, String, Vec<String>), Vec<&MemoryRecord>>::new();
        for memory in &records {
            let mut acl = memory.acl.clone();
            acl.sort();
            acl.dedup();
            by_type_scope_and_acl
                .entry((memory.content_type.clone(), memory.scope.clone(), acl))
                .or_default()
                .push(memory);
        }
        for ((content_type, _scope, _acl), support) in by_type_scope_and_acl {
            let confidence = average_confidence(&support);
            push_representation(
                &mut representations,
                DerivedKind::Observation,
                project,
                format!(
                    "Observed {content_type} pattern across {} canonical record(s)",
                    support.len()
                ),
                confidence,
                support.clone(),
                Vec::new(),
                memory_revision,
            );

            if support.len() >= 2
                && matches!(
                    content_type.as_str(),
                    "semantic" | "preference" | "procedural"
                )
            {
                let (supporting, contradicting) = partition_contradictions(&support);
                let kind = if content_type == "preference" {
                    DerivedKind::Belief
                } else {
                    DerivedKind::MentalModel
                };
                push_representation(
                    &mut representations,
                    kind,
                    project,
                    format!(
                        "Interpretation of {} related {content_type} record(s)",
                        support.len()
                    ),
                    confidence,
                    supporting,
                    contradicting,
                    memory_revision,
                );
            }
        }

        derive_relations(&records, &visible, memory_revision, &mut relations);
    }

    representations.truncate(MAX_DERIVED_REPRESENTATIONS.min(input_limit));
    relations.truncate(MAX_DERIVED_RELATIONS.min(input_limit.saturating_mul(4)));
    Ok(DerivedMemoryResponse {
        contract_version: DERIVED_MEMORY_CONTRACT_VERSION.into(),
        derivation_version: DERIVATION_ENGINE_VERSION.into(),
        memory_revision,
        canonical_memory_mutated: false,
        recomputed: true,
        inputs_considered: visible.len(),
        representations,
        relations,
    })
}

#[allow(clippy::too_many_arguments)]
fn push_representation(
    output: &mut Vec<DerivedRepresentation>,
    kind: DerivedKind,
    project: &str,
    statement: String,
    confidence: f32,
    supporting: Vec<&MemoryRecord>,
    contradicting: Vec<&MemoryRecord>,
    memory_revision: u64,
) {
    if output.len() >= MAX_DERIVED_REPRESENTATIONS || supporting.is_empty() {
        return;
    }
    let acl_inputs = supporting
        .iter()
        .chain(&contradicting)
        .copied()
        .collect::<Vec<_>>();
    let Some(acl) = acl_intersection(&acl_inputs) else {
        return;
    };
    let Some(scope) = common_scope(&acl_inputs) else {
        return;
    };
    let supporting_memory_ids = sorted_ids(&supporting);
    let contradicting_memory_ids = sorted_ids(&contradicting);
    let support_digest = support_digest(&supporting, &contradicting);
    let id = format!(
        "derived:{}",
        stable_json_digest(&serde_json::json!([
            DERIVATION_ENGINE_VERSION,
            kind,
            project,
            statement,
            support_digest,
        ]))
    );
    output.push(DerivedRepresentation {
        id,
        contract_version: DERIVED_MEMORY_CONTRACT_VERSION.into(),
        kind,
        project: project.into(),
        scope,
        statement,
        confidence: confidence.clamp(0.0, 1.0),
        supporting_memory_ids,
        contradicting_memory_ids,
        acl,
        memory_revision,
        freshness: "fresh".into(),
        citation_authority: false,
        provenance: DerivationProvenance {
            engine_version: DERIVATION_ENGINE_VERSION.into(),
            input_revision: memory_revision,
            support_digest,
        },
    });
}

fn derive_relations(
    active: &[&MemoryRecord],
    visible: &[&MemoryRecord],
    memory_revision: u64,
    output: &mut Vec<MemoryRelation>,
) {
    let signatures = active
        .iter()
        .map(|memory| {
            let words = normalized_words(&memory.content);
            let tokens = words.iter().cloned().collect::<BTreeSet<_>>();
            let negated = words
                .iter()
                .any(|word| matches!(word.as_str(), "no" | "not" | "never" | "cannot" | "dont"));
            let causal = words.iter().any(|word| word == "because");
            (memory.id.as_str(), (words, tokens, negated, causal))
        })
        .collect::<BTreeMap<_, _>>();
    let visible_by_id = visible
        .iter()
        .map(|memory| (memory.id.as_str(), *memory))
        .collect::<BTreeMap<_, _>>();
    for memory in active {
        if let Some(target) = memory
            .supersedes_id
            .as_deref()
            .and_then(|id| visible_by_id.get(id).copied())
            .filter(|target| target.project == memory.project)
        {
            push_relation(
                output,
                RelationKind::Supersession,
                memory.project.as_str(),
                memory.id.clone(),
                "supersedes".into(),
                target.id.clone(),
                memory.confidence,
                vec![*memory, target],
                memory_revision,
            );
        }

        let words = &signatures[memory.id.as_str()].0;
        if words.len() >= 3 {
            push_relation(
                output,
                RelationKind::SubjectPredicateObject,
                memory.project.as_str(),
                words[0].clone(),
                words[1].clone(),
                words[2..].join(" "),
                memory.confidence,
                vec![*memory],
                memory_revision,
            );
        }
    }

    let mut episodic = active
        .iter()
        .copied()
        .filter(|memory| memory.content_type == "episodic")
        .collect::<Vec<_>>();
    episodic.sort_by(|left, right| left.observed_at.cmp(&right.observed_at));
    for pair in episodic.windows(2) {
        push_relation(
            output,
            RelationKind::Temporal,
            pair[0].project.as_str(),
            pair[0].id.clone(),
            "before".into(),
            pair[1].id.clone(),
            average_confidence(pair),
            pair.to_vec(),
            memory_revision,
        );
    }

    for (index, left) in active.iter().enumerate() {
        for right in active.iter().skip(index + 1) {
            if output.len() >= MAX_DERIVED_RELATIONS || left.project != right.project {
                continue;
            }
            let left_signature = &signatures[left.id.as_str()];
            let right_signature = &signatures[right.id.as_str()];
            let similarity = token_similarity(&left_signature.1, &right_signature.1);
            if similarity < 0.35 {
                continue;
            }
            let contradiction = left_signature.2 != right_signature.2;
            push_relation(
                output,
                if contradiction {
                    RelationKind::Contradiction
                } else {
                    RelationKind::Reinforcement
                },
                left.project.as_str(),
                left.id.clone(),
                if contradiction {
                    "contradicts".into()
                } else {
                    "reinforces".into()
                },
                right.id.clone(),
                average_confidence(&[*left, *right]),
                vec![*left, *right],
                memory_revision,
            );
            if !contradiction && (left_signature.3 || right_signature.3) {
                push_relation(
                    output,
                    RelationKind::Causal,
                    left.project.as_str(),
                    left.id.clone(),
                    "causally-related".into(),
                    right.id.clone(),
                    average_confidence(&[*left, *right]),
                    vec![*left, *right],
                    memory_revision,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_relation(
    output: &mut Vec<MemoryRelation>,
    kind: RelationKind,
    project: &str,
    subject: String,
    predicate: String,
    object: String,
    confidence: f32,
    support: Vec<&MemoryRecord>,
    memory_revision: u64,
) {
    if output.len() >= MAX_DERIVED_RELATIONS || support.is_empty() {
        return;
    }
    let Some(acl) = acl_intersection(&support) else {
        return;
    };
    let Some(scope) = common_scope(&support) else {
        return;
    };
    let supporting_memory_ids = sorted_ids(&support);
    let support_digest = support_digest(&support, &[]);
    let id = format!(
        "relation:{}",
        stable_json_digest(&serde_json::json!([
            DERIVATION_ENGINE_VERSION,
            kind,
            project,
            subject,
            predicate,
            object,
            support_digest,
        ]))
    );
    output.push(MemoryRelation {
        id,
        contract_version: DERIVED_MEMORY_CONTRACT_VERSION.into(),
        kind,
        project: project.into(),
        scope,
        subject,
        predicate,
        object,
        confidence: confidence.clamp(0.0, 1.0),
        supporting_memory_ids,
        acl,
        memory_revision,
        freshness: "fresh".into(),
        citation_authority: false,
        provenance: DerivationProvenance {
            engine_version: DERIVATION_ENGINE_VERSION.into(),
            input_revision: memory_revision,
            support_digest,
        },
    });
}

fn is_active(memory: &MemoryRecord, now: DateTime<Utc>) -> bool {
    if memory.status != "active" {
        return false;
    }
    let valid_from = DateTime::parse_from_rfc3339(&memory.valid_from)
        .map(|value| value.with_timezone(&Utc))
        .ok();
    let valid_until = memory
        .valid_until
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    valid_from.is_some_and(|value| value <= now) && valid_until.is_none_or(|value| value > now)
}

fn acl_intersection(memories: &[&MemoryRecord]) -> Option<Vec<String>> {
    let mut intersection = memories
        .first()?
        .acl
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for memory in memories.iter().skip(1) {
        let acl = memory.acl.iter().collect::<BTreeSet<_>>();
        intersection.retain(|label| acl.contains(label));
    }
    if memories.len() > 1 && intersection.is_empty() {
        return None;
    }
    Some(intersection.into_iter().collect())
}

fn common_scope(memories: &[&MemoryRecord]) -> Option<String> {
    let scope = memories.first()?.scope.as_str();
    memories
        .iter()
        .all(|memory| memory.scope == scope)
        .then(|| scope.to_string())
}

fn sorted_ids(memories: &[&MemoryRecord]) -> Vec<String> {
    memories
        .iter()
        .map(|memory| memory.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn support_digest(supporting: &[&MemoryRecord], contradicting: &[&MemoryRecord]) -> String {
    let describe = |memory: &&MemoryRecord| {
        serde_json::json!({
            "id": memory.id,
            "updated_at": memory.updated_at,
            "status": memory.status,
        })
    };
    stable_json_digest(&serde_json::json!({
        "supporting": supporting.iter().map(describe).collect::<Vec<_>>(),
        "contradicting": contradicting.iter().map(describe).collect::<Vec<_>>(),
    }))
}

fn average_confidence(memories: &[&MemoryRecord]) -> f32 {
    if memories.is_empty() {
        return 0.0;
    }
    memories.iter().map(|memory| memory.confidence).sum::<f32>() / memories.len() as f32
}

fn partition_contradictions<'a>(
    memories: &[&'a MemoryRecord],
) -> (Vec<&'a MemoryRecord>, Vec<&'a MemoryRecord>) {
    let expected_negation = has_negation(&memories[0].content);
    memories
        .iter()
        .copied()
        .partition(|memory| has_negation(&memory.content) == expected_negation)
}

fn normalized_words(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| !character.is_ascii_alphanumeric())
                .to_ascii_lowercase()
                .chars()
                .take(32)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .take(12)
        .collect()
}

fn safe_excerpt(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn token_similarity(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f32 {
    let total = left.len() + right.len();
    if total == 0 {
        0.0
    } else {
        (2 * left.intersection(right).count()) as f32 / total as f32
    }
}

fn has_negation(value: &str) -> bool {
    normalized_words(value)
        .iter()
        .any(|word| matches!(word.as_str(), "no" | "not" | "never" | "cannot" | "dont"))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;

    fn memory(id: &str, content: &str, content_type: &str, acl: &[&str]) -> MemoryRecord {
        MemoryRecord {
            id: id.into(),
            kind: content_type.into(),
            content_type: content_type.into(),
            retention_tier: "durable".into(),
            scope: "workspace".into(),
            project: "work".into(),
            title: format!("Memory {id}"),
            content: content.into(),
            source: "test".into(),
            source_id: id.into(),
            dedupe_key: None,
            confidence: 0.8,
            importance: 0.5,
            status: "active".into(),
            acl: acl.iter().map(|value| (*value).into()).collect(),
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
    fn derives_all_representation_and_relation_families_with_provenance() {
        let mut old = memory("old", "deploy because tests pass", "semantic", &["work"]);
        old.status = "superseded".into();
        let mut current = memory(
            "current",
            "deploy because tests pass",
            "semantic",
            &["work"],
        );
        current.supersedes_id = Some("old".into());
        let memories = vec![
            memory("e1", "release started today", "episodic", &["work"]),
            memory("e2", "release finished today", "episodic", &["work"]),
            memory("p1", "prefer concise notes", "preference", &["work"]),
            memory("p2", "do not prefer concise notes", "preference", &["work"]),
            memory(
                "current-2",
                "deploy because checks pass",
                "semantic",
                &["work"],
            ),
            old,
            current,
        ];

        let response = derive_memory(&memories, 11, 100).expect("derive");
        assert!(
            response
                .representations
                .iter()
                .any(|item| item.kind == DerivedKind::Experience)
        );
        assert!(
            response
                .representations
                .iter()
                .any(|item| item.kind == DerivedKind::Observation)
        );
        assert!(
            response
                .representations
                .iter()
                .any(|item| matches!(item.kind, DerivedKind::MentalModel | DerivedKind::Belief))
        );
        assert!(
            response
                .relations
                .iter()
                .any(|item| item.kind == RelationKind::Temporal)
        );
        assert!(
            response
                .relations
                .iter()
                .any(|item| item.kind == RelationKind::Contradiction)
        );
        assert!(
            response
                .relations
                .iter()
                .any(|item| item.kind == RelationKind::Supersession)
        );
        for kind in [
            RelationKind::SubjectPredicateObject,
            RelationKind::Causal,
            RelationKind::Reinforcement,
        ] {
            assert!(
                response.relations.iter().any(|item| item.kind == kind),
                "missing {kind:?} relation"
            );
        }
        assert!(response.representations.iter().all(|item| {
            !item.supporting_memory_ids.is_empty()
                && item.provenance.input_revision == 11
                && !item.citation_authority
        }));
        assert!(!response.canonical_memory_mutated);
    }

    #[test]
    fn lifecycle_and_acl_intersection_fail_closed() {
        let mut retired = memory("retired", "shared pattern", "semantic", &["team-a"]);
        retired.status = "retracted".into();
        let memories = vec![
            retired,
            memory("a", "shared pattern", "semantic", &["team-a"]),
            memory("b", "shared pattern", "semantic", &["team-b"]),
        ];
        let response = derive_memory(&memories, 2, 100).expect("derive");
        assert!(response.representations.iter().all(|item| {
            !item.supporting_memory_ids.contains(&"retired".into())
                && item.supporting_memory_ids.len() == 1
        }));
        assert!(
            response
                .relations
                .iter()
                .all(|item| { item.supporting_memory_ids.len() == 1 || !item.acl.is_empty() })
        );
    }

    #[test]
    fn output_is_bounded_and_revision_changes_fresh_projection() {
        let memories = (0..200)
            .map(|index| {
                memory(
                    &format!("m{index}"),
                    "repeat pattern",
                    "semantic",
                    &["work"],
                )
            })
            .collect::<Vec<_>>();
        let first = derive_memory(&memories, 3, usize::MAX).expect("first");
        let second = derive_memory(&memories, 4, usize::MAX).expect("second");
        assert_eq!(first.inputs_considered, MAX_DERIVED_INPUTS);
        assert!(first.representations.len() <= MAX_DERIVED_REPRESENTATIONS);
        assert!(first.relations.len() <= MAX_DERIVED_RELATIONS);
        assert!(
            second
                .representations
                .iter()
                .all(|item| item.memory_revision == 4)
        );
    }

    #[test]
    fn concurrent_revision_change_retries_without_mislabeling_support() {
        let memories = vec![memory("stable", "bounded support", "semantic", &["work"])];
        let mut revisions = [7, 8, 8, 8].into_iter();
        let mut exports = 0;
        let response = derive_consistent(
            10,
            || Ok(revisions.next().expect("scripted revision")),
            || {
                exports += 1;
                Ok(memories.clone())
            },
        )
        .expect("retry stable snapshot");
        assert_eq!(exports, 2);
        assert_eq!(response.memory_revision, 8);
        assert!(
            response
                .representations
                .iter()
                .all(|item| item.memory_revision == 8)
        );
    }
}
