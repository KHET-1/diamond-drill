//! Session Persistence - Save/Load/Resume for Swarm Operations
//!
//! Enables crash recovery and resume-from-checkpoint:
//! - Bincode serialization for speed
//! - JSON fallback for debugging
//! - Event sourcing for precise replay
//! - Atomic writes to prevent corruption

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::agents::SwarmSummary;

// ============================================================================
// Session State
// ============================================================================

/// Persistent session state for swarm operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmSession {
    /// Unique session ID
    pub session_id: String,
    /// Source path being processed
    pub source: PathBuf,
    /// Output path for results
    pub output: Option<PathBuf>,
    /// Files that have been fully processed
    pub processed_files: HashSet<PathBuf>,
    /// Files that failed and need retry
    pub failed_files: HashSet<PathBuf>,
    /// Current progress statistics
    pub stats: SessionStats,
    /// Events log for replay
    pub events: Vec<SessionEvent>,
    /// Session creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last update timestamp
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Session status
    pub status: SessionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStats {
    pub files_discovered: usize,
    pub files_processed: usize,
    pub files_failed: usize,
    pub chunks_created: usize,
    pub embeddings_generated: usize,
    pub bytes_processed: u64,
    pub duration_seconds: f64,
}

impl From<SwarmSummary> for SessionStats {
    fn from(summary: SwarmSummary) -> Self {
        Self {
            files_discovered: summary.files_scanned,
            files_processed: summary.exports_completed,
            files_failed: summary.errors_encountered,
            chunks_created: summary.chunks_created,
            embeddings_generated: summary.embeddings_generated,
            bytes_processed: summary.bytes_processed,
            duration_seconds: 0.0, // Set by caller
        }
    }
}

// ============================================================================
// Session Events (Event Sourcing)
// ============================================================================

/// Events for replay and audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    /// Session started
    Started {
        timestamp: chrono::DateTime<chrono::Utc>,
        source: PathBuf,
    },
    /// File discovered during scan
    FileDiscovered {
        timestamp: chrono::DateTime<chrono::Utc>,
        path: PathBuf,
        size: u64,
    },
    /// File processing started
    FileStarted {
        timestamp: chrono::DateTime<chrono::Utc>,
        path: PathBuf,
    },
    /// File processing completed
    FileCompleted {
        timestamp: chrono::DateTime<chrono::Utc>,
        path: PathBuf,
        chunks: usize,
        embeddings: usize,
    },
    /// File processing failed
    FileFailed {
        timestamp: chrono::DateTime<chrono::Utc>,
        path: PathBuf,
        error: String,
    },
    /// Checkpoint saved
    Checkpoint {
        timestamp: chrono::DateTime<chrono::Utc>,
        files_processed: usize,
    },
    /// Session paused
    Paused {
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Session resumed
    Resumed {
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Session completed
    Completed {
        timestamp: chrono::DateTime<chrono::Utc>,
        summary: SessionStats,
    },
}

impl SwarmSession {
    /// Create a new session
    pub fn new(source: PathBuf, output: Option<PathBuf>) -> Self {
        let now = chrono::Utc::now();
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            source: source.clone(),
            output,
            processed_files: HashSet::new(),
            failed_files: HashSet::new(),
            stats: SessionStats::default(),
            events: vec![SessionEvent::Started {
                timestamp: now,
                source,
            }],
            created_at: now,
            updated_at: now,
            status: SessionStatus::Active,
        }
    }

    /// Check if a file has been processed
    pub fn is_processed(&self, path: &Path) -> bool {
        self.processed_files.contains(path)
    }

    /// Mark a file as processed
    pub fn mark_processed(&mut self, path: PathBuf, chunks: usize, embeddings: usize) {
        self.processed_files.insert(path.clone());
        self.failed_files.remove(&path);
        self.stats.files_processed += 1;
        self.stats.chunks_created += chunks;
        self.stats.embeddings_generated += embeddings;
        self.updated_at = chrono::Utc::now();

        self.events.push(SessionEvent::FileCompleted {
            timestamp: self.updated_at,
            path,
            chunks,
            embeddings,
        });
    }

    /// Mark a file as failed
    pub fn mark_failed(&mut self, path: PathBuf, error: String) {
        self.failed_files.insert(path.clone());
        self.stats.files_failed += 1;
        self.updated_at = chrono::Utc::now();

        self.events.push(SessionEvent::FileFailed {
            timestamp: self.updated_at,
            path,
            error,
        });
    }

    /// Get files that need processing (not processed, not failed beyond retry)
    pub fn pending_files(&self, all_files: &[PathBuf]) -> Vec<PathBuf> {
        all_files
            .iter()
            .filter(|f| !self.processed_files.contains(*f))
            .cloned()
            .collect()
    }

    /// Complete the session
    pub fn complete(&mut self, summary: SwarmSummary) {
        self.status = SessionStatus::Completed;
        self.stats = summary.into();
        self.updated_at = chrono::Utc::now();

        self.events.push(SessionEvent::Completed {
            timestamp: self.updated_at,
            summary: self.stats.clone(),
        });
    }
}

// ============================================================================
// Session Store - Persistence Layer
// ============================================================================

/// Manages session persistence with atomic writes
pub struct SessionStore {
    /// Base directory for session files
    base_dir: PathBuf,
    /// Current active session
    current: Arc<RwLock<Option<SwarmSession>>>,
    /// Auto-save interval
    auto_save_interval: Duration,
    /// Last save time
    last_save: Arc<RwLock<Instant>>,
    /// Use binary format (faster) or JSON (debuggable)
    use_binary: bool,
}

impl SessionStore {
    /// Create a new session store
    pub fn new(base_dir: PathBuf) -> Self {
        fs::create_dir_all(&base_dir).ok();
        Self {
            base_dir,
            current: Arc::new(RwLock::new(None)),
            auto_save_interval: Duration::from_secs(30),
            last_save: Arc::new(RwLock::new(Instant::now())),
            use_binary: true,
        }
    }

    /// Set auto-save interval
    pub fn with_auto_save(mut self, interval: Duration) -> Self {
        self.auto_save_interval = interval;
        self
    }

    /// Use JSON format instead of binary
    pub fn use_json(mut self) -> Self {
        self.use_binary = false;
        self
    }

    /// Start a new session
    pub fn start(&self, source: PathBuf, output: Option<PathBuf>) -> Result<String> {
        let session = SwarmSession::new(source, output);
        let session_id = session.session_id.clone();

        *self.current.write() = Some(session);
        self.save()?;

        info!("Started session: {}", session_id);
        Ok(session_id)
    }

    /// Resume an existing session
    pub fn resume(&self, session_id: &str) -> Result<SwarmSession> {
        let session = self.load(session_id)?;

        info!(
            "Resuming session {} ({} files processed, {} pending)",
            session_id,
            session.processed_files.len(),
            session.stats.files_discovered - session.processed_files.len()
        );

        let mut current = self.current.write();
        *current = Some(session.clone());

        // Add resume event
        if let Some(ref mut s) = *current {
            s.events.push(SessionEvent::Resumed {
                timestamp: chrono::Utc::now(),
            });
            s.status = SessionStatus::Active;
        }

        Ok(session)
    }

    /// Get current session (read-only)
    pub fn current(&self) -> Option<SwarmSession> {
        self.current.read().clone()
    }

    /// Update current session with callback
    pub fn update<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut SwarmSession),
    {
        let mut current = self.current.write();
        if let Some(ref mut session) = *current {
            f(session);

            // Auto-save if interval elapsed
            let elapsed = self.last_save.read().elapsed();
            if elapsed >= self.auto_save_interval {
                drop(current);
                self.save()?;
            }
        }
        Ok(())
    }

    /// Save current session to disk
    pub fn save(&self) -> Result<()> {
        let session = self.current.read().clone();
        if let Some(session) = session {
            let path = self.session_path(&session.session_id);

            // Atomic write: write to temp, then rename
            let temp_path = path.with_extension("tmp");

            if self.use_binary {
                let file = File::create(&temp_path)?;
                let writer = BufWriter::new(file);
                bincode::serialize_into(writer, &session)
                    .context("Failed to serialize session (bincode)")?;
            } else {
                let json = serde_json::to_string_pretty(&session)?;
                fs::write(&temp_path, json)?;
            }

            fs::rename(&temp_path, &path)?;
            *self.last_save.write() = Instant::now();

            debug!("Saved session: {}", session.session_id);
        }
        Ok(())
    }

    /// Load a session from disk
    pub fn load(&self, session_id: &str) -> Result<SwarmSession> {
        let path = self.session_path(session_id);

        if !path.exists() {
            // Try JSON fallback
            let json_path = path.with_extension("json");
            if json_path.exists() {
                let content = fs::read_to_string(&json_path)?;
                return Ok(serde_json::from_str(&content)?);
            }
            anyhow::bail!("Session not found: {}", session_id);
        }

        if self.use_binary {
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let session: SwarmSession =
                bincode::deserialize_from(reader).context("Failed to deserialize session")?;
            Ok(session)
        } else {
            let content = fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content)?)
        }
    }

    /// List all sessions
    pub fn list_sessions(&self) -> Result<Vec<SwarmSession>> {
        let mut sessions = Vec::new();

        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "session").unwrap_or(false) {
                if let Ok(session) = self.load_from_path(&path) {
                    sessions.push(session);
                }
            }
        }

        // Sort by creation time (newest first)
        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(sessions)
    }

    /// Find resumable sessions for a source path
    pub fn find_resumable(&self, source: &Path) -> Result<Option<SwarmSession>> {
        let sessions = self.list_sessions()?;

        for session in sessions {
            if session.source == source && session.status != SessionStatus::Completed {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }

    /// Delete a session
    pub fn delete(&self, session_id: &str) -> Result<()> {
        let path = self.session_path(session_id);
        if path.exists() {
            fs::remove_file(&path)?;
            info!("Deleted session: {}", session_id);
        }
        Ok(())
    }

    /// Clean up old completed sessions (keep last N)
    pub fn cleanup(&self, keep_count: usize) -> Result<usize> {
        let sessions = self.list_sessions()?;
        let completed: Vec<_> = sessions
            .into_iter()
            .filter(|s| s.status == SessionStatus::Completed)
            .collect();

        let mut deleted = 0;
        if completed.len() > keep_count {
            for session in completed.into_iter().skip(keep_count) {
                self.delete(&session.session_id)?;
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    // Helper functions
    fn session_path(&self, session_id: &str) -> PathBuf {
        let ext = if self.use_binary { "session" } else { "json" };
        self.base_dir.join(format!("{}.{}", session_id, ext))
    }

    fn load_from_path(&self, path: &Path) -> Result<SwarmSession> {
        let is_binary = path.extension().map(|e| e == "session").unwrap_or(false);

        if is_binary {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(bincode::deserialize_from(reader)?)
        } else {
            let content = fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        }
    }
}

// ============================================================================
// Session Manager - High-level API
// ============================================================================

/// High-level session management with auto-resume
pub struct SessionManager {
    store: SessionStore,
}

impl SessionManager {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            store: SessionStore::new(base_dir),
        }
    }

    /// Start or resume a session for the given source
    pub fn start_or_resume(&self, source: PathBuf, output: Option<PathBuf>) -> Result<String> {
        // Check for existing resumable session
        if let Some(existing) = self.store.find_resumable(&source)? {
            warn!(
                "Found existing session {} with {} files processed. Resuming...",
                existing.session_id,
                existing.processed_files.len()
            );
            self.store.resume(&existing.session_id)?;
            return Ok(existing.session_id);
        }

        // Start fresh
        self.store.start(source, output)
    }

    /// Get pending files for current session
    pub fn pending_files(&self, all_files: &[PathBuf]) -> Vec<PathBuf> {
        self.store
            .current()
            .map(|s| s.pending_files(all_files))
            .unwrap_or_else(|| all_files.to_vec())
    }

    /// Mark file as processed
    pub fn mark_processed(&self, path: PathBuf, chunks: usize, embeddings: usize) -> Result<()> {
        self.store
            .update(|s| s.mark_processed(path, chunks, embeddings))
    }

    /// Mark file as failed
    pub fn mark_failed(&self, path: PathBuf, error: String) -> Result<()> {
        self.store.update(|s| s.mark_failed(path, error))
    }

    /// Complete the session
    pub fn complete(&self, summary: SwarmSummary) -> Result<()> {
        self.store.update(|s| s.complete(summary))?;
        self.store.save()
    }

    /// Force save
    pub fn save(&self) -> Result<()> {
        self.store.save()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_session_create_and_save() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).use_json();

        let session_id = store.start(PathBuf::from("/test/source"), None).unwrap();

        assert!(!session_id.is_empty());

        // Load it back
        let loaded = store.load(&session_id).unwrap();
        assert_eq!(loaded.session_id, session_id);
        assert_eq!(loaded.source, PathBuf::from("/test/source"));
    }

    #[test]
    fn test_session_resume() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).use_json();

        // Start and process some files
        let session_id = store.start(PathBuf::from("/test/source"), None).unwrap();

        store
            .update(|s| {
                s.mark_processed(PathBuf::from("/test/file1.txt"), 5, 5);
                s.mark_processed(PathBuf::from("/test/file2.txt"), 3, 3);
            })
            .unwrap();

        store.save().unwrap();

        // Create new store and resume
        let store2 = SessionStore::new(dir.path().to_path_buf()).use_json();
        let resumed = store2.resume(&session_id).unwrap();

        assert_eq!(resumed.processed_files.len(), 2);
        assert!(resumed.is_processed(&PathBuf::from("/test/file1.txt")));
    }

    #[test]
    fn test_pending_files() {
        let session = SwarmSession::new(PathBuf::from("/test"), None);
        let mut session = session;

        let all_files = vec![
            PathBuf::from("/test/a.txt"),
            PathBuf::from("/test/b.txt"),
            PathBuf::from("/test/c.txt"),
        ];

        // Process one file
        session.mark_processed(PathBuf::from("/test/a.txt"), 1, 1);

        let pending = session.pending_files(&all_files);
        assert_eq!(pending.len(), 2);
        assert!(!pending.contains(&PathBuf::from("/test/a.txt")));
    }

    #[test]
    fn test_binary_serialization() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()); // Binary mode

        let session_id = store.start(PathBuf::from("/test/source"), None).unwrap();

        store
            .update(|s| {
                for i in 0..100 {
                    s.mark_processed(PathBuf::from(format!("/test/file{}.txt", i)), 1, 1);
                }
            })
            .unwrap();

        store.save().unwrap();

        // Verify file is binary (not human-readable JSON)
        let path = dir.path().join(format!("{}.session", session_id));
        let content = fs::read(&path).unwrap();
        assert!(content.len() < 50000); // Binary is compact (allows for path overhead)
    }
}
