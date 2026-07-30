use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;

use crate::embed::Embedder;
use crate::model::{Evidence, StoredChunk};
use crate::store::Store;

const INTERACTIVE_EMBEDDING_TIMEOUT: Duration = Duration::from_secs(5);

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
    retrieve_with_timeout(
        store,
        embedder,
        query,
        project,
        source,
        limit,
        INTERACTIVE_EMBEDDING_TIMEOUT,
        principal_acl,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn retrieve_with_timeout(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    embedding_timeout: Duration,
    principal_acl: &[String],
) -> Result<Vec<Evidence>> {
    anyhow::ensure!(!query.trim().is_empty(), "query must not be empty");
    let query_embedding =
        match tokio::time::timeout(embedding_timeout, embedder.embed(&[query.to_string()])).await {
            Ok(Ok(vectors)) => vectors.into_iter().next(),
            Ok(Err(error)) => {
                tracing::warn!(%error, "query embedding unavailable; using lexical retrieval");
                None
            }
            Err(_) => {
                tracing::warn!(
                    timeout_seconds = embedding_timeout.as_secs_f32(),
                    "query embedding saturated; using lexical retrieval"
                );
                None
            }
        };
    rank(
        store,
        query,
        query_embedding.as_deref(),
        project,
        source,
        limit.min(50),
        principal_acl,
    )
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
    let candidate_limit = limit.saturating_mul(8).max(32);
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
    let by_id = chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect::<HashMap<_, _>>();
    let mut scores = HashMap::<String, f32>::new();
    for (id, rank) in &semantic_ranks {
        *scores.entry(id.clone()).or_default() += 1.0 / (60.0 + *rank as f32);
    }
    for (id, rank) in &lexical_ranks {
        *scores.entry(id.clone()).or_default() += 1.2 / (60.0 + *rank as f32);
    }
    let now = Utc::now();
    let idf_scores = idf_overlap(query, &chunks);
    let semantic_scores = semantic.into_iter().collect::<HashMap<_, _>>();
    let mut ranked = scores
        .into_iter()
        .filter_map(|(id, score)| {
            let chunk = by_id.get(id.as_str())?;
            let age_days = (now - chunk.updated_at).num_days().max(0) as f32;
            let recency = 1.0 / (1.0 + age_days / 180.0);
            let semantic = semantic_scores.get(&id).copied().unwrap_or_default();
            let idf = idf_scores.get(&id).copied().unwrap_or_default();
            let reranked = score + 0.01 * semantic.max(0.0) + 0.08 * idf;
            Some((chunk, reranked * (0.9 + 0.1 * recency)))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut seen_documents = HashSet::new();
    Ok(ranked
        .into_iter()
        .filter(|(chunk, _)| seen_documents.insert(dedupe_key(chunk)))
        .take(limit)
        .map(|(chunk, score)| evidence(chunk, score, &semantic_ranks, &lexical_ranks))
        .collect())
}

fn dedupe_key(chunk: &StoredChunk) -> (&str, &str) {
    (
        chunk.source.as_str(),
        chunk.uri.as_deref().unwrap_or(chunk.source_id.as_str()),
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
        content: chunk.content.clone(),
        score,
        semantic_rank: semantic.get(&chunk.id).copied(),
        lexical_rank: lexical.get(&chunk.id).copied(),
        updated_at: chunk.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::bail;
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;
    use crate::model::Document;

    struct UnavailableEmbedder;
    struct SlowEmbedder;

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
    impl Embedder for SlowEmbedder {
        fn fingerprint(&self) -> String {
            "slow:2".into()
        }

        async fn embed(&self, _input: &[String]) -> Result<Vec<Vec<f32>>> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(vec![vec![1.0, 0.0]])
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

        let evidence = retrieve(
            &store,
            &(Arc::new(UnavailableEmbedder) as Arc<dyn Embedder>),
            "Qwen",
            Some("cortana"),
            None,
            10,
        )
        .await
        .expect("lexical fallback");

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].title, "Qwen runbook");
        assert_eq!(evidence[0].semantic_rank, None);
        assert_eq!(evidence[0].lexical_rank, Some(1));
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

        let evidence = retrieve_with_timeout(
            &store,
            &(Arc::new(SlowEmbedder) as Arc<dyn Embedder>),
            "blue green",
            Some("cortana"),
            None,
            10,
            Duration::from_millis(1),
            &["*".into()],
        )
        .await
        .expect("timeout fallback");

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].semantic_rank, None);
        assert_eq!(evidence[0].lexical_rank, Some(1));
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
