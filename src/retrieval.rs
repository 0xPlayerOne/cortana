use std::collections::HashMap;

use anyhow::Result;
use chrono::Utc;

use crate::model::{Evidence, StoredChunk};
use crate::store::Store;

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
    let lexical = store.lexical_ids(query, limit.saturating_mul(4))?;
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
    let mut ranked = scores
        .into_iter()
        .filter_map(|(id, score)| {
            let chunk = by_id.get(id.as_str())?;
            let age_days = (now - chunk.updated_at).num_days().max(0) as f32;
            let recency = 1.0 / (1.0 + age_days / 180.0);
            Some((chunk, score * (0.9 + 0.1 * recency)))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(ranked
        .into_iter()
        .take(limit)
        .map(|(chunk, score)| evidence(chunk, score, &semantic_ranks, &lexical_ranks))
        .collect())
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
