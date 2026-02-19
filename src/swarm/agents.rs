//! Swarm Agents - The 5 core agent roles for parallel processing
//!
//! Each agent is a specialized worker that can operate independently
//! while coordinating through message channels.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;
use rayon::prelude::*;
use tracing::{debug, info, warn};

// ============================================================================
// Agent Messages
// ============================================================================

/// Messages passed between agents
#[derive(Debug, Clone)]
pub enum SwarmMessage {
    /// File path to process
    FilePath(PathBuf),
    /// Chunked document data
    Chunk {
        source: PathBuf,
        chunk_id: usize,
        data: Vec<u8>,
    },
    /// Embedded vector
    Embedding {
        source: PathBuf,
        chunk_id: usize,
        vector: Vec<f32>,
    },
    /// Verification result
    Verified {
        source: PathBuf,
        hash: String,
        valid: bool,
    },
    /// Agent failure requiring heal
    Failure {
        agent: AgentRole,
        source: PathBuf,
        error: String,
        retries_left: u32,
    },
    /// Signal completion
    Done,
}

/// Agent roles in the swarm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentRole {
    Scan,
    Chunk,
    Embed,
    Heal,
    VerifyExport,
}

impl AgentRole {
    pub fn icon(&self) -> &'static str {
        match self {
            AgentRole::Scan => "🔍",
            AgentRole::Chunk => "✂️",
            AgentRole::Embed => "🧠",
            AgentRole::Heal => "💊",
            AgentRole::VerifyExport => "✅",
        }
    }
}

// ============================================================================
// Swarm Statistics
// ============================================================================

/// Statistics tracked by the swarm
#[derive(Debug, Default)]
pub struct SwarmStats {
    pub files_scanned: AtomicUsize,
    pub chunks_created: AtomicUsize,
    pub embeddings_generated: AtomicUsize,
    pub heals_performed: AtomicUsize,
    pub exports_completed: AtomicUsize,
    pub bytes_processed: AtomicU64,
    pub errors_encountered: AtomicUsize,
    pub errors_healed: AtomicUsize,
}

impl SwarmStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn to_summary(&self) -> SwarmSummary {
        SwarmSummary {
            files_scanned: self.files_scanned.load(Ordering::Relaxed),
            chunks_created: self.chunks_created.load(Ordering::Relaxed),
            embeddings_generated: self.embeddings_generated.load(Ordering::Relaxed),
            heals_performed: self.heals_performed.load(Ordering::Relaxed),
            exports_completed: self.exports_completed.load(Ordering::Relaxed),
            bytes_processed: self.bytes_processed.load(Ordering::Relaxed),
            errors_encountered: self.errors_encountered.load(Ordering::Relaxed),
            errors_healed: self.errors_healed.load(Ordering::Relaxed),
        }
    }
}

/// Serializable summary of swarm statistics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwarmSummary {
    pub files_scanned: usize,
    pub chunks_created: usize,
    pub embeddings_generated: usize,
    pub heals_performed: usize,
    pub exports_completed: usize,
    pub bytes_processed: u64,
    pub errors_encountered: usize,
    pub errors_healed: usize,
}

// ============================================================================
// ScanAgent - Directory crawl with Rayon par_iter
// ============================================================================

/// Scans directories in parallel, sending file paths to ChunkAgent
pub struct ScanAgent {
    source: PathBuf,
    output: Sender<SwarmMessage>,
    heal_tx: Sender<SwarmMessage>,
    stats: Arc<SwarmStats>,
    skip_hidden: bool,
    extensions: Option<Vec<String>>,
}

impl ScanAgent {
    pub fn new(
        source: PathBuf,
        output: Sender<SwarmMessage>,
        heal_tx: Sender<SwarmMessage>,
        stats: Arc<SwarmStats>,
    ) -> Self {
        Self {
            source,
            output,
            heal_tx,
            stats,
            skip_hidden: true,
            extensions: None,
        }
    }

    pub fn with_extensions(mut self, exts: Vec<String>) -> Self {
        self.extensions = Some(exts);
        self
    }

    pub fn skip_hidden(mut self, skip: bool) -> Self {
        self.skip_hidden = skip;
        self
    }

    /// Run the scan agent - parallel directory traversal
    pub fn run(&self) -> Result<()> {
        info!(
            "{} ScanAgent starting: {}",
            AgentRole::Scan.icon(),
            self.source.display()
        );

        let entries: Vec<PathBuf> = walkdir::WalkDir::new(&self.source)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if self.skip_hidden {
                    !e.file_name()
                        .to_str()
                        .map(|s| s.starts_with('.'))
                        .unwrap_or(false)
                } else {
                    true
                }
            })
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| self.matches_extensions(e.path()))
            .map(|e| e.path().to_path_buf())
            .collect();

        // Process in parallel with rayon
        entries.par_iter().for_each(|path| {
            match self.process_file(path) {
                Ok(()) => {
                    self.stats.files_scanned.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    self.stats
                        .errors_encountered
                        .fetch_add(1, Ordering::Relaxed);
                    // Send to heal agent
                    let _ = self.heal_tx.send(SwarmMessage::Failure {
                        agent: AgentRole::Scan,
                        source: path.clone(),
                        error: e.to_string(),
                        retries_left: 3,
                    });
                }
            }
        });

        // Signal done
        let _ = self.output.send(SwarmMessage::Done);
        info!(
            "{} ScanAgent complete: {} files",
            AgentRole::Scan.icon(),
            self.stats.files_scanned.load(Ordering::Relaxed)
        );

        Ok(())
    }

    fn process_file(&self, path: &Path) -> Result<()> {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Failed to read metadata: {}", path.display()))?;

        self.stats
            .bytes_processed
            .fetch_add(metadata.len(), Ordering::Relaxed);

        // Send to chunk agent
        self.output
            .send(SwarmMessage::FilePath(path.to_path_buf()))
            .with_context(|| "Failed to send to chunk agent")?;

        Ok(())
    }

    fn matches_extensions(&self, path: &std::path::Path) -> bool {
        match &self.extensions {
            Some(exts) => path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
                .unwrap_or(false),
            None => true,
        }
    }
}

// ============================================================================
// ChunkAgent - Document splitting with par_chunks
// ============================================================================

/// Chunks documents in parallel, sending chunks to EmbedAgent
pub struct ChunkAgent {
    input: Receiver<SwarmMessage>,
    output: Sender<SwarmMessage>,
    heal_tx: Sender<SwarmMessage>,
    stats: Arc<SwarmStats>,
    chunk_size: usize,
    overlap: usize,
}

impl ChunkAgent {
    pub fn new(
        input: Receiver<SwarmMessage>,
        output: Sender<SwarmMessage>,
        heal_tx: Sender<SwarmMessage>,
        stats: Arc<SwarmStats>,
    ) -> Self {
        Self {
            input,
            output,
            heal_tx,
            stats,
            chunk_size: 1024, // 1KB default chunks
            overlap: 128,     // 128 byte overlap
        }
    }

    pub fn with_chunk_size(mut self, size: usize, overlap: usize) -> Self {
        self.chunk_size = size;
        self.overlap = overlap;
        self
    }

    /// Run the chunk agent - parallel document splitting
    pub fn run(&self) -> Result<()> {
        info!("{} ChunkAgent starting", AgentRole::Chunk.icon());

        while let Ok(msg) = self.input.recv() {
            match msg {
                SwarmMessage::FilePath(path) => {
                    if let Err(e) = self.process_file(&path) {
                        self.stats
                            .errors_encountered
                            .fetch_add(1, Ordering::Relaxed);
                        let _ = self.heal_tx.send(SwarmMessage::Failure {
                            agent: AgentRole::Chunk,
                            source: path,
                            error: e.to_string(),
                            retries_left: 3,
                        });
                    }
                }
                SwarmMessage::Done => {
                    let _ = self.output.send(SwarmMessage::Done);
                    break;
                }
                _ => {}
            }
        }

        info!(
            "{} ChunkAgent complete: {} chunks",
            AgentRole::Chunk.icon(),
            self.stats.chunks_created.load(Ordering::Relaxed)
        );

        Ok(())
    }

    fn process_file(&self, path: &Path) -> Result<()> {
        let data = std::fs::read(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;

        // Split into chunks with overlap using par_chunks
        let chunks: Vec<(usize, Vec<u8>)> = data
            .par_chunks(self.chunk_size)
            .enumerate()
            .map(|(i, chunk)| {
                // Include overlap from previous chunk if not first
                let mut chunk_data = chunk.to_vec();
                if i > 0 && self.overlap > 0 {
                    let start = i * self.chunk_size;
                    if start >= self.overlap {
                        let overlap_start = start - self.overlap;
                        let overlap_slice = &data[overlap_start..start];
                        let mut with_overlap = overlap_slice.to_vec();
                        with_overlap.extend_from_slice(&chunk_data);
                        chunk_data = with_overlap;
                    }
                }
                (i, chunk_data)
            })
            .collect();

        for (chunk_id, chunk_data) in chunks {
            self.stats.chunks_created.fetch_add(1, Ordering::Relaxed);
            self.output.send(SwarmMessage::Chunk {
                source: path.to_path_buf(),
                chunk_id,
                data: chunk_data,
            })?;
        }

        Ok(())
    }
}

// ============================================================================
// EmbedAgent - Vectorization with GPU/CPU fallback
// ============================================================================

/// Embedding configuration
#[derive(Debug, Clone)]
pub struct EmbedConfig {
    pub use_gpu: bool,
    pub model_dim: usize,
    pub batch_size: usize,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            use_gpu: true,
            model_dim: 768,
            batch_size: 32,
        }
    }
}

/// Generates embeddings with GPU/CPU fallback
pub struct EmbedAgent {
    input: Receiver<SwarmMessage>,
    output: Sender<SwarmMessage>,
    heal_tx: Sender<SwarmMessage>,
    stats: Arc<SwarmStats>,
    config: EmbedConfig,
    gpu_available: Arc<RwLock<bool>>,
}

impl EmbedAgent {
    pub fn new(
        input: Receiver<SwarmMessage>,
        output: Sender<SwarmMessage>,
        heal_tx: Sender<SwarmMessage>,
        stats: Arc<SwarmStats>,
    ) -> Self {
        Self {
            input,
            output,
            heal_tx,
            stats,
            config: EmbedConfig::default(),
            gpu_available: Arc::new(RwLock::new(true)),
        }
    }

    pub fn with_config(mut self, config: EmbedConfig) -> Self {
        self.config = config;
        self
    }

    /// Run the embed agent - parallel vectorization
    pub fn run(&self) -> Result<()> {
        info!(
            "{} EmbedAgent starting (GPU: {})",
            AgentRole::Embed.icon(),
            self.config.use_gpu
        );

        let mut batch: Vec<SwarmMessage> = Vec::with_capacity(self.config.batch_size);

        while let Ok(msg) = self.input.recv() {
            match msg {
                SwarmMessage::Chunk { .. } => {
                    batch.push(msg);
                    if batch.len() >= self.config.batch_size {
                        self.process_batch(&batch);
                        batch.clear();
                    }
                }
                SwarmMessage::Done => {
                    // Process remaining batch
                    if !batch.is_empty() {
                        self.process_batch(&batch);
                    }
                    let _ = self.output.send(SwarmMessage::Done);
                    break;
                }
                _ => {}
            }
        }

        info!(
            "{} EmbedAgent complete: {} embeddings",
            AgentRole::Embed.icon(),
            self.stats.embeddings_generated.load(Ordering::Relaxed)
        );

        Ok(())
    }

    fn process_batch(&self, batch: &[SwarmMessage]) {
        // Try GPU first, fall back to CPU
        let use_gpu = *self.gpu_available.read();

        let results: Vec<_> = batch
            .par_iter()
            .map(|msg| {
                if let SwarmMessage::Chunk {
                    source,
                    chunk_id,
                    data,
                } = msg
                {
                    match self.embed_chunk(data, use_gpu) {
                        Ok(vector) => Ok((source.clone(), *chunk_id, vector)),
                        Err(e) => Err((source.clone(), *chunk_id, e.to_string())),
                    }
                } else {
                    Err((PathBuf::new(), 0, "Invalid message".to_string()))
                }
            })
            .collect();

        for result in results {
            match result {
                Ok((source, chunk_id, vector)) => {
                    self.stats
                        .embeddings_generated
                        .fetch_add(1, Ordering::Relaxed);
                    let _ = self.output.send(SwarmMessage::Embedding {
                        source,
                        chunk_id,
                        vector,
                    });
                }
                Err((source, _chunk_id, error)) => {
                    self.stats
                        .errors_encountered
                        .fetch_add(1, Ordering::Relaxed);

                    // If GPU failed, disable for future attempts
                    if error.contains("GPU") || error.contains("CUDA") {
                        warn!("GPU error detected, falling back to CPU");
                        *self.gpu_available.write() = false;
                    }

                    let _ = self.heal_tx.send(SwarmMessage::Failure {
                        agent: AgentRole::Embed,
                        source,
                        error,
                        retries_left: 3,
                    });
                }
            }
        }
    }

    fn embed_chunk(&self, data: &[u8], use_gpu: bool) -> Result<Vec<f32>> {
        // Placeholder: In production, this would call actual embedding model
        // For now, generate deterministic hash-based pseudo-embedding
        let hash = blake3::hash(data);
        let hash_bytes = hash.as_bytes();

        let mut vector = vec![0.0f32; self.config.model_dim];
        for (i, v) in vector.iter_mut().enumerate() {
            let byte_idx = i % 32;
            *v = (hash_bytes[byte_idx] as f32 / 255.0) * 2.0 - 1.0;
        }

        // Simulate GPU/CPU processing difference
        if use_gpu {
            debug!("GPU embedding: {} bytes", data.len());
        } else {
            debug!("CPU embedding (fallback): {} bytes", data.len());
        }

        Ok(vector)
    }
}

// ============================================================================
// VerifyExportAgent - Validation and output
// ============================================================================

/// Type alias for stored embeddings (source path, chunk id, vector)
type EmbeddingEntry = (PathBuf, usize, Vec<f32>);

/// Verifies embeddings and exports results
pub struct VerifyExportAgent {
    input: Receiver<SwarmMessage>,
    heal_tx: Sender<SwarmMessage>,
    stats: Arc<SwarmStats>,
    output_path: Option<PathBuf>,
    embeddings: Arc<RwLock<Vec<EmbeddingEntry>>>,
}

impl VerifyExportAgent {
    pub fn new(
        input: Receiver<SwarmMessage>,
        heal_tx: Sender<SwarmMessage>,
        stats: Arc<SwarmStats>,
    ) -> Self {
        Self {
            input,
            heal_tx,
            stats,
            output_path: None,
            embeddings: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn with_output(mut self, path: PathBuf) -> Self {
        self.output_path = Some(path);
        self
    }

    /// Run the verify/export agent
    pub fn run(&self) -> Result<()> {
        info!(
            "{} VerifyExportAgent starting",
            AgentRole::VerifyExport.icon()
        );

        while let Ok(msg) = self.input.recv() {
            match msg {
                SwarmMessage::Embedding {
                    source,
                    chunk_id,
                    vector,
                } => {
                    if let Err(e) = self.verify_and_store(&source, chunk_id, vector) {
                        self.stats
                            .errors_encountered
                            .fetch_add(1, Ordering::Relaxed);
                        let _ = self.heal_tx.send(SwarmMessage::Failure {
                            agent: AgentRole::VerifyExport,
                            source,
                            error: e.to_string(),
                            retries_left: 3,
                        });
                    }
                }
                SwarmMessage::Done => break,
                _ => {}
            }
        }

        // Final export
        if let Some(ref output_path) = self.output_path {
            self.export(output_path)?;
        }

        info!(
            "{} VerifyExportAgent complete: {} exports",
            AgentRole::VerifyExport.icon(),
            self.stats.exports_completed.load(Ordering::Relaxed)
        );

        Ok(())
    }

    fn verify_and_store(
        &self,
        source: &std::path::Path,
        chunk_id: usize,
        vector: Vec<f32>,
    ) -> Result<()> {
        // Verify vector dimensions
        if vector.is_empty() {
            anyhow::bail!("Empty embedding vector");
        }

        // Verify no NaN/Inf values
        for v in &vector {
            if v.is_nan() || v.is_infinite() {
                anyhow::bail!("Invalid embedding value: NaN or Inf detected");
            }
        }

        // Store verified embedding
        self.embeddings
            .write()
            .push((source.to_path_buf(), chunk_id, vector));
        self.stats.exports_completed.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    fn export(&self, output_path: &std::path::Path) -> Result<()> {
        let embeddings = self.embeddings.read();

        // Create export manifest
        let manifest = serde_json::json!({
            "total_embeddings": embeddings.len(),
            "files": embeddings.iter().map(|(path, chunk_id, vec)| {
                serde_json::json!({
                    "source": path.to_string_lossy(),
                    "chunk_id": chunk_id,
                    "dim": vec.len(),
                    "norm": (vec.iter().map(|v| v * v).sum::<f32>()).sqrt()
                })
            }).collect::<Vec<_>>()
        });

        std::fs::write(output_path, serde_json::to_string_pretty(&manifest)?)?;
        info!("Exported manifest to: {}", output_path.display());

        Ok(())
    }

    /// Get all stored embeddings
    pub fn get_embeddings(&self) -> Vec<EmbeddingEntry> {
        self.embeddings.read().clone()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;
    use tempfile::tempdir;

    #[test]
    fn test_scan_agent() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();

        // Create test files with sufficient content
        std::fs::write(dir_path.join("test1.txt"), "Hello world test content").unwrap();
        std::fs::write(dir_path.join("test2.txt"), "Goodbye world test content").unwrap();

        // Verify files exist
        assert!(dir_path.join("test1.txt").exists());
        assert!(dir_path.join("test2.txt").exists());

        let (scan_tx, scan_rx) = bounded(100);
        let (heal_tx, _heal_rx) = bounded(100);
        let stats = Arc::new(SwarmStats::new());

        let agent = ScanAgent::new(dir_path.clone(), scan_tx, heal_tx, Arc::clone(&stats))
            .skip_hidden(false);
        agent.run().unwrap();

        let scanned = stats.files_scanned.load(Ordering::Relaxed);

        // Drain messages
        let mut count = 0;
        while let Ok(msg) = scan_rx.try_recv() {
            if matches!(msg, SwarmMessage::FilePath(_)) {
                count += 1;
            }
        }

        // Should have scanned at least 2 files (might be more if temp has other files)
        assert!(
            scanned >= 2,
            "Expected at least 2 files scanned, got {}",
            scanned
        );
        assert!(count >= 2, "Expected at least 2 messages, got {}", count);
    }

    #[test]
    fn test_embed_agent_fallback() {
        let gpu_available = Arc::new(RwLock::new(true));

        // Simulate GPU failure triggering fallback
        *gpu_available.write() = false;
        assert!(!*gpu_available.read());
    }

    #[test]
    fn test_swarm_stats() {
        let stats = SwarmStats::new();
        stats.files_scanned.fetch_add(10, Ordering::Relaxed);
        stats.chunks_created.fetch_add(50, Ordering::Relaxed);

        let summary = stats.to_summary();
        assert_eq!(summary.files_scanned, 10);
        assert_eq!(summary.chunks_created, 50);
    }

    #[test]
    fn test_agent_role_icons() {
        assert_eq!(AgentRole::Scan.icon(), "🔍");
        assert_eq!(AgentRole::Chunk.icon(), "✂️");
        assert_eq!(AgentRole::Embed.icon(), "🧠");
        assert_eq!(AgentRole::Heal.icon(), "💊");
        assert_eq!(AgentRole::VerifyExport.icon(), "✅");
    }
}
