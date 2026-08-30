use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;

use crate::embed::Embedder;
use crate::model::{Evidence, StoredChunk};
use crate::store::Store;

const INTERACTIVE_EMBEDDING_TIMEOUT: Duration = Duration::from_secs(5);
const NEIGHBOR_RADIUS: usize = 1;
const MAX_EXPANDED_CONTENT_BYTES: usize = 16 * 1024;
pub const MAX_QUERY_BYTES: usize = 16 * 1024;
/// The public retrieval result cap shared by MCP, HTTP, and the CLI.
pub const MAX_RESULT_LIMIT: usize = 50;
/// Bump this when ranking inputs or weights change. It is included in query
/// cache keys so a new ranking policy cannot reuse an old answer.
pub const RETRIEVAL_RANKING_VERSION: &str = "cortana.retrieval.ranking.v2";

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RetrievalTuning {
    pub candidate_multiplier: usize,
    pub semantic_weight: f32,
    pub lexical_weight: f32,
    pub idf_weight: f32,
    pub recency_weight: f32,
    /// Apply the bounded, deterministic local reranker after hybrid fusion.
    /// It never calls a provider and remains disabled by default.
    pub reranker_enabled: bool,
}

impl Default for RetrievalTuning {
    fn default() -> Self {
        Self {
            candidate_multiplier: 8,
            semantic_weight: 1.0,
            lexical_weight: 1.2,
            idf_weight: 0.08,
            recency_weight: 0.1,
            reranker_enabled: false,
        }
    }
}

impl RetrievalTuning {
    pub fn bounded(self) -> Self {
        Self {
            candidate_multiplier: self.candidate_multiplier.clamp(1, 32),
            semantic_weight: bounded_weight(self.semantic_weight, 1.0, 4.0),
            lexical_weight: bounded_weight(self.lexical_weight, 1.2, 4.0),
            idf_weight: bounded_weight(self.idf_weight, 0.08, 1.0),
            recency_weight: bounded_weight(self.recency_weight, 0.1, 1.0),
            reranker_enabled: self.reranker_enabled,
        }
    }
}

fn bounded_weight(value: f32, fallback: f32, maximum: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, maximum)
    } else {
        fallback
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RetrievalDiagnostics {
    pub contract_version: &'static str,
    pub candidate_limit: usize,
    pub semantic_candidates: usize,
    pub lexical_candidates: usize,
    pub fused_candidates: usize,
    pub deduplicated_candidates: usize,
    pub returned: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetrievalMode {
    Hybrid,
    LexicalFallback,
}

impl RetrievalMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::LexicalFallback => "lexical-fallback",
        }
    }
}

#[derive(Debug)]
pub struct RetrievalOutcome {
    pub evidence: Vec<Evidence>,
    pub mode: RetrievalMode,
    pub warning: Option<String>,
    pub diagnostics: RetrievalDiagnostics,
}

impl RetrievalOutcome {
    pub fn degraded(&self) -> bool {
        self.mode == RetrievalMode::LexicalFallback
    }
}

pub async fn retrieve(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
) -> Result<Vec<Evidence>> {
    retrieve_scoped(
        store,
        embedder,
        query,
        project,
        source,
        limit,
        &["*".into()],
    )
    .await
}

pub async fn retrieve_scoped(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    principal_acl: &[String],
) -> Result<Vec<Evidence>> {
    Ok(retrieve_scoped_with_status(
        store,
        embedder,
        query,
        project,
        source,
        limit,
        principal_acl,
    )
    .await?
    .evidence)
}

pub async fn retrieve_scoped_with_status(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    principal_acl: &[String],
) -> Result<RetrievalOutcome> {
    retrieve_with_timeout_status(
        store,
        embedder,
        query,
        project,
        source,
        limit,
        INTERACTIVE_EMBEDDING_TIMEOUT,
        principal_acl,
        RetrievalTuning::default(),
    )
    .await
}

pub async fn retrieve_sources_scoped(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    sources: &[String],
    limit: usize,
    principal_acl: &[String],
) -> Result<Vec<Evidence>> {
    Ok(retrieve_sources_scoped_with_status(
        store,
        embedder,
        query,
        project,
        sources,
        limit,
        principal_acl,
    )
    .await?
    .evidence)
}

pub async fn retrieve_sources_scoped_with_status(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    sources: &[String],
    limit: usize,
    principal_acl: &[String],
) -> Result<RetrievalOutcome> {
    validate_query(query)?;
    let mut unique_sources = sources
        .iter()
        .map(|source| source.trim())
        .filter(|source| !source.is_empty())
        .collect::<Vec<_>>();
    unique_sources.sort_unstable();
    unique_sources.dedup();
    unique_sources.truncate(32);
    if unique_sources.is_empty() {
        return Ok(RetrievalOutcome {
            evidence: Vec::new(),
            mode: RetrievalMode::Hybrid,
            warning: None,
            diagnostics: RetrievalDiagnostics {
                contract_version: RETRIEVAL_RANKING_VERSION,
                ..RetrievalDiagnostics::default()
            },
        });
    }
    let (query_embedding, mut warning) =
        query_embedding(embedder, query, INTERACTIVE_EMBEDDING_TIMEOUT).await;
    let result_limit = limit.min(MAX_RESULT_LIMIT);
    let mut fused = HashMap::<String, (Evidence, f32)>::new();
    let mut diagnostics = RetrievalDiagnostics {
        contract_version: RETRIEVAL_RANKING_VERSION,
        ..RetrievalDiagnostics::default()
    };
    for source in unique_sources {
        let rows = match rank_with_tuning(
            store,
            query,
            query_embedding.as_deref(),
            project,
            Some(source),
            result_limit,
            principal_acl,
            RetrievalTuning::default(),
        ) {
            Ok(rows) => rows,
            Err(error) if query_embedding.is_some() => {
                tracing::warn!(%error, "semantic source retrieval failed; using lexical retrieval");
                warning = Some("semantic retrieval unavailable; using lexical retrieval");
                rank_with_tuning(
                    store,
                    query,
                    None,
                    project,
                    Some(source),
                    result_limit,
                    principal_acl,
                    RetrievalTuning::default(),
                )?
            }
            Err(error) => return Err(error),
        };
        diagnostics.candidate_limit = diagnostics.candidate_limit.max(rows.1.candidate_limit);
        diagnostics.semantic_candidates += rows.1.semantic_candidates;
        diagnostics.lexical_candidates += rows.1.lexical_candidates;
        for (rank, evidence) in rows.0.into_iter().enumerate() {
            let key = evidence_dedupe_key(&evidence);
            let reciprocal_rank = 1.0 / (60.0 + rank as f32 + 1.0);
            fused
                .entry(key)
                .and_modify(|(_, score)| *score += reciprocal_rank)
                .or_insert((evidence, reciprocal_rank));
        }
    }
    let mut rows = fused
        .into_values()
        .map(|(mut evidence, fused_score)| {
            evidence.score = fused_score;
            evidence
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    let fused_candidates = rows.len();
    rows.truncate(result_limit);
    diagnostics.fused_candidates = fused_candidates;
    diagnostics.returned = rows.len();
    Ok(RetrievalOutcome {
        evidence: rows,
        mode: if warning.is_some() {
            RetrievalMode::LexicalFallback
        } else {
            RetrievalMode::Hybrid
        },
        warning: warning.map(str::to_string),
        diagnostics,
    })
}

// The public tuned entry point mirrors the stable retrieval call shape while
// accepting the bounded policy inputs needed by evaluation and configuration.
#[allow(clippy::too_many_arguments)]
pub async fn retrieve_scoped_with_status_tuned(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    principal_acl: &[String],
    tuning: RetrievalTuning,
) -> Result<RetrievalOutcome> {
    retrieve_with_timeout_status(
        store,
        embedder,
        query,
        project,
        source,
        limit,
        INTERACTIVE_EMBEDDING_TIMEOUT,
        principal_acl,
        tuning,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn retrieve_with_timeout_status(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    embedding_timeout: Duration,
    principal_acl: &[String],
    tuning: RetrievalTuning,
) -> Result<RetrievalOutcome> {
    validate_query(query)?;
    let (query_embedding, mut warning) = query_embedding(embedder, query, embedding_timeout).await;
    let tuning = tuning.bounded();
    let (evidence, diagnostics) = match rank_with_tuning(
        store,
        query,
        query_embedding.as_deref(),
        project,
        source,
        limit.min(MAX_RESULT_LIMIT),
        principal_acl,
        tuning,
    ) {
        Ok(result) => result,
        Err(error) if query_embedding.is_some() => {
            tracing::warn!(%error, "semantic retrieval failed; using lexical retrieval");
            warning = Some("semantic retrieval unavailable; using lexical retrieval");
            let (evidence, diagnostics) = rank_with_tuning(
                store,
                query,
                None,
                project,
                source,
                limit.min(MAX_RESULT_LIMIT),
                principal_acl,
                tuning,
            )?;
            (evidence, diagnostics)
        }
        Err(error) => return Err(error),
    };
    let mode = if warning.is_some() {
        RetrievalMode::LexicalFallback
    } else {
        RetrievalMode::Hybrid
    };
    Ok(RetrievalOutcome {
        evidence,
        mode,
        warning: warning.map(str::to_string),
        diagnostics,
    })
}

fn validate_query(query: &str) -> Result<()> {
    anyhow::ensure!(!query.trim().is_empty(), "query must not be empty");
    anyhow::ensure!(
        query.len() <= MAX_QUERY_BYTES,
        "query exceeds {MAX_QUERY_BYTES} bytes"
    );
    Ok(())
}

async fn query_embedding(
    embedder: &Arc<dyn Embedder>,
    query: &str,
    embedding_timeout: Duration,
) -> (Option<Vec<f32>>, Option<&'static str>) {
    // Keep the lexical query untouched, but collapse insignificant whitespace
    // before embedding so equivalent UI/MCP inputs reuse one cache entry.
    let embedding_query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    match tokio::time::timeout(embedding_timeout, embedder.embed(&[embedding_query])).await {
        Ok(Ok(vectors)) => match vectors.into_iter().next() {
            Some(vector) if !vector.is_empty() && vector.iter().all(|value| value.is_finite()) => {
                (Some(vector), None)
            }
            None => {
                tracing::warn!("query embedding returned no vector; using lexical retrieval");
                (
                    None,
                    Some("query embedding unavailable; using lexical retrieval"),
                )
            }
            Some(_) => {
                tracing::warn!(
                    "query embedding returned an invalid vector; using lexical retrieval"
                );
                (
                    None,
                    Some("query embedding unavailable; using lexical retrieval"),
                )
            }
        },
        Ok(Err(error)) => {
            tracing::warn!(%error, "query embedding unavailable; using lexical retrieval");
            (
                None,
                Some("query embedding unavailable; using lexical retrieval"),
            )
        }
        Err(_) => {
            tracing::warn!(
                timeout_seconds = embedding_timeout.as_secs_f32(),
                "query embedding saturated; using lexical retrieval"
            );
            (
                None,
                Some("query embedding timed out; using lexical retrieval"),
            )
        }
    }
}

pub fn search(
    store: &Store,
    query: &str,
    query_embedding: &[f32],
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
) -> Result<Vec<Evidence>> {
    rank(
        store,
        query,
        Some(query_embedding),
        project,
        source,
        limit,
        &["*".into()],
    )
}

fn rank(
    store: &Store,
    query: &str,
    query_embedding: Option<&[f32]>,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    principal_acl: &[String],
) -> Result<Vec<Evidence>> {
    Ok(rank_with_tuning(
        store,
        query,
        query_embedding,
        project,
        source,
        limit,
        principal_acl,
        RetrievalTuning::default(),
    )?
    .0)
}

// Keep the low-level ranking helper explicit: each scope and ranking input is
// independently audited and the call sites remain easy to compare in tests.
#[allow(clippy::too_many_arguments)]
fn rank_with_tuning(
    store: &Store,
    query: &str,
    query_embedding: Option<&[f32]>,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    principal_acl: &[String],
    tuning: RetrievalTuning,
) -> Result<(Vec<Evidence>, RetrievalDiagnostics)> {
    let tuning = tuning.bounded();
    let candidate_limit = limit.saturating_mul(tuning.candidate_multiplier).max(32);
    let semantic = query_embedding.map_or_else(
        || Ok(Vec::new()),
        |embedding| {
            store.semantic_ids_scoped(embedding, project, source, candidate_limit, principal_acl)
        },
    )?;
    let lexical =
        store.lexical_ids_scoped(query, project, source, candidate_limit, principal_acl)?;
    let candidate_ids = semantic
        .iter()
        .map(|(id, _)| id.clone())
        .chain(lexical.iter().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let chunks = store.chunks_by_ids_scoped(&candidate_ids, principal_acl)?;
    let semantic_ranks = semantic
        .iter()
        .enumerate()
        .map(|(rank, (id, _))| (id.clone(), rank + 1))
        .collect::<HashMap<_, _>>();
    let lexical_ranks = lexical
        .iter()
        .enumerate()
        .map(|(rank, id)| (id.clone(), rank + 1))
        .collect::<HashMap<_, _>>();
    let semantic_candidate_count = semantic_ranks.len();
    let lexical_candidate_count = lexical_ranks.len();
    let by_id = chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect::<HashMap<_, _>>();
    let mut scores = HashMap::<String, f32>::new();
    for (id, rank) in &semantic_ranks {
        *scores.entry(id.clone()).or_default() += tuning.semantic_weight / (60.0 + *rank as f32);
    }
    for (id, rank) in &lexical_ranks {
        *scores.entry(id.clone()).or_default() += tuning.lexical_weight / (60.0 + *rank as f32);
    }
    let now = Utc::now();
    let idf_scores = idf_overlap(query, &chunks);
    let semantic_scores = semantic.into_iter().collect::<HashMap<_, _>>();
    let fused_candidate_count = scores.len();
    let mut ranked = scores
        .into_iter()
        .filter_map(|(id, score)| {
            let chunk = by_id.get(id.as_str())?;
            let age_days = (now - chunk.updated_at).num_days().max(0) as f32;
            let recency = 1.0 / (1.0 + age_days / 180.0);
            let semantic = semantic_scores.get(&id).copied().unwrap_or_default();
            let idf = idf_scores.get(&id).copied().unwrap_or_default();
            let reranked = score + 0.01 * semantic.max(0.0) + tuning.idf_weight * idf;
            let recency_adjusted =
                reranked * (1.0 - tuning.recency_weight + tuning.recency_weight * recency);
            let score = if tuning.reranker_enabled {
                apply_local_reranker(query, chunk, recency_adjusted)
            } else {
                recency_adjusted
            };
            Some((chunk, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            // HashMap-backed candidate collection has no stable iteration
            // order. Keep equal-score results deterministic for UI state,
            // answer caching, and agent replayability.
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    let ranked_candidate_count = ranked.len();
    let mut seen_documents = HashSet::new();
    let deduplicated = ranked
        .into_iter()
        .filter(|(chunk, _)| seen_documents.insert(dedupe_key(chunk)))
        .collect::<Vec<_>>();
    let deduplicated_candidates = ranked_candidate_count.saturating_sub(deduplicated.len());
    let selected = deduplicated.into_iter().take(limit).collect::<Vec<_>>();
    let selected_ids = selected
        .iter()
        .map(|(chunk, _)| chunk.id.clone())
        .collect::<Vec<_>>();
    let neighboring_content = store.neighboring_content_scoped(
        &selected_ids,
        NEIGHBOR_RADIUS,
        MAX_EXPANDED_CONTENT_BYTES,
        principal_acl,
    )?;
    let evidence = selected
        .into_iter()
        .map(|(chunk, score)| {
            evidence(
                chunk,
                neighboring_content.get(&chunk.id),
                score,
                &semantic_ranks,
                &lexical_ranks,
            )
        })
        .collect::<Vec<_>>();
    let returned = evidence.len();
    Ok((
        evidence,
        RetrievalDiagnostics {
            contract_version: RETRIEVAL_RANKING_VERSION,
            candidate_limit,
            semantic_candidates: semantic_candidate_count,
            lexical_candidates: lexical_candidate_count,
            fused_candidates: fused_candidate_count,
            deduplicated_candidates,
            returned,
        },
    ))
}

/// Apply a small deterministic second-pass score to the bounded candidate set.
/// This deliberately uses only local title/content terms, so enabling it never
/// introduces provider latency, secrets, or a new failure mode. The boost is
/// capped and is included in the ranking version/cache key through tuning.
fn apply_local_reranker(query: &str, chunk: &StoredChunk, score: f32) -> f32 {
    let normalized_query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized_query.is_empty() {
        return score;
    }
    let title = chunk.title.to_ascii_lowercase();
    let content = chunk.content.to_ascii_lowercase();
    let query_lower = normalized_query.to_ascii_lowercase();
    let phrase_boost = if content.contains(&query_lower) {
        0.08
    } else {
        0.0
    };
    let title_boost = if title.contains(&query_lower) {
        0.06
    } else {
        0.0
    };
    let exact_term_count = tokenize(&normalized_query)
        .into_iter()
        .filter(|term| title.contains(term) || content.contains(term))
        .count()
        .min(16);
    let coverage_boost = (exact_term_count as f32 * 0.01).min(0.16);
    score + phrase_boost + title_boost + coverage_boost
}

fn dedupe_key(chunk: &StoredChunk) -> (&str, &str) {
    (
        chunk.source.as_str(),
        chunk.uri.as_deref().unwrap_or(chunk.source_id.as_str()),
    )
}

fn evidence_dedupe_key(evidence: &Evidence) -> String {
    format!(
        "{}\u{0}{}",
        evidence.source,
        evidence.uri.as_deref().unwrap_or(&evidence.source_id)
    )
}

fn idf_overlap(query: &str, chunks: &[StoredChunk]) -> HashMap<String, f32> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() || chunks.is_empty() {
        return HashMap::new();
    }
    let chunk_terms = chunks
        .iter()
        .map(|chunk| tokenize(&format!("{} {}", chunk.title, chunk.content)))
        .collect::<Vec<_>>();
    let mut document_frequency = HashMap::<&str, usize>::new();
    for term in &query_terms {
        document_frequency.insert(
            term,
            chunk_terms
                .iter()
                .filter(|terms| terms.contains(term))
                .count(),
        );
    }
    let count = chunks.len() as f32;
    let weights = query_terms
        .iter()
        .map(|term| {
            let frequency = *document_frequency.get(term.as_str()).unwrap_or(&0) as f32;
            (term, ((count + 1.0) / (frequency + 1.0)).ln() + 1.0)
        })
        .collect::<Vec<_>>();
    let denominator = weights.iter().map(|(_, weight)| *weight).sum::<f32>();
    chunks
        .iter()
        .zip(chunk_terms)
        .map(|(chunk, terms)| {
            let matched = weights
                .iter()
                .filter(|(term, _)| terms.contains(*term))
                .map(|(_, weight)| *weight)
                .sum::<f32>();
            (chunk.id.clone(), matched / denominator)
        })
        .collect()
}

fn tokenize(value: &str) -> HashSet<String> {
    const STOPWORDS: [&str; 32] = [
        "a", "an", "and", "are", "be", "can", "did", "do", "does", "for", "from", "how", "i", "in",
        "is", "it", "my", "of", "on", "or", "our", "should", "the", "this", "to", "was", "were",
        "what", "when", "with", "you", "your",
    ];
    value
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|token| token.len() > 1)
        .map(str::to_lowercase)
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .map(|token| {
            if token.len() > 4 && token.ends_with("ly") {
                token[..token.len() - 2].to_string()
            } else {
                token
            }
        })
        .collect()
}

fn evidence(
    chunk: &StoredChunk,
    expanded_content: Option<&String>,
    score: f32,
    semantic: &HashMap<String, usize>,
    lexical: &HashMap<String, usize>,
) -> Evidence {
    Evidence {
        chunk_id: chunk.id.clone(),
        source: chunk.source.clone(),
        source_id: chunk.source_id.clone(),
        title: chunk.title.clone(),
        uri: chunk.uri.clone(),
        content: expanded_content
            .cloned()
            .unwrap_or_else(|| chunk.content.clone()),
        score,
        semantic_rank: semantic.get(&chunk.id).copied(),
        lexical_rank: lexical.get(&chunk.id).copied(),
        updated_at: chunk.updated_at,
        metadata: chunk.metadata.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use anyhow::bail;
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;
    use crate::model::Document;

    struct UnavailableEmbedder;
    struct InvalidEmbedder;
    struct SlowEmbedder;
    struct CountingEmbedder(Arc<AtomicUsize>);
    struct RecordingEmbedder(Arc<Mutex<Vec<String>>>);

    #[async_trait]
    impl Embedder for UnavailableEmbedder {
        fn fingerprint(&self) -> String {
            "unavailable:2".into()
        }

        async fn embed(&self, _input: &[String]) -> Result<Vec<Vec<f32>>> {
            bail!("embedding backend unavailable")
        }
    }

    #[async_trait]
    impl Embedder for InvalidEmbedder {
        fn fingerprint(&self) -> String {
            "invalid:0".into()
        }

        async fn embed(&self, _input: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(vec![vec![f32::NAN]])
        }
    }

    #[async_trait]
    impl Embedder for SlowEmbedder {
        fn fingerprint(&self) -> String {
            "slow:2".into()
        }

        async fn embed(&self, _input: &[String]) -> Result<Vec<Vec<f32>>> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(vec![vec![1.0, 0.0]])
        }
    }

    #[async_trait]
    impl Embedder for CountingEmbedder {
        fn fingerprint(&self) -> String {
            "counting:2".into()
        }

        async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(input.iter().map(|_| vec![1.0, 0.0]).collect())
        }
    }

    #[async_trait]
    impl Embedder for RecordingEmbedder {
        fn fingerprint(&self) -> String {
            "recording:2".into()
        }

        async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
            self.0
                .lock()
                .expect("recording lock")
                .extend(input.iter().cloned());
            Ok(input.iter().map(|_| vec![1.0, 0.0]).collect())
        }
    }

    fn chunk(id: &str, title: &str, content: &str) -> StoredChunk {
        StoredChunk {
            id: id.into(),
            source: "test".into(),
            source_id: id.into(),
            title: title.into(),
            uri: None,
            content: content.into(),
            acl: Vec::new(),
            embedding: vec![1.0, 0.0],
            updated_at: Utc::now(),
            metadata: serde_json::Value::Null,
            strategy: None,
            parent_key: None,
            previous_key: None,
            next_key: None,
        }
    }

    #[test]
    fn idf_overlap_rewards_rare_exact_terms() {
        let chunks = vec![
            chunk("specific", "Qwen setup", "qwen embeddings cache"),
            chunk("general", "Embedding setup", "embeddings cache"),
            chunk("other", "Unrelated", "release checklist"),
        ];
        let scores = idf_overlap("qwen embeddings", &chunks);
        assert!(scores["specific"] > scores["general"]);
        assert_eq!(scores["other"], 0.0);
    }

    #[test]
    fn canonical_uri_deduplicates_source_records() {
        let mut first = chunk("message-1", "First", "same thread");
        first.source = "gmail".into();
        first.uri = Some("https://mail.google.com/mail/u/0/#all/thread-1".into());
        let mut second = chunk("message-2", "Second", "same thread");
        second.source = "gmail".into();
        second.uri.clone_from(&first.uri);

        assert_eq!(dedupe_key(&first), dedupe_key(&second));
    }

    #[tokio::test]
    async fn unavailable_embeddings_fall_back_to_lexical_evidence() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let document = Document {
            source: "notes".into(),
            source_id: "qwen-runbook".into(),
            title: "Qwen runbook".into(),
            content: "Cortana owns the Qwen embedding service.".into(),
            uri: None,
            updated_at: Utc::now(),
            project: "cortana".into(),
            acl: Vec::new(),
            metadata: json!({}),
        };
        store
            .upsert(
                &document,
                &[(
                    "Cortana owns the Qwen embedding service.".into(),
                    vec![1.0, 0.0],
                )],
            )
            .expect("upsert");

        let outcome = retrieve_scoped_with_status(
            &store,
            &(Arc::new(UnavailableEmbedder) as Arc<dyn Embedder>),
            "Qwen",
            Some("cortana"),
            None,
            10,
            &["*".into()],
        )
        .await
        .expect("lexical fallback");

        assert_eq!(outcome.mode, RetrievalMode::LexicalFallback);
        assert_eq!(
            outcome.warning.as_deref(),
            Some("query embedding unavailable; using lexical retrieval")
        );
        let evidence = outcome.evidence;
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].title, "Qwen runbook");
        assert_eq!(evidence[0].semantic_rank, None);
        assert_eq!(evidence[0].lexical_rank, Some(1));
    }

    #[tokio::test]
    async fn selected_evidence_includes_bounded_neighboring_context() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let document = Document {
            source: "notes".into(),
            source_id: "decision-record".into(),
            title: "Architecture decision".into(),
            content: "before context decision phrase after context".into(),
            uri: None,
            updated_at: Utc::now(),
            project: "cortana".into(),
            acl: vec!["work".into()],
            metadata: json!({}),
        };
        store
            .upsert(
                &document,
                &[
                    ("before context".into(), vec![1.0, 0.0]),
                    ("decision phrase".into(), vec![1.0, 0.0]),
                    ("after context".into(), vec![1.0, 0.0]),
                ],
            )
            .expect("upsert");

        let evidence = retrieve_scoped(
            &store,
            &(Arc::new(UnavailableEmbedder) as Arc<dyn Embedder>),
            "decision phrase",
            Some("cortana"),
            None,
            10,
            &["work".into()],
        )
        .await
        .expect("expanded lexical evidence");

        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].content,
            "before context\n\ndecision phrase\n\nafter context"
        );
    }

    #[tokio::test]
    async fn multi_source_retrieval_embeds_once_and_fuses_scoped_results() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        for source in ["code-work", "code-personal"] {
            let document = Document {
                source: source.into(),
                source_id: format!("{source}-runbook"),
                title: format!("{source} runbook"),
                content: "shared symbol implementation".into(),
                uri: Some(format!("file:///{source}/runbook.rs")),
                updated_at: Utc::now(),
                project: "cortana".into(),
                acl: vec!["work".into()],
                metadata: json!({}),
            };
            store
                .upsert(
                    &document,
                    &[("shared symbol implementation".into(), vec![1.0, 0.0])],
                )
                .expect("upsert");
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder: Arc<dyn Embedder> = Arc::new(CountingEmbedder(calls.clone()));

        let empty = retrieve_sources_scoped(
            &store,
            &embedder,
            "shared symbol",
            Some("cortana"),
            &[],
            10,
            &["work".into()],
        )
        .await
        .expect("empty source group");
        assert!(empty.is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 0);

        let evidence = retrieve_sources_scoped(
            &store,
            &embedder,
            "shared symbol",
            Some("cortana"),
            &["code-personal".into(), "code-work".into()],
            10,
            &["work".into()],
        )
        .await
        .expect("multi-source retrieval");

        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(evidence.len(), 2);
        assert!(evidence.iter().all(|item| item.score > 0.0));
    }

    #[tokio::test]
    async fn query_embedding_collapses_insignificant_whitespace_for_cache_reuse() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let queries = Arc::new(Mutex::new(Vec::new()));
        let embedder: Arc<dyn Embedder> = Arc::new(RecordingEmbedder(queries.clone()));

        retrieve_sources_scoped(
            &store,
            &embedder,
            "  release\n\tplaybook  ",
            None,
            &["notes".into()],
            10,
            &["*".into()],
        )
        .await
        .expect("retrieval with no indexed rows");

        assert_eq!(
            queries.lock().expect("recording lock").as_slice(),
            ["release playbook"]
        );
    }

    #[tokio::test]
    async fn oversized_queries_fail_before_calling_the_embedding_provider() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder: Arc<dyn Embedder> = Arc::new(CountingEmbedder(calls.clone()));

        let error = retrieve_scoped(
            &store,
            &embedder,
            &"x".repeat(MAX_QUERY_BYTES + 1),
            None,
            None,
            10,
            &["*".into()],
        )
        .await
        .expect_err("oversized query");

        assert!(error.to_string().contains("query exceeds"));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn saturated_embeddings_respect_the_interactive_latency_budget() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let document = Document {
            source: "notes".into(),
            source_id: "release-runbook".into(),
            title: "Release runbook".into(),
            content: "Use the blue green release procedure.".into(),
            uri: None,
            updated_at: Utc::now(),
            project: "cortana".into(),
            acl: Vec::new(),
            metadata: json!({}),
        };
        store
            .upsert(
                &document,
                &[(
                    "Use the blue green release procedure.".into(),
                    vec![1.0, 0.0],
                )],
            )
            .expect("upsert");

        let evidence = retrieve_with_timeout_status(
            &store,
            &(Arc::new(SlowEmbedder) as Arc<dyn Embedder>),
            "blue green",
            Some("cortana"),
            None,
            10,
            Duration::from_millis(1),
            &["*".into()],
            RetrievalTuning::default(),
        )
        .await
        .expect("timeout fallback")
        .evidence;

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].semantic_rank, None);
        assert_eq!(evidence[0].lexical_rank, Some(1));
    }

    #[tokio::test]
    async fn invalid_embedding_vectors_fall_back_without_failing_the_query() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let document = Document {
            source: "notes".into(),
            source_id: "invalid-vector-runbook".into(),
            title: "Invalid vector runbook".into(),
            content: "Lexical fallback remains available.".into(),
            uri: None,
            updated_at: Utc::now(),
            project: "cortana".into(),
            acl: Vec::new(),
            metadata: json!({}),
        };
        store
            .upsert(
                &document,
                &[("Lexical fallback remains available.".into(), vec![1.0, 0.0])],
            )
            .expect("upsert");

        let outcome = retrieve_scoped_with_status(
            &store,
            &(Arc::new(InvalidEmbedder) as Arc<dyn Embedder>),
            "fallback",
            Some("cortana"),
            None,
            10,
            &["*".into()],
        )
        .await
        .expect("invalid vector fallback");

        assert_eq!(outcome.mode, RetrievalMode::LexicalFallback);
        assert_eq!(outcome.evidence.len(), 1);
        assert_eq!(outcome.evidence[0].semantic_rank, None);
        assert_eq!(outcome.evidence[0].lexical_rank, Some(1));
        assert!(outcome.diagnostics.candidate_limit >= 32);
    }

    #[test]
    fn tuning_is_bounded_and_cache_safe() {
        let tuning = RetrievalTuning {
            candidate_multiplier: 999,
            semantic_weight: -1.0,
            lexical_weight: 9.0,
            idf_weight: f32::NAN,
            recency_weight: 2.0,
            reranker_enabled: true,
        }
        .bounded();
        assert_eq!(tuning.candidate_multiplier, 32);
        assert_eq!(tuning.semantic_weight, 0.0);
        assert_eq!(tuning.lexical_weight, 4.0);
        assert_eq!(tuning.idf_weight, 0.08);
        assert_eq!(tuning.recency_weight, 1.0);
        assert!(tuning.reranker_enabled);
    }

    #[test]
    fn local_reranker_is_bounded_and_prefers_exact_title_phrase() {
        let exact = chunk(
            "exact",
            "Qwen embedding setup",
            "Configure the Qwen embedding cache.",
        );
        let distractor = chunk(
            "distractor",
            "Embedding notes",
            "General provider guidance.",
        );
        let base = 0.1;
        assert!(
            apply_local_reranker("Qwen embedding setup", &exact, base)
                > apply_local_reranker("Qwen embedding setup", &distractor, base)
        );
        assert!(apply_local_reranker("Qwen embedding setup", &exact, base) < base + 0.31);
    }

    #[tokio::test]
    async fn scoped_retrieval_filters_both_semantic_and_lexical_candidates() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let mut private = Document {
            source: "notes".into(),
            source_id: "private".into(),
            title: "Private launch".into(),
            content: "private launch sequence".into(),
            uri: None,
            updated_at: Utc::now(),
            project: "demo".into(),
            acl: vec!["personal".into()],
            metadata: json!({}),
        };
        store
            .upsert(
                &private,
                &[("private launch sequence".into(), vec![1.0, 0.0])],
            )
            .expect("private document");
        private.source_id = "public".into();
        private.title = "Public launch".into();
        private.content = "public launch checklist".into();
        private.acl.clear();
        store
            .upsert(
                &private,
                &[("public launch checklist".into(), vec![1.0, 0.0])],
            )
            .expect("public document");

        let work = retrieve_scoped(
            &store,
            &(Arc::new(UnavailableEmbedder) as Arc<dyn Embedder>),
            "launch",
            Some("demo"),
            None,
            10,
            &["work".into()],
        )
        .await
        .expect("work retrieval");
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].source_id, "public");

        let personal = retrieve_scoped(
            &store,
            &(Arc::new(UnavailableEmbedder) as Arc<dyn Embedder>),
            "launch",
            Some("demo"),
            None,
            10,
            &["personal".into()],
        )
        .await
        .expect("personal retrieval");
        assert_eq!(personal.len(), 2);
    }
}
