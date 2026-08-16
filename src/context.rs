use serde::Serialize;

use crate::memory::MemorySearchResult;
use crate::model::Evidence;

const CHARS_PER_TOKEN: usize = 4;
pub const MIN_CONTEXT_TOKENS: usize = 256;
pub const MAX_CONTEXT_TOKENS: usize = 64_000;

#[derive(Clone, Debug, Serialize)]
pub struct ContextBundle {
    pub query: String,
    pub context: String,
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<MemorySearchResult>,
    pub metrics: ContextMetrics,
    pub retrieval_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_warning: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContextMetrics {
    pub retrieved: usize,
    pub included: usize,
    pub omitted: usize,
    pub memories_retrieved: usize,
    pub memories_included: usize,
    pub memories_omitted: usize,
    pub estimated_tokens: usize,
    pub max_tokens: usize,
}

pub fn build(query: &str, evidence: &[Evidence], max_tokens: usize) -> ContextBundle {
    build_with_retrieval(query, evidence, max_tokens, "hybrid", None)
}

pub fn build_with_retrieval(
    query: &str,
    evidence: &[Evidence],
    max_tokens: usize,
    retrieval_mode: &str,
    retrieval_warning: Option<&str>,
) -> ContextBundle {
    build_with_retrieval_and_memory(
        query,
        evidence,
        &[],
        max_tokens,
        retrieval_mode,
        retrieval_warning,
    )
}

/// Build a bounded context bundle with source evidence and ACL-filtered native
/// memories. Memories are deliberately separated from evidence so agents can
/// use durable conclusions without presenting them as source citations.
pub fn build_with_retrieval_and_memory(
    query: &str,
    evidence: &[Evidence],
    memories: &[MemorySearchResult],
    max_tokens: usize,
    retrieval_mode: &str,
    retrieval_warning: Option<&str>,
) -> ContextBundle {
    let max_tokens = max_tokens.clamp(MIN_CONTEXT_TOKENS, MAX_CONTEXT_TOKENS);
    let max_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN);
    let query_prefix = "# Cortana evidence context\n\nQuery: ";
    let instructions = if memories.is_empty() {
        "\n\nUse only the evidence below for factual claims. Cite sources with [n]."
    } else {
        "\n\nUse source evidence for factual claims and cite it with [n]. Treat agent memory as scoped operational context, not an external citation."
    };
    let query_budget = max_chars.saturating_sub(query_prefix.len() + instructions.len());
    let bounded_query = truncate(query, query_budget);
    let mut context = format!("{query_prefix}{bounded_query}{instructions}");
    let mut included_memories = Vec::new();
    let mut included = Vec::new();

    if !memories.is_empty() {
        let memory_budget = max_chars / 3;
        let memory_start = context.len();
        context.push_str("\n\n## Agent memory\n\n");
        for memory in memories {
            let prefix = memory_prefix(included_memories.len() + 1, memory);
            let reserved = context.len() + prefix.len() + 4;
            let memory_used = context.len().saturating_sub(memory_start);
            if reserved >= max_chars || memory_used >= memory_budget {
                break;
            }
            let available = (max_chars - reserved).min(memory_budget - memory_used);
            let content = truncate(&memory.memory.content, available);
            if content.is_empty() {
                break;
            }
            context.push_str(&prefix);
            context.push_str(&content);
            included_memories.push(memory.clone());
        }
    }

    for item in evidence {
        let index = included.len() + 1;
        let prefix = evidence_prefix(index, item);
        let reserved = context.len() + prefix.len() + 4;
        if reserved >= max_chars {
            break;
        }
        let available = max_chars - reserved;
        let content = truncate(&item.content, available);
        if content.is_empty() {
            break;
        }
        let mut selected = item.clone();
        selected.content = content;
        context.push_str("\n\n");
        context.push_str(&prefix);
        context.push_str(&selected.content);
        included.push(selected);
    }

    let estimated_tokens = estimate_tokens(&context);
    ContextBundle {
        query: query.to_string(),
        context,
        metrics: ContextMetrics {
            retrieved: evidence.len(),
            included: included.len(),
            omitted: evidence.len().saturating_sub(included.len()),
            memories_retrieved: memories.len(),
            memories_included: included_memories.len(),
            memories_omitted: memories.len().saturating_sub(included_memories.len()),
            estimated_tokens,
            max_tokens,
        },
        evidence: included,
        memories: included_memories,
        retrieval_mode: retrieval_mode.to_string(),
        retrieval_warning: retrieval_warning.map(str::to_string),
    }
}

pub fn estimate_tokens(value: &str) -> usize {
    value.len().div_ceil(CHARS_PER_TOKEN).max(1)
}

fn evidence_prefix(index: usize, item: &Evidence) -> String {
    let location = item
        .uri
        .as_ref()
        .map(|uri| format!(" ({uri})"))
        .unwrap_or_default();
    format!(
        "### [{index}] {}{location}\nSource: {} · Updated: {}\n\n",
        item.title,
        item.source,
        item.updated_at.to_rfc3339()
    )
}

fn memory_prefix(index: usize, item: &MemorySearchResult) -> String {
    let expiry = item
        .memory
        .valid_until
        .as_deref()
        .map(|value| format!(" · Expires: {value}"))
        .unwrap_or_default();
    format!(
        "### [memory {index}] {} ({})\nProject: {} · Source: {} · Observed: {} · Confidence: {:.2} · Importance: {:.2}{expiry}\n\n",
        item.memory.title,
        item.memory.kind,
        item.memory.project,
        item.memory.source,
        item.memory.observed_at,
        item.memory.confidence,
        item.memory.importance
    )
}

fn truncate(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let marker = "\n[…truncated]";
    if max_bytes <= marker.len() {
        return String::new();
    }
    let target = max_bytes - marker.len();
    let keep = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= target)
        .last()
        .unwrap_or(0);
    let mut output = value[..keep].to_string();
    output.push_str(marker);
    output
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn evidence(content: &str) -> Evidence {
        Evidence {
            chunk_id: "doc:0".into(),
            source: "notes".into(),
            source_id: "doc".into(),
            title: "Release playbook".into(),
            uri: Some("file:///playbook.md".into()),
            content: content.into(),
            score: 0.9,
            semantic_rank: Some(1),
            lexical_rank: Some(2),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn builds_cited_agent_context_with_metrics() {
        let rows = vec![evidence("Deploy after validation.")];
        let bundle = build("How do releases work?", &rows, 2_000);
        assert!(bundle.context.contains("### [1] Release playbook"));
        assert!(bundle.context.contains("Cite sources with [n]"));
        assert_eq!(bundle.metrics.included, 1);
        assert_eq!(bundle.metrics.omitted, 0);
        assert_eq!(bundle.metrics.memories_included, 0);
        assert_eq!(bundle.evidence, rows);
        assert_eq!(bundle.retrieval_mode, "hybrid");
        assert!(bundle.retrieval_warning.is_none());
    }

    #[test]
    fn respects_budget_without_splitting_unicode() {
        let rows = vec![evidence(&"🧠".repeat(4_000)), evidence("second")];
        let bundle = build("memory", &rows, 256);
        assert!(bundle.context.contains("[…truncated]"));
        assert!(bundle.metrics.estimated_tokens <= 256);
        assert_eq!(bundle.metrics.included, 1);
        assert_eq!(bundle.metrics.omitted, 1);
    }

    #[test]
    fn bounds_a_maximum_length_query_inside_the_context_budget() {
        let bundle = build(&"query ".repeat(4_000), &[], 256);
        assert!(bundle.context.contains("[…truncated]"));
        assert!(bundle.metrics.estimated_tokens <= 256);
        assert!(bundle.context.len() <= 256 * CHARS_PER_TOKEN);
    }

    #[test]
    fn includes_native_memories_before_evidence_with_separate_metrics() {
        let memory = MemorySearchResult {
            memory: crate::memory::MemoryRecord {
                id: "memory-1".into(),
                kind: "preference".into(),
                project: "work".into(),
                title: "Release style".into(),
                content: "Prefer concise release notes.".into(),
                source: "agent".into(),
                source_id: "memory-1".into(),
                dedupe_key: None,
                confidence: 0.9,
                importance: 0.8,
                status: "active".into(),
                acl: vec!["work".into()],
                provenance: serde_json::json!({"source":"test"}),
                observed_at: "2026-01-01T00:00:00Z".into(),
                valid_from: "2026-01-01T00:00:00Z".into(),
                valid_until: None,
                supersedes_id: None,
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
            },
            lexical_score: -1.0,
        };
        let bundle = build_with_retrieval_and_memory(
            "release notes",
            &[evidence("Deploy after validation.")],
            &[memory],
            2_000,
            "hybrid",
            None,
        );
        assert_eq!(bundle.memories.len(), 1);
        assert_eq!(bundle.metrics.memories_retrieved, 1);
        assert_eq!(bundle.metrics.memories_included, 1);
        assert!(bundle.context.contains("## Agent memory"));
        assert!(bundle.context.contains("### [memory 1] Release style"));
        assert!(bundle.context.find("## Agent memory") < bundle.context.find("### [1]"));
    }
}
