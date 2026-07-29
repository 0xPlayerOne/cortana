use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Document {
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub content: String,
    pub uri: Option<String>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub acl: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Evidence {
    pub chunk_id: String,
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub uri: Option<String>,
    pub content: String,
    pub score: f32,
    pub semantic_rank: Option<usize>,
    pub lexical_rank: Option<usize>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct StoredChunk {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub uri: Option<String>,
    pub content: String,
    pub embedding: Vec<f32>,
    pub updated_at: DateTime<Utc>,
}
