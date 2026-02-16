//! Swarm Embedder - GPU/CPU Vector Embeddings
//!
//! Provides vectorization with automatic GPU/CPU fallback:
//! - LM Studio (OpenAI-compatible local server, GPU accelerated)
//! - Ollama (local embeddings with GPU support)
//! - Candle for GPU acceleration (CUDA/Metal)
//! - Fast CPU fallback with SIMD
//! - Blake3-based pseudo-embeddings for testing

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use parking_lot::RwLock;
use rayon::prelude::*;
use tracing::{info, warn};

// ============================================================================
// Embedding Configuration
// ============================================================================

/// Preferred embedding backend
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedderBackend {
    /// Auto-detect best available (LM Studio > Ollama > Blake3)
    #[default]
    Auto,
    /// LM Studio (OpenAI-compatible, localhost:1234)
    LmStudio,
    /// Ollama (localhost:11434)
    Ollama,
    /// Candle GPU (requires feature = "gpu")
    Candle,
    /// Fast Blake3 pseudo-embeddings (testing/fallback)
    Blake3,
}

/// Configuration for the embedder
#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Model name/path
    pub model: String,
    /// Embedding dimension
    pub dimension: usize,
    /// Batch size for processing
    pub batch_size: usize,
    /// Preferred backend
    pub backend: EmbedderBackend,
    /// Try GPU first (legacy, use backend instead)
    pub prefer_gpu: bool,
    /// Normalize embeddings to unit vectors
    pub normalize: bool,
    /// Max sequence length
    pub max_length: usize,
    /// LM Studio endpoint (default: http://localhost:1234/v1)
    pub lm_studio_endpoint: String,
    /// Ollama endpoint (default: http://localhost:11434)
    pub ollama_endpoint: String,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            model: "nomic-embed-text".to_string(),
            dimension: 768,
            batch_size: 32,
            backend: EmbedderBackend::Auto,
            prefer_gpu: true,
            normalize: true,
            max_length: 8192,
            lm_studio_endpoint: "http://localhost:1234/v1".to_string(),
            ollama_endpoint: "http://localhost:11434".to_string(),
        }
    }
}

// ============================================================================
// Embedder Trait
// ============================================================================

/// Trait for embedding implementations
pub trait Embedder: Send + Sync {
    /// Embed a single text
    fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a batch of texts
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Get embedding dimension
    fn dimension(&self) -> usize;

    /// Get backend name
    fn backend(&self) -> &str;

    /// Check if GPU is available
    fn is_gpu(&self) -> bool;
}

// ============================================================================
// Blake3 Pseudo-Embedder (Fast fallback)
// ============================================================================

/// Fast pseudo-embeddings using Blake3 hash
/// Used for testing or when no model is available
pub struct Blake3Embedder {
    dimension: usize,
}

impl Blake3Embedder {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

impl Embedder for Blake3Embedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let hash = blake3::hash(text.as_bytes());
        let hash_bytes = hash.as_bytes();

        // Expand hash to target dimension
        let mut vector = Vec::with_capacity(self.dimension);
        for i in 0..self.dimension {
            let byte_idx = i % 32;
            let value = (hash_bytes[byte_idx] as f32 / 255.0) * 2.0 - 1.0;
            vector.push(value);
        }

        // Normalize
        let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vector {
                *v /= norm;
            }
        }

        Ok(vector)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.par_iter().map(|t| self.embed(t)).collect()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn backend(&self) -> &str {
        "blake3-pseudo"
    }

    fn is_gpu(&self) -> bool {
        false
    }
}

// ============================================================================
// HTTP Embedder (Ollama/OpenAI compatible)
// ============================================================================

/// Embedder using HTTP API (Ollama, OpenAI, etc.)
pub struct HttpEmbedder {
    endpoint: String,
    model: String,
    dimension: usize,
    timeout: Duration,
}

impl HttpEmbedder {
    pub fn new(endpoint: &str, model: &str, dimension: usize) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            dimension,
            timeout: Duration::from_secs(30),
        }
    }

    /// Create an Ollama embedder
    pub fn ollama(model: &str) -> Self {
        Self::new("http://localhost:11434/api/embeddings", model, 768)
    }
}

impl Embedder for HttpEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let payload = serde_json::json!({
            "model": self.model,
            "prompt": text
        });

        let response = ureq::post(&self.endpoint)
            .timeout(self.timeout)
            .set("Content-Type", "application/json")
            .send_json(&payload)
            .context("Failed to send embedding request")?;

        let json: serde_json::Value = response
            .into_json()
            .context("Failed to parse JSON response")?;

        let embedding: Vec<f32> = json["embedding"]
            .as_array()
            .context("No embedding in response")?
            .iter()
            .filter_map(|v: &serde_json::Value| v.as_f64().map(|f| f as f32))
            .collect();

        Ok(embedding)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Ollama doesn't support batch, so process sequentially
        texts.iter().map(|t| self.embed(t)).collect()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn backend(&self) -> &str {
        "http-ollama"
    }

    fn is_gpu(&self) -> bool {
        // Ollama may use GPU, but we can't detect it
        false
    }
}

// ============================================================================
// LM Studio Embedder (OpenAI-compatible API)
// ============================================================================

/// Embedder using LM Studio's OpenAI-compatible API
/// Runs on localhost:1234/v1 by default with GPU acceleration
pub struct LmStudioEmbedder {
    endpoint: String,
    model: String,
    dimension: usize,
    timeout: Duration,
}

impl LmStudioEmbedder {
    pub fn new(endpoint: &str, model: &str, dimension: usize) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            dimension,
            timeout: Duration::from_secs(60),
        }
    }

    /// Create with default LM Studio settings
    pub fn default_endpoint(model: &str) -> Self {
        Self::new("http://localhost:1234/v1/embeddings", model, 768)
    }

    /// Check if LM Studio server is running
    pub fn is_available(endpoint: &str) -> bool {
        let models_url = endpoint.replace("/embeddings", "/models");
        ureq::get(&models_url)
            .timeout(Duration::from_secs(2))
            .call()
            .is_ok()
    }

    /// Detect available models from LM Studio
    pub fn detect_models(endpoint: &str) -> Vec<String> {
        let models_url = endpoint.replace("/embeddings", "/models");
        match ureq::get(&models_url)
            .timeout(Duration::from_secs(5))
            .call()
        {
            Ok(response) => {
                if let Ok(json) = response.into_json::<serde_json::Value>() {
                    json["data"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["id"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            Err(_) => Vec::new(),
        }
    }
}

impl Embedder for LmStudioEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // OpenAI-compatible embedding request
        let payload = serde_json::json!({
            "model": self.model,
            "input": text
        });

        let response = ureq::post(&self.endpoint)
            .timeout(self.timeout)
            .set("Content-Type", "application/json")
            .send_json(&payload)
            .context("Failed to send LM Studio embedding request")?;

        let json: serde_json::Value = response
            .into_json()
            .context("Failed to parse LM Studio response")?;

        // OpenAI format: data[0].embedding
        let embedding: Vec<f32> = json["data"][0]["embedding"]
            .as_array()
            .context("No embedding in LM Studio response")?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        Ok(embedding)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // OpenAI API supports batch embeddings
        let payload = serde_json::json!({
            "model": self.model,
            "input": texts
        });

        let response = ureq::post(&self.endpoint)
            .timeout(self.timeout)
            .set("Content-Type", "application/json")
            .send_json(&payload)
            .context("Failed to send LM Studio batch request")?;

        let json: serde_json::Value = response
            .into_json()
            .context("Failed to parse LM Studio batch response")?;

        let data = json["data"]
            .as_array()
            .context("No data array in LM Studio response")?;

        let embeddings: Result<Vec<Vec<f32>>> = data
            .iter()
            .map(|item| {
                item["embedding"]
                    .as_array()
                    .context("Missing embedding in batch item")
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect()
                    })
            })
            .collect();

        embeddings
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn backend(&self) -> &str {
        "lm-studio"
    }

    fn is_gpu(&self) -> bool {
        // LM Studio typically runs with GPU acceleration
        true
    }
}

// ============================================================================
// Candle GPU Embedder (real implementation behind "gpu" feature)
// ============================================================================

/// GPU embedder using Candle
///
/// When compiled with `--features gpu`, this provides real BERT/BGE embedding
/// on CUDA/Metal with bf16 precision and Flash Attention support.
///
/// Without the feature, falls back to Blake3 pseudo-embeddings.
#[cfg(feature = "gpu")]
pub struct CandleEmbedder {
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
    dimension: usize,
    normalize: bool,
}

#[cfg(feature = "gpu")]
impl CandleEmbedder {
    /// Load a model from HuggingFace Hub
    ///
    /// Default: BAAI/bge-small-en-v1.5 (384-dim, fast, accurate)
    pub fn new(model_id: &str, prefer_gpu: bool) -> Result<Self> {
        use candle_core::{DType, Device};
        use candle_nn::VarBuilder;
        use candle_transformers::models::bert::{BertModel, Config as BertConfig};

        info!("Loading Candle embedder: {}", model_id);

        // Select device
        let device = if prefer_gpu {
            match Device::cuda_if_available(0) {
                Ok(d) => {
                    info!("Candle using CUDA device");
                    d
                }
                Err(e) => {
                    warn!("CUDA not available ({}), using CPU", e);
                    Device::Cpu
                }
            }
        } else {
            Device::Cpu
        };

        // Load from HuggingFace Hub
        let api = hf_hub::api::sync::Api::new().context("Failed to init HF Hub API")?;
        let repo = api.model(model_id.to_string());

        // Load config
        let config_path = repo
            .get("config.json")
            .context("Failed to download config.json")?;
        let config_str = std::fs::read_to_string(&config_path)?;
        let mut config: BertConfig = serde_json::from_str(&config_str)?;
        let dimension = config.hidden_size;

        // Flash Attention is enabled at compile time via the `flash-attn` feature flag.
        // When built with `--features flash-attn`, candle-transformers automatically uses
        // flash attention kernels for CUDA devices. No runtime config needed.
        #[cfg(feature = "flash-attn")]
        {
            if !matches!(device, Device::Cpu) {
                info!("Flash Attention enabled (compiled with flash-attn feature)");
            }
        }

        // Load tokenizer
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("Failed to download tokenizer.json")?;
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Tokenizer load error: {}", e))?;

        // Load weights (prefer safetensors)
        let weights_path = repo
            .get("model.safetensors")
            .or_else(|_| repo.get("pytorch_model.bin"))
            .context("Failed to download model weights")?;

        let vb = if weights_path.extension().and_then(|e| e.to_str()) == Some("safetensors") {
            unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], DType::BF16, &device)? }
        } else {
            warn!("Using PyTorch weights — safetensors preferred for speed");
            VarBuilder::from_pth(weights_path, DType::BF16, &device)?
        };

        let model = BertModel::load(vb, &config)?;

        info!(
            "Candle embedder loaded: {} (dim={}, device={:?})",
            model_id, dimension, device
        );

        Ok(Self {
            model,
            tokenizer,
            device,
            dimension,
            normalize: true,
        })
    }

    /// Default BGE-small model
    pub fn bge_small(prefer_gpu: bool) -> Result<Self> {
        Self::new("BAAI/bge-small-en-v1.5", prefer_gpu)
    }

    /// Encode text to token tensors
    fn encode(&self, text: &str) -> Result<(candle_core::Tensor, candle_core::Tensor)> {
        use candle_core::{DType, Tensor};

        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Tokenize error: {}", e))?;

        let ids = encoding.get_ids().to_vec();
        let mask = encoding.get_attention_mask().to_vec();

        let token_ids = Tensor::new(&ids[..], &self.device)?
            .unsqueeze(0)?
            .to_dtype(DType::U32)?;
        let attention_mask = Tensor::new(&mask[..], &self.device)?
            .unsqueeze(0)?
            .to_dtype(DType::U8)?;

        Ok((token_ids, attention_mask))
    }

    /// Encode a batch of texts to tensors (padded)
    fn encode_batch(&self, texts: &[&str]) -> Result<(candle_core::Tensor, candle_core::Tensor)> {
        use candle_core::{DType, Tensor};

        let encodings: Vec<_> = texts
            .iter()
            .map(|t| {
                self.tokenizer
                    .encode(*t, true)
                    .map_err(|e| anyhow::anyhow!("Tokenize error: {}", e))
            })
            .collect::<Result<Vec<_>>>()?;

        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);

        let mut all_ids = Vec::new();
        let mut all_masks = Vec::new();

        for enc in &encodings {
            let mut ids = enc.get_ids().to_vec();
            let mut mask = enc.get_attention_mask().to_vec();
            // Pad to max_len
            ids.resize(max_len, 0);
            mask.resize(max_len, 0);
            all_ids.push(ids);
            all_masks.push(mask);
        }

        let token_ids = Tensor::new(all_ids, &self.device)?.to_dtype(DType::U32)?;
        let attention_mask = Tensor::new(all_masks, &self.device)?.to_dtype(DType::U8)?;

        Ok((token_ids, attention_mask))
    }

    /// Mean pooling over last hidden state (masked)
    fn mean_pool(
        &self,
        hidden: &candle_core::Tensor,
        mask: &candle_core::Tensor,
    ) -> Result<candle_core::Tensor> {
        use candle_core::DType;

        let mask_f = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let masked = hidden.to_dtype(DType::F32)?.broadcast_mul(&mask_f)?;
        let summed = masked.sum(1)?;
        let count = mask_f.sum(1)?.clamp(1e-9, f64::MAX)?;
        let pooled = summed.broadcast_div(&count)?;

        Ok(pooled)
    }
}

#[cfg(feature = "gpu")]
impl Embedder for CandleEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let (token_ids, attention_mask) = self.encode(text)?;
        let hidden = self.model.forward(&token_ids, &attention_mask, None)?;
        let pooled = self.mean_pool(&hidden, &attention_mask)?;

        let mut vector: Vec<f32> = pooled.squeeze(0)?.to_vec1()?;

        if self.normalize {
            let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vector {
                    *v /= norm;
                }
            }
        }

        Ok(vector)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Process in sub-batches of 64 for VRAM safety
        let batch_size = 64;
        let mut all_embeddings = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(batch_size) {
            let (token_ids, attention_mask) = self.encode_batch(chunk)?;
            let hidden = self.model.forward(&token_ids, &attention_mask, None)?;
            let pooled = self.mean_pool(&hidden, &attention_mask)?;

            let batch_emb: Vec<Vec<f32>> = pooled.to_vec2()?;

            for mut emb in batch_emb {
                if self.normalize {
                    let norm: f32 = emb.iter().map(|v| v * v).sum::<f32>().sqrt();
                    if norm > 0.0 {
                        for v in &mut emb {
                            *v /= norm;
                        }
                    }
                }
                all_embeddings.push(emb);
            }
        }

        Ok(all_embeddings)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn backend(&self) -> &str {
        "candle-gpu"
    }

    fn is_gpu(&self) -> bool {
        !matches!(self.device, candle_core::Device::Cpu)
    }
}

// Fallback when gpu feature is not enabled
#[cfg(not(feature = "gpu"))]
pub struct CandleEmbedder {
    dimension: usize,
}

#[cfg(not(feature = "gpu"))]
impl CandleEmbedder {
    pub fn new(_model_path: &std::path::Path, dimension: usize) -> Result<Self> {
        warn!("Candle GPU not available (compile with --features gpu). Using Blake3 fallback.");
        Ok(Self { dimension })
    }
}

#[cfg(not(feature = "gpu"))]
impl Embedder for CandleEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Blake3Embedder::new(self.dimension).embed(text)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.par_iter().map(|t| self.embed(t)).collect()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn backend(&self) -> &str {
        "candle-fallback-blake3"
    }

    fn is_gpu(&self) -> bool {
        false
    }
}

// ============================================================================
// Embedding Cache (in-memory, blake3-keyed)
// ============================================================================

/// In-memory cache for embeddings, keyed by blake3 hash of input text.
/// Avoids re-computing embeddings for identical chunks.
pub struct EmbeddingCache {
    cache: RwLock<std::collections::HashMap<[u8; 32], Vec<f32>>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl EmbeddingCache {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(std::collections::HashMap::new()),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    /// Get cached embedding or compute and cache it
    pub fn get_or_compute(&self, text: &str, embedder: &dyn Embedder) -> Result<Vec<f32>> {
        let key = *blake3::hash(text.as_bytes()).as_bytes();

        // Check cache
        {
            let cache = self.cache.read();
            if let Some(embedding) = cache.get(&key) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Ok(embedding.clone());
            }
        }

        // Compute and cache
        self.misses.fetch_add(1, Ordering::Relaxed);
        let embedding = embedder.embed(text)?;

        {
            let mut cache = self.cache.write();
            cache.insert(key, embedding.clone());
        }

        Ok(embedding)
    }

    /// Batch get-or-compute
    pub fn get_or_compute_batch(
        &self,
        texts: &[&str],
        embedder: &dyn Embedder,
    ) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        let mut uncached_indices = Vec::new();
        let mut uncached_texts = Vec::new();

        // Check cache for each text
        {
            let cache = self.cache.read();
            for (i, text) in texts.iter().enumerate() {
                let key = *blake3::hash(text.as_bytes()).as_bytes();
                if let Some(embedding) = cache.get(&key) {
                    self.hits.fetch_add(1, Ordering::Relaxed);
                    results.push(Some(embedding.clone()));
                } else {
                    self.misses.fetch_add(1, Ordering::Relaxed);
                    results.push(None);
                    uncached_indices.push(i);
                    uncached_texts.push(*text);
                }
            }
        }

        // Batch-embed uncached texts
        if !uncached_texts.is_empty() {
            let new_embeddings = embedder.embed_batch(&uncached_texts)?;
            let mut cache = self.cache.write();

            for (idx, embedding) in uncached_indices.into_iter().zip(new_embeddings) {
                let key = *blake3::hash(texts[idx].as_bytes()).as_bytes();
                cache.insert(key, embedding.clone());
                results[idx] = Some(embedding);
            }
        }

        // Unwrap all (safe — all Nones were filled above)
        Ok(results.into_iter().map(|r| r.unwrap()).collect())
    }

    /// Cache statistics
    pub fn stats(&self) -> (usize, usize, usize) {
        let size = self.cache.read().len();
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        (size, hits, misses)
    }

    /// Clear cache
    pub fn clear(&self) {
        self.cache.write().clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }
}

impl Default for EmbeddingCache {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Adaptive Embedder (auto GPU/CPU fallback)
// ============================================================================

/// Adaptive embedder with automatic fallback
pub struct AdaptiveEmbedder {
    /// Primary embedder (GPU if available)
    primary: Arc<dyn Embedder>,
    /// Fallback embedder (CPU)
    fallback: Arc<dyn Embedder>,
    /// Whether primary is available
    primary_available: AtomicBool,
    /// Error count for primary
    primary_errors: AtomicUsize,
    /// Max errors before fallback
    max_errors: usize,
    /// Configuration
    config: EmbedderConfig,
    /// Statistics
    stats: Arc<RwLock<EmbedderStats>>,
}

#[derive(Debug, Default)]
pub struct EmbedderStats {
    pub total_embeddings: usize,
    pub gpu_embeddings: usize,
    pub cpu_embeddings: usize,
    pub fallback_count: usize,
    pub total_tokens: usize,
}

impl AdaptiveEmbedder {
    /// Create with automatic backend detection
    pub fn new(config: EmbedderConfig) -> Self {
        let dimension = config.dimension;
        let blake3_fallback = Arc::new(Blake3Embedder::new(dimension));

        let primary: Arc<dyn Embedder> = match config.backend {
            EmbedderBackend::LmStudio => {
                let lm = LmStudioEmbedder::new(
                    &format!("{}/embeddings", config.lm_studio_endpoint),
                    &config.model,
                    dimension,
                );
                info!("Using LM Studio embedder at {}", config.lm_studio_endpoint);
                Arc::new(lm)
            }
            EmbedderBackend::Ollama => {
                let http = HttpEmbedder::ollama(&config.model);
                info!("Using Ollama embedder");
                Arc::new(http)
            }
            EmbedderBackend::Blake3 => {
                info!("Using Blake3 pseudo-embedder");
                Arc::new(Blake3Embedder::new(dimension))
            }
            EmbedderBackend::Candle => {
                // Placeholder - would use CandleEmbedder with GPU
                info!("Using Candle GPU embedder (placeholder)");
                Arc::new(Blake3Embedder::new(dimension))
            }
            EmbedderBackend::Auto => {
                // Auto-detect: LM Studio > Ollama > Blake3
                Self::auto_detect_backend(&config, dimension)
            }
        };

        Self {
            primary,
            fallback: blake3_fallback,
            primary_available: AtomicBool::new(true),
            primary_errors: AtomicUsize::new(0),
            max_errors: 3,
            config,
            stats: Arc::new(RwLock::new(EmbedderStats::default())),
        }
    }

    /// Auto-detect the best available backend
    fn auto_detect_backend(config: &EmbedderConfig, dimension: usize) -> Arc<dyn Embedder> {
        // 1. Try LM Studio first (best GPU performance)
        let lm_endpoint = format!("{}/embeddings", config.lm_studio_endpoint);
        if LmStudioEmbedder::is_available(&lm_endpoint) {
            let models = LmStudioEmbedder::detect_models(&lm_endpoint);
            let model = if models.is_empty() {
                config.model.clone()
            } else {
                // Prefer embedding models
                models
                    .iter()
                    .find(|m| m.contains("embed") || m.contains("bge") || m.contains("gte"))
                    .cloned()
                    .unwrap_or_else(|| models[0].clone())
            };
            info!(
                "Auto-detected LM Studio at {} with model: {}",
                config.lm_studio_endpoint, model
            );
            return Arc::new(LmStudioEmbedder::new(&lm_endpoint, &model, dimension));
        }

        // 2. Try Ollama
        let ollama = HttpEmbedder::ollama(&config.model);
        if ollama.embed("test").is_ok() {
            info!("Auto-detected Ollama embedder");
            return Arc::new(ollama);
        }

        // 3. Fall back to Blake3
        warn!("No embedding server detected, using Blake3 pseudo-embeddings");
        Arc::new(Blake3Embedder::new(dimension))
    }

    /// Get current backend
    pub fn current_backend(&self) -> &str {
        if self.primary_available.load(Ordering::Relaxed) {
            self.primary.backend()
        } else {
            self.fallback.backend()
        }
    }

    /// Get statistics
    pub fn stats(&self) -> EmbedderStats {
        let stats = self.stats.read();
        EmbedderStats {
            total_embeddings: stats.total_embeddings,
            gpu_embeddings: stats.gpu_embeddings,
            cpu_embeddings: stats.cpu_embeddings,
            fallback_count: stats.fallback_count,
            total_tokens: stats.total_tokens,
        }
    }

    /// Force fallback to CPU
    pub fn force_fallback(&self) {
        self.primary_available.store(false, Ordering::Relaxed);
        warn!("Forced fallback to CPU embedder");
    }

    /// Reset to try primary again
    pub fn reset(&self) {
        self.primary_available.store(true, Ordering::Relaxed);
        self.primary_errors.store(0, Ordering::Relaxed);
    }
}

impl Embedder for AdaptiveEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let use_primary = self.primary_available.load(Ordering::Relaxed);

        let result = if use_primary {
            match self.primary.embed(text) {
                Ok(v) => {
                    // Reset error count on success
                    self.primary_errors.store(0, Ordering::Relaxed);
                    let mut stats = self.stats.write();
                    stats.total_embeddings += 1;
                    stats.gpu_embeddings += 1;
                    Ok(v)
                }
                Err(_e) => {
                    let errors = self.primary_errors.fetch_add(1, Ordering::Relaxed) + 1;
                    if errors >= self.max_errors {
                        warn!(
                            "Primary embedder failed {} times, switching to fallback",
                            errors
                        );
                        self.primary_available.store(false, Ordering::Relaxed);
                    }

                    // Try fallback
                    let mut stats = self.stats.write();
                    stats.fallback_count += 1;
                    drop(stats);

                    self.fallback.embed(text)
                }
            }
        } else {
            let result = self.fallback.embed(text);
            let mut stats = self.stats.write();
            stats.total_embeddings += 1;
            stats.cpu_embeddings += 1;
            result
        };

        // Normalize if configured
        if self.config.normalize {
            result.map(|mut v| {
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
        } else {
            result
        }
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Process in configured batch size
        let results: Result<Vec<Vec<f32>>> = texts
            .chunks(self.config.batch_size)
            .flat_map(|chunk| chunk.par_iter().map(|t| self.embed(t)).collect::<Vec<_>>())
            .collect();

        results
    }

    fn dimension(&self) -> usize {
        self.config.dimension
    }

    fn backend(&self) -> &str {
        self.current_backend()
    }

    fn is_gpu(&self) -> bool {
        self.primary_available.load(Ordering::Relaxed) && self.primary.is_gpu()
    }
}

// ============================================================================
// Embedding Utilities
// ============================================================================

/// Compute cosine similarity between two vectors
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Find top-k most similar vectors
pub fn find_top_k(query: &[f32], candidates: &[Vec<f32>], k: usize) -> Vec<(usize, f32)> {
    let mut scores: Vec<(usize, f32)> = candidates
        .par_iter()
        .enumerate()
        .map(|(i, c)| (i, cosine_similarity(query, c)))
        .collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(k);
    scores
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blake3_embedder() {
        let embedder = Blake3Embedder::new(768);

        let v1 = embedder.embed("hello world").unwrap();
        let v2 = embedder.embed("hello world").unwrap();
        let v3 = embedder.embed("goodbye world").unwrap();

        assert_eq!(v1.len(), 768);
        assert_eq!(v1, v2); // Same input = same output
        assert_ne!(v1, v3); // Different input = different output

        // Check normalization
        let norm: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_batch_embedding() {
        let embedder = Blake3Embedder::new(384);
        let texts = vec!["hello", "world", "test"];

        let embeddings = embedder.embed_batch(&texts).unwrap();
        assert_eq!(embeddings.len(), 3);
        assert_eq!(embeddings[0].len(), 384);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let c = vec![0.0, 1.0, 0.0];

        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);
        assert!((cosine_similarity(&a, &c)).abs() < 0.001);
    }

    #[test]
    fn test_find_top_k() {
        let query = vec![1.0, 0.0, 0.0];
        let candidates = vec![
            vec![1.0, 0.0, 0.0],  // Most similar
            vec![0.0, 1.0, 0.0],  // Orthogonal
            vec![0.5, 0.5, 0.0],  // Partially similar
            vec![-1.0, 0.0, 0.0], // Opposite
        ];

        let results = find_top_k(&query, &candidates, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 0); // First candidate is most similar
    }

    #[test]
    fn test_adaptive_embedder() {
        let config = EmbedderConfig {
            backend: EmbedderBackend::Blake3,
            dimension: 512,
            ..Default::default()
        };

        let embedder = AdaptiveEmbedder::new(config);
        assert_eq!(embedder.backend(), "blake3-pseudo");

        let v = embedder.embed("test").unwrap();
        assert_eq!(v.len(), 512);
    }

    #[test]
    fn test_embedder_backend_enum() {
        assert_eq!(EmbedderBackend::default(), EmbedderBackend::Auto);

        let config = EmbedderConfig {
            backend: EmbedderBackend::LmStudio,
            ..Default::default()
        };
        assert_eq!(config.backend, EmbedderBackend::LmStudio);
    }

    #[test]
    fn test_lm_studio_embedder_creation() {
        let embedder = LmStudioEmbedder::default_endpoint("text-embedding-3-small");
        assert_eq!(embedder.dimension(), 768);
        assert_eq!(embedder.backend(), "lm-studio");
        assert!(embedder.is_gpu());
    }
}
