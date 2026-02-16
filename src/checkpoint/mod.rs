//! Checkpoint module - Resume/checkpoint system for long operations
//!
//! Saves progress to JSON files so indexing, exporting, and dedup operations
//! can be resumed after interruption. Uses serde_json (NOT bincode) for
//! human-debuggable checkpoint files.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::BadSector;

/// Which operation phase this checkpoint covers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckpointPhase {
    /// File indexing / scanning
    Indexing,
    /// File export
    Exporting,
    /// Deduplication analysis
    Dedup,
}

impl std::fmt::Display for CheckpointPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckpointPhase::Indexing => write!(f, "Indexing"),
            CheckpointPhase::Exporting => write!(f, "Exporting"),
            CheckpointPhase::Dedup => write!(f, "Dedup"),
        }
    }
}

/// A checkpoint of operation progress
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Blake3 hash of the source path (for matching)
    pub source_hash: String,
    /// Source path (human-readable)
    pub source_path: String,
    /// Which phase this checkpoint is for
    pub phase: CheckpointPhase,
    /// Paths already processed (skip on resume)
    pub processed_paths: HashSet<String>,
    /// Hashes already computed (reuse on resume)
    pub hashes_computed: HashMap<String, String>,
    /// Bad sectors discovered so far
    pub bad_sectors_found: Vec<BadSector>,
    /// How often to auto-save (every N items)
    pub auto_save_interval: usize,
    /// Items processed since last save
    #[serde(default)]
    pub items_since_save: usize,
    /// When this checkpoint was created
    pub created_at: DateTime<Utc>,
    /// When this checkpoint was last updated
    pub updated_at: DateTime<Utc>,
    /// Checkpoint format version
    pub version: u32,
}

impl Checkpoint {
    const VERSION: u32 = 1;

    /// Create a new checkpoint for the given source and phase
    pub fn new(source: &Path, phase: CheckpointPhase, auto_save_interval: usize) -> Self {
        let source_hash =
            hex::encode(&blake3::hash(source.to_string_lossy().as_bytes()).as_bytes()[..8]);

        Self {
            source_hash,
            source_path: source.to_string_lossy().to_string(),
            phase,
            processed_paths: HashSet::new(),
            hashes_computed: HashMap::new(),
            bad_sectors_found: Vec::new(),
            auto_save_interval,
            items_since_save: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            version: Self::VERSION,
        }
    }

    /// Check if a path has already been processed
    pub fn is_already_processed(&self, path: &str) -> bool {
        self.processed_paths.contains(path)
    }

    /// Mark a path as processed, optionally storing its hash
    pub fn mark_processed(&mut self, path: &str, hash: Option<String>) {
        self.processed_paths.insert(path.to_string());
        if let Some(h) = hash {
            self.hashes_computed.insert(path.to_string(), h);
        }
        self.items_since_save += 1;
        self.updated_at = Utc::now();
    }

    /// Check if we should auto-save based on items processed since last save
    pub fn should_auto_save(&self) -> bool {
        self.auto_save_interval > 0 && self.items_since_save >= self.auto_save_interval
    }

    /// Reset the items-since-save counter (call after saving)
    pub fn reset_save_counter(&mut self) {
        self.items_since_save = 0;
    }

    /// Get the set of processed paths for scanner skip filtering
    pub fn processed_set(&self) -> &HashSet<String> {
        &self.processed_paths
    }

    /// Number of items processed so far
    pub fn processed_count(&self) -> usize {
        self.processed_paths.len()
    }
}

/// Manages checkpoint persistence (load/save/clear)
pub struct CheckpointManager {
    /// Directory where checkpoints are stored
    checkpoint_dir: PathBuf,
}

impl CheckpointManager {
    /// Create a new checkpoint manager
    pub fn new() -> Self {
        let checkpoint_dir = directories::ProjectDirs::from("com", "tunclon", "diamond-drill")
            .map(|dirs| dirs.data_dir().join("checkpoints"))
            .unwrap_or_else(|| PathBuf::from(".diamond-drill-checkpoints"));

        Self { checkpoint_dir }
    }

    /// Create with a custom directory (useful for tests)
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            checkpoint_dir: dir,
        }
    }

    /// Get the checkpoint file path for a source
    fn checkpoint_path(&self, source: &Path, phase: CheckpointPhase) -> PathBuf {
        let hash = hex::encode(&blake3::hash(source.to_string_lossy().as_bytes()).as_bytes()[..8]);
        self.checkpoint_dir.join(format!("{}-{}.json", hash, phase))
    }

    /// Load a checkpoint if one exists for this source and phase
    pub fn load(&self, source: &Path, phase: CheckpointPhase) -> Result<Option<Checkpoint>> {
        let path = self.checkpoint_path(source, phase);

        if !path.exists() {
            return Ok(None);
        }

        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read checkpoint: {}", path.display()))?;

        let checkpoint: Checkpoint = serde_json::from_str(&data)
            .with_context(|| format!("Failed to parse checkpoint: {}", path.display()))?;

        // Version check
        if checkpoint.version != Checkpoint::VERSION {
            tracing::warn!(
                "Checkpoint version mismatch: expected {}, found {}. Starting fresh.",
                Checkpoint::VERSION,
                checkpoint.version
            );
            return Ok(None);
        }

        tracing::info!(
            "Resumed checkpoint: {} items already processed for {} phase",
            checkpoint.processed_count(),
            checkpoint.phase
        );

        Ok(Some(checkpoint))
    }

    /// Save a checkpoint to disk
    pub fn save(&self, checkpoint: &Checkpoint) -> Result<()> {
        // Ensure directory exists
        std::fs::create_dir_all(&self.checkpoint_dir).with_context(|| {
            format!(
                "Failed to create checkpoint dir: {}",
                self.checkpoint_dir.display()
            )
        })?;

        let source_path = PathBuf::from(&checkpoint.source_path);
        let path = self.checkpoint_path(&source_path, checkpoint.phase);

        let data =
            serde_json::to_string_pretty(checkpoint).context("Failed to serialize checkpoint")?;

        std::fs::write(&path, data)
            .with_context(|| format!("Failed to write checkpoint: {}", path.display()))?;

        tracing::debug!(
            "Checkpoint saved: {} items processed",
            checkpoint.processed_count()
        );

        Ok(())
    }

    /// Save checkpoint and reset the save counter
    pub fn auto_save(&self, checkpoint: &mut Checkpoint) -> Result<()> {
        self.save(checkpoint)?;
        checkpoint.reset_save_counter();
        Ok(())
    }

    /// Clear (delete) a checkpoint for the given source and phase
    pub fn clear(&self, source: &Path, phase: CheckpointPhase) -> Result<()> {
        let path = self.checkpoint_path(source, phase);

        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to remove checkpoint: {}", path.display()))?;
            tracing::info!("Checkpoint cleared: {}", path.display());
        }

        Ok(())
    }

    /// Check if a checkpoint exists for the given source and phase
    pub fn exists(&self, source: &Path, phase: CheckpointPhase) -> bool {
        self.checkpoint_path(source, phase).exists()
    }
}

impl Default for CheckpointManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_checkpoint_save_load() {
        let dir = tempdir().unwrap();
        let mgr = CheckpointManager::with_dir(dir.path().to_path_buf());
        let source = PathBuf::from("/test/source");

        let mut cp = Checkpoint::new(&source, CheckpointPhase::Indexing, 100);
        cp.mark_processed("/test/file1.txt", Some("hash1".to_string()));
        cp.mark_processed("/test/file2.txt", None);

        // Save
        mgr.save(&cp).unwrap();

        // Load
        let loaded = mgr
            .load(&source, CheckpointPhase::Indexing)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.processed_count(), 2);
        assert!(loaded.is_already_processed("/test/file1.txt"));
        assert!(loaded.is_already_processed("/test/file2.txt"));
        assert!(!loaded.is_already_processed("/test/file3.txt"));
        assert_eq!(
            loaded.hashes_computed.get("/test/file1.txt").unwrap(),
            "hash1"
        );
        assert!(!loaded.hashes_computed.contains_key("/test/file2.txt"));
    }

    #[test]
    fn test_checkpoint_skip_processed() {
        let source = PathBuf::from("/test");
        let mut cp = Checkpoint::new(&source, CheckpointPhase::Indexing, 100);

        cp.mark_processed("a.txt", None);
        cp.mark_processed("b.txt", None);

        assert!(cp.is_already_processed("a.txt"));
        assert!(cp.is_already_processed("b.txt"));
        assert!(!cp.is_already_processed("c.txt"));
    }

    #[test]
    fn test_checkpoint_auto_save_interval() {
        let source = PathBuf::from("/test");
        let mut cp = Checkpoint::new(&source, CheckpointPhase::Indexing, 3);

        assert!(!cp.should_auto_save());

        cp.mark_processed("1.txt", None);
        assert!(!cp.should_auto_save());

        cp.mark_processed("2.txt", None);
        assert!(!cp.should_auto_save());

        cp.mark_processed("3.txt", None);
        assert!(cp.should_auto_save());

        cp.reset_save_counter();
        assert!(!cp.should_auto_save());
    }

    #[test]
    fn test_checkpoint_clear_on_complete() {
        let dir = tempdir().unwrap();
        let mgr = CheckpointManager::with_dir(dir.path().to_path_buf());
        let source = PathBuf::from("/test/source");

        let cp = Checkpoint::new(&source, CheckpointPhase::Exporting, 100);
        mgr.save(&cp).unwrap();

        assert!(mgr.exists(&source, CheckpointPhase::Exporting));

        mgr.clear(&source, CheckpointPhase::Exporting).unwrap();

        assert!(!mgr.exists(&source, CheckpointPhase::Exporting));
        assert!(mgr
            .load(&source, CheckpointPhase::Exporting)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_checkpoint_no_file_returns_none() {
        let dir = tempdir().unwrap();
        let mgr = CheckpointManager::with_dir(dir.path().to_path_buf());
        let source = PathBuf::from("/nonexistent");

        let result = mgr.load(&source, CheckpointPhase::Indexing).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_checkpoint_different_phases_are_separate() {
        let dir = tempdir().unwrap();
        let mgr = CheckpointManager::with_dir(dir.path().to_path_buf());
        let source = PathBuf::from("/test/source");

        let mut cp_index = Checkpoint::new(&source, CheckpointPhase::Indexing, 100);
        cp_index.mark_processed("index_file.txt", None);
        mgr.save(&cp_index).unwrap();

        let mut cp_export = Checkpoint::new(&source, CheckpointPhase::Exporting, 100);
        cp_export.mark_processed("export_file.txt", None);
        mgr.save(&cp_export).unwrap();

        // Each phase has its own checkpoint
        let loaded_index = mgr
            .load(&source, CheckpointPhase::Indexing)
            .unwrap()
            .unwrap();
        assert!(loaded_index.is_already_processed("index_file.txt"));
        assert!(!loaded_index.is_already_processed("export_file.txt"));

        let loaded_export = mgr
            .load(&source, CheckpointPhase::Exporting)
            .unwrap()
            .unwrap();
        assert!(loaded_export.is_already_processed("export_file.txt"));
        assert!(!loaded_export.is_already_processed("index_file.txt"));
    }
}
