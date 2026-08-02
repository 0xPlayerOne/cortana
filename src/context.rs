use serde::Serialize;

use crate::model::Evidence;

const CHARS_PER_TOKEN: usize = 4;
pub const MIN_CONTEXT_TOKENS: usize = 256;
pub const MAX_CONTEXT_TOKENS: usize = 64_000;

#[derive(Clone, Debug, Serialize)]
pub struct ContextBundle {
    pub query: String,
    pub context: String,
    pub evidence: Vec<Evidence>,
    pub metrics: ContextMetrics,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContextMetrics {
    pub retrieved: usize,
    pub included: usize,
    pub omitted: usize,
    pub estimated_tokens: usize,
    pub max_tokens: usize,
}

pub fn build(query: &str, evidence: &[Evidence], max_tokens: usize) -> ContextBundle {
    let max_tokens = max_tokens.clamp(MIN_CONTEXT_TOKENS, MAX_CONTEXT_TOKENS);
    let max_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN);
    let query_prefix = "# Cortana evidence context\n\nQuery: ";
    let instructions = "\n\nUse only the evidence below for factual claims. Cite sources with [n].";
    let query_budget = max_chars.saturating_sub(query_prefix.len() + instructions.len());
    let bounded_query = truncate(query, query_budget);
    let mut context = format!("{query_prefix}{bounded_query}{instructions}");
    let mut included = Vec::new();

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
            estimated_tokens,
            max_tokens,
        },
        evidence: included,
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
        assert_eq!(bundle.evidence, rows);
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
}
