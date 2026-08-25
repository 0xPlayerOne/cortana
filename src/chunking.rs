//! Deterministic, source-aware chunk generation.
//!
//! Documents are canonical records. Chunks are derived records and may be
//! regenerated whenever this contract changes. The chunker deliberately uses
//! only document content and non-secret metadata, so equal inputs always
//! produce equal output without contacting a provider.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::Document;

pub const CHUNKING_CONTRACT_VERSION: &str = "cortana.chunking.v1";
pub const DEFAULT_TARGET_BYTES: usize = 1_600;
pub const DEFAULT_OVERLAP_BYTES: usize = 200;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStrategy {
    Generic,
    MarkdownSection,
    HtmlSection,
    MessageThread,
    CalendarEvent,
    StructuredRecord,
}

impl ChunkStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::MarkdownSection => "markdown_section",
            Self::HtmlSection => "html_section",
            Self::MessageThread => "message_thread",
            Self::CalendarEvent => "calendar_event",
            Self::StructuredRecord => "structured_record",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkingPolicy {
    pub target_bytes: usize,
    pub overlap_bytes: usize,
}

impl Default for ChunkingPolicy {
    fn default() -> Self {
        Self {
            target_bytes: DEFAULT_TARGET_BYTES,
            overlap_bytes: DEFAULT_OVERLAP_BYTES,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ChunkSpec {
    /// Stable within a document and policy. Storage prefixes it with the
    /// stable document ID to produce the externally visible chunk ID.
    pub key: String,
    pub content: String,
    pub ordinal: usize,
    pub strategy: ChunkStrategy,
    pub parent_key: String,
    pub previous_key: Option<String>,
    pub next_key: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub policy_version: &'static str,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ChunkingStats {
    pub strategy: String,
    pub chunks: usize,
    pub source_bytes: usize,
    pub chunk_bytes: usize,
    pub overlap_bytes: usize,
}

/// Generate chunks using source metadata when it is explicit and safe to do
/// so. Unknown sources always use the legacy-compatible generic chunker.
pub fn chunk_document(document: &Document) -> Vec<ChunkSpec> {
    chunk_document_with_policy(document, ChunkingPolicy::default())
}

pub fn chunk_document_with_policy(document: &Document, policy: ChunkingPolicy) -> Vec<ChunkSpec> {
    let strategy = strategy_for(document);
    let mut chunks = match strategy {
        ChunkStrategy::MarkdownSection => {
            section_chunks(&document.content, strategy, policy, is_markdown_heading)
        }
        ChunkStrategy::HtmlSection => {
            section_chunks(&document.content, strategy, policy, is_html_heading)
        }
        ChunkStrategy::MessageThread => message_chunks(&document.content, policy),
        ChunkStrategy::CalendarEvent | ChunkStrategy::StructuredRecord => {
            structured_chunks(&document.content, strategy, policy)
        }
        ChunkStrategy::Generic => generic_chunks(&document.content, policy),
    };
    link_chunks(&mut chunks);
    chunks
}

pub fn stats(document: &Document) -> ChunkingStats {
    let chunks = chunk_document(document);
    let chunk_bytes: usize = chunks.iter().map(|chunk| chunk.content.len()).sum();
    let overlap_bytes = chunk_bytes.saturating_sub(document.content.len());
    ChunkingStats {
        strategy: chunks.first().map_or_else(
            || strategy_for(document).as_str().to_string(),
            |chunk| chunk.strategy.as_str().to_string(),
        ),
        chunks: chunks.len(),
        source_bytes: document.content.len(),
        chunk_bytes,
        overlap_bytes,
    }
}

fn strategy_for(document: &Document) -> ChunkStrategy {
    let metadata = document.metadata.as_object();
    let value = |key: &str| {
        metadata
            .and_then(|map| map.get(key))
            .and_then(serde_json::Value::as_str)
    };
    let kind = value("mime_type")
        .or_else(|| value("mimeType"))
        .or_else(|| value("content_type"))
        .or_else(|| value("format"))
        .or_else(|| value("extension"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if kind.contains("markdown")
        || kind.contains("google-apps.document")
        || kind.contains("wordprocessingml")
        || kind.contains("opendocument.text")
        || matches!(kind.as_str(), "md" | ".md")
    {
        return ChunkStrategy::MarkdownSection;
    }
    if kind.contains("html") || matches!(kind.as_str(), "htm" | ".htm" | "html" | ".html") {
        return ChunkStrategy::HtmlSection;
    }
    if metadata.is_some_and(|map| {
        map.contains_key("thread_id")
            || map.contains_key("message_id")
            || map.contains_key("channel_id")
            || map.contains_key("participants")
    }) || matches!(document.source.as_str(), "gmail" | "slack" | "discord")
    {
        return ChunkStrategy::MessageThread;
    }
    if metadata.is_some_and(|map| {
        map.contains_key("event_id")
            || map.contains_key("start")
            || map.contains_key("start_time")
            || map.contains_key("recurrence")
    }) || document.source.contains("calendar")
    {
        return ChunkStrategy::CalendarEvent;
    }
    if metadata.is_some_and(|map| map.contains_key("fields") || map.contains_key("record_type")) {
        return ChunkStrategy::StructuredRecord;
    }
    ChunkStrategy::Generic
}

fn generic_chunks(content: &str, policy: ChunkingPolicy) -> Vec<ChunkSpec> {
    bounded_ranges(content, policy, ChunkStrategy::Generic, |_| false)
}

fn section_chunks(
    content: &str,
    strategy: ChunkStrategy,
    policy: ChunkingPolicy,
    heading: fn(&str) -> bool,
) -> Vec<ChunkSpec> {
    let mut sections = Vec::<(usize, usize)>::new();
    let mut start = 0;
    for (offset, line) in content
        .split_inclusive('\n')
        .map(|line| (line.as_ptr() as usize, line))
    {
        let offset = offset.saturating_sub(content.as_ptr() as usize);
        if heading(line) && offset > start {
            sections.push((start, offset));
            start = offset;
        }
    }
    if start < content.len() {
        sections.push((start, content.len()));
    }
    if sections.is_empty() {
        return bounded_ranges(content, policy, strategy, |_| false);
    }
    sections
        .into_iter()
        .flat_map(|(start, end)| {
            let parent = stable_hash(&format!(
                "{CHUNKING_CONTRACT_VERSION}:{}:{start}:{end}",
                strategy.as_str()
            ));
            bounded_ranges(&content[start..end], policy, strategy, |_| false)
                .into_iter()
                .map(move |mut chunk| {
                    chunk.start_byte += start;
                    chunk.end_byte += start;
                    chunk.parent_key = parent.clone();
                    chunk
                })
        })
        .collect()
}

fn message_chunks(content: &str, policy: ChunkingPolicy) -> Vec<ChunkSpec> {
    // Keep message boundaries when a connector provides an exported
    // transcript. A single-message document naturally falls through to the
    // bounded paragraph splitter.
    let mut records = Vec::new();
    let mut start = 0;
    for (offset, line) in content
        .split_inclusive('\n')
        .map(|line| (line.as_ptr() as usize, line))
    {
        let offset = offset.saturating_sub(content.as_ptr() as usize);
        let marker = line.trim_start();
        if (marker.starts_with("From:") || marker.starts_with('>') || marker.starts_with("["))
            && offset > start
        {
            records.push((start, offset));
            start = offset;
        }
    }
    if start < content.len() {
        records.push((start, content.len()));
    }
    if records.len() <= 1 {
        return bounded_ranges(content, policy, ChunkStrategy::MessageThread, |_| false);
    }
    records
        .into_iter()
        .flat_map(|(start, end)| {
            bounded_ranges(
                &content[start..end],
                policy,
                ChunkStrategy::MessageThread,
                |_| false,
            )
            .into_iter()
            .map(move |mut chunk| {
                chunk.start_byte += start;
                chunk.end_byte += start;
                chunk.parent_key = stable_hash(&format!("message:{start}:{end}"));
                chunk
            })
        })
        .collect()
}

fn structured_chunks(
    content: &str,
    strategy: ChunkStrategy,
    policy: ChunkingPolicy,
) -> Vec<ChunkSpec> {
    let mut fields = Vec::<(usize, usize)>::new();
    let mut start = 0;
    for (offset, line) in content
        .split_inclusive('\n')
        .map(|line| (line.as_ptr() as usize, line))
    {
        let offset = offset.saturating_sub(content.as_ptr() as usize);
        if line.contains(':')
            && offset > start
            && line.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        {
            fields.push((start, offset));
            start = offset;
        }
    }
    if start < content.len() {
        fields.push((start, content.len()));
    }
    if fields.len() <= 1 {
        return bounded_ranges(content, policy, strategy, |_| false);
    }
    fields
        .into_iter()
        .flat_map(|(start, end)| {
            bounded_ranges(&content[start..end], policy, strategy, |_| false)
                .into_iter()
                .map(move |mut chunk| {
                    chunk.start_byte += start;
                    chunk.end_byte += start;
                    chunk.parent_key = stable_hash(&format!("record:{start}:{end}"));
                    chunk
                })
        })
        .collect()
}

fn bounded_ranges(
    content: &str,
    policy: ChunkingPolicy,
    strategy: ChunkStrategy,
    _boundary: fn(&str) -> bool,
) -> Vec<ChunkSpec> {
    let target = policy.target_bytes.max(1);
    let overlap = policy.overlap_bytes.min(target.saturating_sub(1));
    let mut output = Vec::new();
    let mut start = 0;
    while start < content.len() {
        let hard_end = char_boundary_at_or_before(content, start.saturating_add(target));
        let mut end = hard_end.max(next_char_boundary(content, start));
        if end < content.len() {
            let window = &content[start..end];
            let floor = window.len() / 2;
            end = window
                .rfind("\n\n")
                .filter(|position| *position >= floor)
                .map_or(end, |position| start + position + 2);
            end = char_boundary_at_or_before(content, end);
        }
        let text = content[start..end].trim();
        if !text.is_empty() {
            let start_byte = start + content[start..end].find(text).unwrap_or(0);
            let end_byte = start_byte + text.len();
            output.push(ChunkSpec {
                key: stable_hash(&format!(
                    "{CHUNKING_CONTRACT_VERSION}:{}:{start_byte}:{end_byte}:{text}",
                    strategy.as_str()
                )),
                content: text.to_string(),
                ordinal: output.len(),
                strategy,
                parent_key: stable_hash(&format!(
                    "{CHUNKING_CONTRACT_VERSION}:parent:{start}:{end}"
                )),
                previous_key: None,
                next_key: None,
                start_byte,
                end_byte,
                policy_version: CHUNKING_CONTRACT_VERSION,
            });
        }
        if end == content.len() {
            break;
        }
        let mut next = char_boundary_at_or_before(content, end.saturating_sub(overlap));
        if next <= start {
            next = next_char_boundary(content, end);
        }
        start = next;
    }
    output
}

fn link_chunks(chunks: &mut [ChunkSpec]) {
    for index in 0..chunks.len() {
        chunks[index].ordinal = index;
        chunks[index].previous_key = index
            .checked_sub(1)
            .map(|previous| chunks[previous].key.clone());
        chunks[index].next_key = (index + 1 < chunks.len()).then(|| chunks[index + 1].key.clone());
    }
}

fn char_boundary_at_or_before(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while offset > 0 && !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn next_char_boundary(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while offset < value.len() && !value.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

fn stable_hash(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(value.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_markdown_heading(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

fn is_html_heading(line: &str) -> bool {
    let lower = line.trim_start().to_ascii_lowercase();
    (1..=6).any(|level| lower.starts_with(&format!("<h{level}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn document(source: &str, content: &str, metadata: serde_json::Value) -> Document {
        Document {
            source: source.into(),
            source_id: "stable-source-id".into(),
            title: "Fixture".into(),
            content: content.into(),
            uri: None,
            updated_at: chrono::Utc::now(),
            project: "work".into(),
            acl: vec!["work".into()],
            metadata,
        }
    }

    #[test]
    fn generic_fallback_is_bounded_and_unicode_safe() {
        let value = document("unknown", &"🧠".repeat(2_000), json!({}));
        let chunks = chunk_document(&value);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.content.len() <= DEFAULT_TARGET_BYTES)
        );
        assert_eq!(chunks, chunk_document(&value));
        assert!(chunks.iter().any(|chunk| chunk.content.contains('🧠')));
    }

    #[test]
    fn markdown_sections_keep_parent_and_neighbors_stable() {
        let value = document(
            "drive",
            "# One\nalpha\n\n# Two\nbeta",
            json!({"mime_type":"text/markdown"}),
        );
        let chunks = chunk_document(&value);
        assert_eq!(chunks[0].strategy, ChunkStrategy::MarkdownSection);
        assert!(chunks[0].next_key.is_some());
        assert_eq!(
            chunks[1].previous_key.as_deref(),
            Some(chunks[0].key.as_str())
        );
        assert_eq!(chunks[0].parent_key, chunks[0].parent_key.clone());
    }

    #[test]
    fn message_and_event_metadata_select_structured_strategies() {
        let message = document(
            "gmail",
            "From: A\nhello\nFrom: B\nreply",
            json!({"thread_id":"t1"}),
        );
        let event = document(
            "google-calendar",
            "title: Launch\nstart: 2026-01-01",
            json!({"event_id":"e1"}),
        );
        assert_eq!(
            chunk_document(&message)[0].strategy,
            ChunkStrategy::MessageThread
        );
        assert_eq!(
            chunk_document(&event)[0].strategy,
            ChunkStrategy::CalendarEvent
        );
    }

    #[test]
    fn stats_expose_bounded_comparison_inputs() {
        let value = document("unknown", "one\n\ntwo", json!({}));
        let stats = stats(&value);
        assert_eq!(stats.source_bytes, value.content.len());
        assert!(stats.chunk_bytes >= stats.source_bytes);
    }
}
