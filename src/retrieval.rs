use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::embed::Embedder;
use crate::model::{Evidence, StoredChunk};
use crate::store::Store;

pub async fn retrieve(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
) -> Result<Vec<Evidence>> {
    anyhow::ensure!(!query.trim().is_empty(), "query must not be empty");
    let vectors = embedder.embed(&[query.to_string()]).await?;
    let query_embedding = vectors
        .first()
        .ok_or_else(|| anyhow::anyhow!("embedding provider returned no query vector"))?;
    search(
        store,
        query,
        query_embedding,
        project,
        source,
        limit.min(50),
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
    let chunks = store.all_chunks(project, source)?;
    let mut semantic = chunks
        .iter()
        .map(|chunk| (chunk.id.clone(), cosine(query_embedding, &chunk.embedding)))
        .collect::<Vec<_>>();
    semantic.sort_by(|a, b| b.1.total_cmp(&a.1));
    let lexical = store.lexical_ids(query, project, source, limit.saturating_mul(4))?;
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
            let reranked = score + 0.01 * semantic.max(0.0) + 0.015 * idf;
            Some((chunk, reranked * (0.9 + 0.1 * recency)))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut seen_documents = HashSet::new();
    Ok(ranked
        .into_iter()
        .filter(|(chunk, _)| {
            seen_documents.insert((chunk.source.as_str(), chunk.source_id.as_str()))
        })
        .take(limit)
        .map(|(chunk, score)| evidence(chunk, score, &semantic_ranks, &lexical_ranks))
        .collect())
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
    value
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|token| token.len() > 1)
        .map(str::to_lowercase)
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

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|v| v * v).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|v| v * v).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn chunk(id: &str, title: &str, content: &str) -> StoredChunk {
        StoredChunk {
            id: id.into(),
            source: "test".into(),
            source_id: id.into(),
            title: title.into(),
            uri: None,
            content: content.into(),
            embedding: vec![1.0, 0.0],
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn cosine_handles_normal_and_invalid_vectors() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < f32::EPSILON);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
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
}
