use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::EmbeddingConfig;

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>>;
    fn fingerprint(&self) -> String;
}

#[derive(Clone)]
pub struct OpenAiEmbedder {
    client: Client,
    config: EmbeddingConfig,
}

impl OpenAiEmbedder {
    pub fn new(config: EmbeddingConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedItem>,
}

#[derive(Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));
        let mut request = self.client.post(url).json(&EmbedRequest {
            model: &self.config.model,
            input,
        });
        if let Some(name) = &self.config.api_key_env {
            let key = std::env::var(name).with_context(|| format!("{name} is not set"))?;
            request = request.bearer_auth(key);
        }
        let response = request.send().await?.error_for_status()?;
        let vectors = response.json::<EmbedResponse>().await?.data;
        if vectors.len() != input.len()
            || vectors
                .iter()
                .any(|item| item.embedding.len() != self.config.dimension)
        {
            bail!("embedding response count or dimension mismatch");
        }
        Ok(vectors.into_iter().map(|item| item.embedding).collect())
    }

    fn fingerprint(&self) -> String {
        format!("{}:{}", self.config.model, self.config.dimension)
    }
}

#[derive(Clone)]
pub struct DeterministicEmbedder {
    dimension: usize,
}

impl DeterministicEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

#[async_trait]
impl Embedder for DeterministicEmbedder {
    async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(input
            .iter()
            .map(|text| {
                let mut values = vec![0.0; self.dimension];
                for token in text.split_whitespace() {
                    let digest = Sha256::digest(token.to_lowercase().as_bytes());
                    let index =
                        u16::from_le_bytes([digest[0], digest[1]]) as usize % self.dimension;
                    values[index] += 1.0;
                }
                normalize(values)
            })
            .collect())
    }

    fn fingerprint(&self) -> String {
        format!("deterministic:{}", self.dimension)
    }
}

fn normalize(mut values: Vec<f32>) -> Vec<f32> {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        values.iter_mut().for_each(|value| *value /= norm);
    }
    values
}
