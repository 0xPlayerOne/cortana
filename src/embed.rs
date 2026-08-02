use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use futures_util::future::join_all;
use reqwest::{Client, StatusCode, header::RETRY_AFTER};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, watch};

use crate::config::{EmbeddingConfig, validate_provider_base_url};
use crate::store::Store;

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>>;
    fn fingerprint(&self) -> String;
    fn request_concurrency(&self) -> usize {
        1
    }

    async fn probe(&self) -> Result<()> {
        let vectors = self.embed(&["__cortana_probe__".into()]).await?;
        anyhow::ensure!(
            vectors.first().is_some_and(|vector| !vector.is_empty()),
            "embedding provider returned no probe vector"
        );
        Ok(())
    }
}

pub struct CachedEmbedder {
    inner: Arc<dyn Embedder>,
    store: Store,
    max_entries: usize,
    inflight: Arc<AsyncMutex<HashMap<String, watch::Sender<bool>>>>,
}

impl CachedEmbedder {
    pub fn new(store: Store, inner: Arc<dyn Embedder>) -> Self {
        Self::with_limit(store, inner, 250_000)
    }

    pub fn with_limit(store: Store, inner: Arc<dyn Embedder>, max_entries: usize) -> Self {
        Self {
            inner,
            store,
            max_entries,
            inflight: Arc::new(AsyncMutex::new(HashMap::new())),
        }
    }

    async fn embed_single_cached(&self, text: &str) -> Result<Vec<f32>> {
        let fingerprint = self.fingerprint();
        let key = format!("{fingerprint}\u{0}{text}");
        loop {
            if let Some(vector) = self.store.cached_embedding(&fingerprint, text)? {
                return Ok(vector);
            }

            let (leader, mut ready) = {
                let mut inflight = self.inflight.lock().await;
                if let Some(sender) = inflight.get(&key) {
                    (false, sender.subscribe())
                } else {
                    let (sender, receiver) = watch::channel(false);
                    inflight.insert(key.clone(), sender);
                    (true, receiver)
                }
            };
            if !leader {
                let _ = ready.wait_for(|value| *value).await;
                continue;
            }

            let result = async {
                let vectors = self.inner.embed(&[text.to_string()]).await?;
                anyhow::ensure!(
                    vectors.len() == 1,
                    "embedding provider returned an unexpected vector count"
                );
                let vector = vectors
                    .into_iter()
                    .next()
                    .context("embedding provider returned no vector")?;
                if self.max_entries > 0 {
                    if !self
                        .store
                        .cache_embedding_if_available(&fingerprint, text, &vector)?
                    {
                        tracing::warn!(
                            "embedding cache write skipped because another index writer is active"
                        );
                    }
                    self.store.prune_embedding_cache(self.max_entries)?;
                }
                Ok(vector)
            }
            .await;

            let mut inflight = self.inflight.lock().await;
            if let Some(sender) = inflight.remove(&key) {
                let _ = sender.send(true);
            }
            drop(inflight);
            return result;
        }
    }

    async fn embed_batch_cached(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
        let fingerprint = self.fingerprint();
        let mut resolved = HashMap::<String, Vec<f32>>::new();
        let mut output = vec![None; input.len()];

        loop {
            let mut missing = HashMap::<String, Vec<usize>>::new();
            for (index, text) in input.iter().enumerate() {
                let vector = match resolved.get(text) {
                    Some(vector) => Some(vector.clone()),
                    None => self.store.cached_embedding(&fingerprint, text)?,
                };
                if let Some(vector) = vector {
                    output[index] = Some(vector);
                } else {
                    missing.entry(text.clone()).or_default().push(index);
                }
            }
            if missing.is_empty() {
                return output
                    .into_iter()
                    .map(|vector| vector.context("embedding cache left a missing vector"))
                    .collect();
            }

            let mut leaders = Vec::<(String, watch::Sender<bool>)>::new();
            let mut waiters = Vec::<watch::Receiver<bool>>::new();
            {
                let mut inflight = self.inflight.lock().await;
                for text in missing.keys() {
                    let key = format!("{fingerprint}\u{0}{text}");
                    if let Some(sender) = inflight.get(&key) {
                        waiters.push(sender.subscribe());
                    } else {
                        let (sender, receiver) = watch::channel(false);
                        inflight.insert(key, sender.clone());
                        drop(receiver);
                        leaders.push((text.clone(), sender));
                    }
                }
            }

            let waiting = join_all(waiters.into_iter().map(|mut receiver| async move {
                let _ = receiver.wait_for(|value| *value).await;
            }));
            if leaders.is_empty() {
                waiting.await;
                continue;
            }

            let leader_texts = leaders
                .iter()
                .map(|(text, _)| text.clone())
                .collect::<Vec<_>>();
            let leader_keys = leaders
                .iter()
                .map(|(text, sender)| (format!("{fingerprint}\u{0}{text}"), sender.clone()))
                .collect::<Vec<_>>();
            let leader_result = async {
                let result: Result<Vec<(String, Vec<f32>)>> = match self
                    .inner
                    .embed(&leader_texts)
                    .await
                {
                    Ok(vectors) => {
                        anyhow::ensure!(
                            vectors.len() == leader_texts.len(),
                            "embedding provider returned an unexpected vector count"
                        );
                        let pairs = leader_texts
                            .iter()
                            .cloned()
                            .zip(vectors)
                            .collect::<Vec<_>>();
                        if self.max_entries > 0 {
                            for (text, vector) in &pairs {
                                if !self.store.cache_embedding_if_available(
                                    &fingerprint,
                                    text,
                                    vector,
                                )? {
                                    tracing::warn!(
                                        "embedding cache write skipped because another index writer is active"
                                    );
                                }
                            }
                            self.store.prune_embedding_cache(self.max_entries)?;
                        }
                        Ok(pairs)
                    }
                    Err(error) => Err(error),
                };
                let mut inflight = self.inflight.lock().await;
                for (key, sender) in leader_keys {
                    inflight.remove(&key);
                    let _ = sender.send(true);
                }
                result
            };
            let (_waited, result) = tokio::join!(waiting, leader_result);
            for (text, vector) in result? {
                resolved.insert(text, vector);
            }
        }
    }
}

#[async_trait]
impl Embedder for CachedEmbedder {
    async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
        if input.len() == 1 {
            return Ok(vec![self.embed_single_cached(&input[0]).await?]);
        }
        self.embed_batch_cached(input).await
    }

    fn fingerprint(&self) -> String {
        self.inner.fingerprint()
    }

    fn request_concurrency(&self) -> usize {
        self.inner.request_concurrency()
    }

    async fn probe(&self) -> Result<()> {
        self.inner.probe().await
    }
}

#[derive(Clone)]
pub struct OpenAiEmbedder {
    client: Client,
    config: EmbeddingConfig,
    api_key: Option<String>,
}

impl OpenAiEmbedder {
    pub fn new(config: EmbeddingConfig, api_key: Option<String>) -> Result<Self> {
        validate_provider_base_url("embedding", &config.base_url)?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(config.request_timeout_seconds.max(1)))
            .build()?;
        Ok(Self {
            client,
            config,
            api_key,
        })
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
        let mut response = None;
        for attempt in 0..8 {
            let mut request = self.client.post(&url).json(&EmbedRequest {
                model: &self.config.model,
                input,
            });
            if let Some(key) = &self.api_key {
                request = request.bearer_auth(key);
            }
            match request.send().await {
                Ok(candidate) if is_retryable(candidate.status()) && attempt < 7 => {
                    let delay = retry_delay(&candidate, attempt);
                    tracing::warn!(
                        status = %candidate.status(),
                        attempt = attempt + 1,
                        delay_ms = delay.as_millis(),
                        "embedding provider asked Cortana to retry"
                    );
                    tokio::time::sleep(delay).await;
                }
                Ok(candidate) => {
                    response = Some(candidate);
                    break;
                }
                Err(error) if (error.is_connect() || error.is_timeout()) && attempt < 7 => {
                    let delay = exponential_delay(attempt);
                    tracing::warn!(
                        attempt = attempt + 1,
                        delay_ms = delay.as_millis(),
                        "embedding provider connection failed; retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
        let response = response
            .context("embedding provider did not return a response after bounded retries")?
            .error_for_status()?;
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
        format!(
            "openai:{}:{}:{}",
            self.config.base_url.trim_end_matches('/'),
            self.config.model,
            self.config.dimension
        )
    }

    fn request_concurrency(&self) -> usize {
        self.config.request_concurrency.max(1)
    }
}

fn is_retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR
                | StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
        )
}

fn retry_delay(response: &reqwest::Response, attempt: usize) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite())
        .map(|seconds| Duration::from_secs_f64(seconds.clamp(0.0, 30.0)))
        .unwrap_or_else(|| exponential_delay(attempt))
}

fn exponential_delay(attempt: usize) -> Duration {
    Duration::from_secs(1_u64 << attempt.min(5))
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tempfile::tempdir;

    use super::*;

    struct CountingEmbedder {
        calls: AtomicUsize,
        texts: AtomicUsize,
    }
    struct DelayedCountingEmbedder {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Embedder for CountingEmbedder {
        async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.texts.fetch_add(input.len(), Ordering::SeqCst);
            Ok(input.iter().map(|text| vec![text.len() as f32]).collect())
        }

        fn fingerprint(&self) -> String {
            "counting:1".into()
        }
    }

    #[async_trait]
    impl Embedder for DelayedCountingEmbedder {
        async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(25)).await;
            Ok(input.iter().map(|text| vec![text.len() as f32]).collect())
        }

        fn fingerprint(&self) -> String {
            "delayed-counting:1".into()
        }
    }

    #[tokio::test]
    async fn persistent_cache_deduplicates_batches_and_reuses_vectors() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let inner = Arc::new(CountingEmbedder {
            calls: AtomicUsize::new(0),
            texts: AtomicUsize::new(0),
        });
        let cached = CachedEmbedder::new(store.clone(), inner.clone());
        let input = vec!["same".into(), "other".into(), "same".into()];

        let first = cached.embed(&input).await.expect("first batch");
        let second = cached.embed(&input).await.expect("cached batch");

        assert_eq!(first, second);
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(inner.texts.load(Ordering::SeqCst), 2);
        let stats = store.stats().expect("cache stats");
        assert_eq!(stats.embedding_cache_entries, 2);
        assert_eq!(stats.embedding_cache_hits, 3);
    }

    #[tokio::test]
    async fn zero_cache_limit_skips_embedding_cache_writes() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let inner = Arc::new(CountingEmbedder {
            calls: AtomicUsize::new(0),
            texts: AtomicUsize::new(0),
        });
        let cached = CachedEmbedder::with_limit(store.clone(), inner.clone(), 0);

        cached
            .embed(&["same".into()])
            .await
            .expect("first embedding");
        cached
            .embed(&["same".into()])
            .await
            .expect("second embedding");

        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            store.stats().expect("cache stats").embedding_cache_entries,
            0
        );
    }

    #[test]
    fn openai_fingerprint_isolated_by_provider_endpoint() {
        let local = OpenAiEmbedder::new(EmbeddingConfig::default(), None).expect("local config");
        let mut cloud_config = EmbeddingConfig::default();
        cloud_config.base_url = "https://api.example.test/v1".into();
        let cloud = OpenAiEmbedder::new(cloud_config, None).expect("cloud config");
        let mut normalized_config = EmbeddingConfig::default();
        normalized_config.base_url = "http://127.0.0.1:6999/v1/".into();
        let normalized =
            OpenAiEmbedder::new(normalized_config, None).expect("normalized local config");

        assert_ne!(local.fingerprint(), cloud.fingerprint());
        assert_eq!(local.fingerprint(), normalized.fingerprint());
    }

    #[tokio::test]
    async fn concurrent_single_embeddings_share_one_provider_request() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let inner = Arc::new(DelayedCountingEmbedder {
            calls: AtomicUsize::new(0),
        });
        let cached = CachedEmbedder::new(store, inner.clone());
        let first_input = ["same".to_string()];
        let second_input = ["same".to_string()];
        let first = cached.embed(&first_input);
        let second = cached.embed(&second_input);
        let (first, second) = tokio::join!(first, second);

        assert_eq!(
            first.expect("first embedding"),
            second.expect("second embedding")
        );
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_batches_share_requests_for_duplicate_text() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        let inner = Arc::new(DelayedCountingEmbedder {
            calls: AtomicUsize::new(0),
        });
        let cached = CachedEmbedder::new(store, inner.clone());
        let first_input = vec!["same".to_string(), "other".to_string()];
        let second_input = first_input.clone();
        let first = cached.embed(&first_input);
        let second = cached.embed(&second_input);
        let (first, second) = tokio::join!(first, second);

        assert_eq!(first.expect("first batch"), second.expect("second batch"));
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn embedding_retry_policy_is_bounded_and_transient_only() {
        assert!(is_retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retryable(StatusCode::UNAUTHORIZED));
        assert_eq!(exponential_delay(0), Duration::from_secs(1));
        assert_eq!(exponential_delay(10), Duration::from_secs(32));
    }
}
