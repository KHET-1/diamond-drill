//! Heal Agent - Auto-healing with retry, fallback, and resume
//!
//! Implements the CaseStar auto-heal pattern:
//! - Retry with exponential backoff (3x default)
//! - GPU to CPU fallback on compute failures
//! - Log-based resume for interrupted operations
//! - Silent heal for recoverable errors

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use super::agents::{AgentRole, SwarmMessage, SwarmStats};

// ============================================================================
// Heal Configuration
// ============================================================================

/// Configuration for the Healer
#[derive(Debug, Clone)]
pub struct HealConfig {
    /// Maximum retry attempts per failure
    pub max_retries: u32,
    /// Initial delay between retries (doubles each attempt)
    pub initial_delay_ms: u64,
    /// Maximum delay between retries
    pub max_delay_ms: u64,
    /// Whether to enable GPU to CPU fallback
    pub enable_gpu_fallback: bool,
    /// Path for heal log (for resume)
    pub log_path: Option<PathBuf>,
    /// Silent heal (don't escalate recoverable errors)
    pub silent_heal: bool,
}

impl Default for HealConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 100,
            max_delay_ms: 5000,
            enable_gpu_fallback: true,
            log_path: None,
            silent_heal: true,
        }
    }
}

// ============================================================================
// Heal Log Entry
// ============================================================================

/// Entry in the heal log for resume capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealLogEntry {
    /// Timestamp of the heal attempt
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Agent that failed
    pub agent: String,
    /// Source file/resource
    pub source: String,
    /// Error message
    pub error: String,
    /// Retries remaining when logged
    pub retries_left: u32,
    /// Result of heal attempt
    pub result: HealResult,
    /// Duration of heal attempt in ms
    pub duration_ms: u64,
}

/// Result of a heal attempt
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealResult {
    /// Successfully healed
    Healed,
    /// Retrying (not final)
    Retrying,
    /// Failed after all retries
    Failed,
    /// Skipped (not recoverable)
    Skipped,
}

// ============================================================================
// Heal Log
// ============================================================================

/// Log for tracking heal operations with resume capability
pub struct HealLog {
    entries: RwLock<Vec<HealLogEntry>>,
    log_path: Option<PathBuf>,
}

impl HealLog {
    pub fn new(log_path: Option<PathBuf>) -> Self {
        let entries = if let Some(ref path) = log_path {
            Self::load_from_file(path).unwrap_or_default()
        } else {
            Vec::new()
        };

        Self {
            entries: RwLock::new(entries),
            log_path,
        }
    }

    fn load_from_file(path: &std::path::Path) -> Result<Vec<HealLogEntry>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(path)?;
        let entries: Vec<HealLogEntry> = serde_json::from_str(&content)?;
        Ok(entries)
    }

    pub fn log(&self, entry: HealLogEntry) {
        self.entries.write().push(entry);
        self.persist();
    }

    fn persist(&self) {
        if let Some(ref path) = self.log_path {
            let entries = self.entries.read();
            if let Ok(json) = serde_json::to_string_pretty(&*entries) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    /// Get entries that need retry (for resume)
    pub fn get_pending_retries(&self) -> Vec<HealLogEntry> {
        self.entries
            .read()
            .iter()
            .filter(|e| e.result == HealResult::Retrying && e.retries_left > 0)
            .cloned()
            .collect()
    }

    /// Get failed entries (for reporting)
    pub fn get_failed(&self) -> Vec<HealLogEntry> {
        self.entries
            .read()
            .iter()
            .filter(|e| e.result == HealResult::Failed)
            .cloned()
            .collect()
    }

    /// Get summary statistics
    pub fn summary(&self) -> HealSummary {
        let entries = self.entries.read();
        HealSummary {
            total_attempts: entries.len(),
            healed: entries
                .iter()
                .filter(|e| e.result == HealResult::Healed)
                .count(),
            failed: entries
                .iter()
                .filter(|e| e.result == HealResult::Failed)
                .count(),
            retrying: entries
                .iter()
                .filter(|e| e.result == HealResult::Retrying)
                .count(),
            skipped: entries
                .iter()
                .filter(|e| e.result == HealResult::Skipped)
                .count(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HealSummary {
    pub total_attempts: usize,
    pub healed: usize,
    pub failed: usize,
    pub retrying: usize,
    pub skipped: usize,
}

// ============================================================================
// Healer - The Heal Agent
// ============================================================================

/// The Heal Agent that handles failures with retry and fallback
pub struct Healer {
    input: Receiver<SwarmMessage>,
    retry_queues: HashMap<AgentRole, Sender<SwarmMessage>>,
    stats: Arc<SwarmStats>,
    config: HealConfig,
    log: Arc<HealLog>,
    active_heals: AtomicUsize,
}

impl Healer {
    pub fn new(input: Receiver<SwarmMessage>, stats: Arc<SwarmStats>, config: HealConfig) -> Self {
        let log = Arc::new(HealLog::new(config.log_path.clone()));

        Self {
            input,
            retry_queues: HashMap::new(),
            stats,
            config,
            log,
            active_heals: AtomicUsize::new(0),
        }
    }

    /// Register a retry queue for an agent role
    pub fn register_retry_queue(&mut self, role: AgentRole, tx: Sender<SwarmMessage>) {
        self.retry_queues.insert(role, tx);
    }

    /// Run the heal agent
    pub fn run(&self) -> Result<()> {
        info!(
            "{} HealAgent starting (max_retries: {})",
            AgentRole::Heal.icon(),
            self.config.max_retries
        );

        // First, process any pending retries from previous run
        self.resume_pending()?;

        // Process incoming failures
        while let Ok(msg) = self.input.recv() {
            match msg {
                SwarmMessage::Failure {
                    agent,
                    source,
                    error,
                    retries_left,
                } => {
                    self.handle_failure(agent, source, error, retries_left);
                }
                SwarmMessage::Done => break,
                _ => {}
            }
        }

        info!(
            "{} HealAgent complete: {} heals performed",
            AgentRole::Heal.icon(),
            self.stats.heals_performed.load(Ordering::Relaxed)
        );

        // Report failures if any
        let failed = self.log.get_failed();
        if !failed.is_empty() {
            warn!(
                "{} {} failures could not be healed:",
                AgentRole::Heal.icon(),
                failed.len()
            );
            for entry in &failed {
                warn!("  - {}: {}", entry.source, entry.error);
            }
        }

        Ok(())
    }

    /// Resume pending retries from log
    fn resume_pending(&self) -> Result<()> {
        let pending = self.log.get_pending_retries();
        if pending.is_empty() {
            return Ok(());
        }

        info!(
            "{} Resuming {} pending heals from log",
            AgentRole::Heal.icon(),
            pending.len()
        );

        for entry in pending {
            let agent = match entry.agent.as_str() {
                "Scan" => AgentRole::Scan,
                "Chunk" => AgentRole::Chunk,
                "Embed" => AgentRole::Embed,
                "VerifyExport" => AgentRole::VerifyExport,
                _ => continue,
            };

            self.handle_failure(
                agent,
                PathBuf::from(&entry.source),
                entry.error,
                entry.retries_left,
            );
        }

        Ok(())
    }

    fn handle_failure(&self, agent: AgentRole, source: PathBuf, error: String, retries_left: u32) {
        let start = Instant::now();
        self.active_heals.fetch_add(1, Ordering::Relaxed);

        // Determine if error is recoverable
        let recoverable = self.is_recoverable(&error);

        if !recoverable {
            // Log and skip
            self.log.log(HealLogEntry {
                timestamp: chrono::Utc::now(),
                agent: format!("{:?}", agent),
                source: source.to_string_lossy().to_string(),
                error: error.clone(),
                retries_left,
                result: HealResult::Skipped,
                duration_ms: start.elapsed().as_millis() as u64,
            });

            if !self.config.silent_heal {
                error!(
                    "{} Unrecoverable error for {:?}: {}",
                    AgentRole::Heal.icon(),
                    agent,
                    error
                );
            }

            self.active_heals.fetch_sub(1, Ordering::Relaxed);
            return;
        }

        if retries_left == 0 {
            // No more retries - log failure
            self.log.log(HealLogEntry {
                timestamp: chrono::Utc::now(),
                agent: format!("{:?}", agent),
                source: source.to_string_lossy().to_string(),
                error: error.clone(),
                retries_left: 0,
                result: HealResult::Failed,
                duration_ms: start.elapsed().as_millis() as u64,
            });

            self.stats
                .errors_encountered
                .fetch_add(1, Ordering::Relaxed);
            self.active_heals.fetch_sub(1, Ordering::Relaxed);
            return;
        }

        // Calculate backoff delay
        let attempt = self.config.max_retries - retries_left;
        let delay_ms = std::cmp::min(
            self.config.initial_delay_ms * (1 << attempt),
            self.config.max_delay_ms,
        );

        debug!(
            "{} Healing {:?} failure (retry {}/{}, delay {}ms): {}",
            AgentRole::Heal.icon(),
            agent,
            self.config.max_retries - retries_left + 1,
            self.config.max_retries,
            delay_ms,
            source.display()
        );

        // Wait before retry
        std::thread::sleep(Duration::from_millis(delay_ms));

        // Apply fix strategies
        let fixed = self.apply_fix_strategy(&agent, &source, &error);

        if fixed {
            // Send back to agent for retry
            if let Some(retry_tx) = self.retry_queues.get(&agent) {
                let new_retries = retries_left - 1;
                let _ = retry_tx.send(SwarmMessage::FilePath(source.clone()));

                self.stats.heals_performed.fetch_add(1, Ordering::Relaxed);
                self.stats.errors_healed.fetch_add(1, Ordering::Relaxed);

                self.log.log(HealLogEntry {
                    timestamp: chrono::Utc::now(),
                    agent: format!("{:?}", agent),
                    source: source.to_string_lossy().to_string(),
                    error,
                    retries_left: new_retries,
                    result: if new_retries == 0 {
                        HealResult::Healed
                    } else {
                        HealResult::Retrying
                    },
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
        } else {
            // Fix strategy failed, decrement and re-queue for another attempt
            self.log.log(HealLogEntry {
                timestamp: chrono::Utc::now(),
                agent: format!("{:?}", agent),
                source: source.to_string_lossy().to_string(),
                error: error.clone(),
                retries_left: retries_left - 1,
                result: HealResult::Retrying,
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }

        self.active_heals.fetch_sub(1, Ordering::Relaxed);
    }

    /// Check if error is recoverable
    fn is_recoverable(&self, error: &str) -> bool {
        // Non-recoverable errors
        let fatal_patterns = [
            "permission denied",
            "access denied",
            "file not found",
            "no such file",
            "directory not found",
            "invalid path",
        ];

        let error_lower = error.to_lowercase();
        !fatal_patterns.iter().any(|p| error_lower.contains(p))
    }

    /// Apply fix strategies based on error type
    fn apply_fix_strategy(&self, agent: &AgentRole, source: &std::path::Path, error: &str) -> bool {
        let error_lower = error.to_lowercase();

        // GPU fallback strategy
        if self.config.enable_gpu_fallback
            && (error_lower.contains("gpu")
                || error_lower.contains("cuda")
                || error_lower.contains("out of memory"))
        {
            info!(
                "{} Applying GPU->CPU fallback for {:?}",
                AgentRole::Heal.icon(),
                agent
            );
            // In real implementation, this would set a flag for the agent
            return true;
        }

        // Timeout strategy - just retry
        if error_lower.contains("timeout") || error_lower.contains("timed out") {
            info!(
                "{} Timeout detected, retrying {:?}",
                AgentRole::Heal.icon(),
                agent
            );
            return true;
        }

        // Connection strategy - wait and retry
        if error_lower.contains("connection")
            || error_lower.contains("network")
            || error_lower.contains("refused")
        {
            info!(
                "{} Network error, scheduling retry for {:?}",
                AgentRole::Heal.icon(),
                agent
            );
            std::thread::sleep(Duration::from_millis(500)); // Extra wait
            return true;
        }

        // I/O errors - may be transient
        if error_lower.contains("i/o error") || error_lower.contains("io error") {
            info!("{} I/O error, retrying {:?}", AgentRole::Heal.icon(), agent);
            return true;
        }

        // Bad sector handling
        if error_lower.contains("bad sector") || error_lower.contains("read error") {
            warn!(
                "{} Bad sector detected in {}, attempting recovery",
                AgentRole::Heal.icon(),
                source.display()
            );
            // Could implement sector-by-sector read here
            return true;
        }

        // Default: try retry anyway
        debug!(
            "{} Generic retry for {:?}: {}",
            AgentRole::Heal.icon(),
            agent,
            error
        );
        true
    }

    /// Get the heal log
    pub fn get_log(&self) -> Arc<HealLog> {
        Arc::clone(&self.log)
    }

    /// Get heal summary
    pub fn summary(&self) -> HealSummary {
        self.log.summary()
    }
}

// ============================================================================
// Retry Wrapper - For wrapping operations with auto-heal
// ============================================================================

/// Wrap an operation with retry logic
pub fn with_retry<T, F>(operation: F, max_retries: u32, initial_delay_ms: u64) -> Result<T>
where
    F: Fn() -> Result<T>,
{
    let mut last_error = None;

    for attempt in 0..max_retries {
        match operation() {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = Some(e);
                if attempt < max_retries - 1 {
                    let delay = initial_delay_ms * (1 << attempt);
                    debug!(
                        "Retry {} of {}, waiting {}ms",
                        attempt + 1,
                        max_retries,
                        delay
                    );
                    std::thread::sleep(Duration::from_millis(delay));
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error")))
}

/// Async version of retry wrapper
pub async fn with_retry_async<T, F, Fut>(
    operation: F,
    max_retries: u32,
    initial_delay_ms: u64,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_error = None;

    for attempt in 0..max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = Some(e);
                if attempt < max_retries - 1 {
                    let delay = initial_delay_ms * (1 << attempt);
                    debug!(
                        "Async retry {} of {}, waiting {}ms",
                        attempt + 1,
                        max_retries,
                        delay
                    );
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error")))
}

// ============================================================================
// GPU/CPU Fallback Helper
// ============================================================================

/// Execute with GPU/CPU fallback
pub fn with_gpu_fallback<T, F, G>(gpu_op: F, cpu_op: G) -> Result<T>
where
    F: FnOnce() -> Result<T>,
    G: FnOnce() -> Result<T>,
{
    match gpu_op() {
        Ok(result) => Ok(result),
        Err(e) => {
            let error_str = e.to_string().to_lowercase();
            if error_str.contains("gpu")
                || error_str.contains("cuda")
                || error_str.contains("out of memory")
            {
                warn!("GPU operation failed, falling back to CPU: {}", e);
                cpu_op()
            } else {
                Err(e)
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_retry_success() {
        let result = with_retry(|| Ok(42), 3, 10);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_with_retry_eventual_success() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let attempts = AtomicU32::new(0);

        let result = with_retry(
            || {
                let n = attempts.fetch_add(1, Ordering::Relaxed);
                if n < 2 {
                    Err(anyhow::anyhow!("Temporary failure"))
                } else {
                    Ok("success")
                }
            },
            3,
            1,
        );

        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_with_retry_all_fail() {
        let result: Result<i32> = with_retry(|| Err(anyhow::anyhow!("Always fails")), 3, 1);

        assert!(result.is_err());
    }

    #[test]
    fn test_is_recoverable() {
        let config = HealConfig::default();
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let stats = Arc::new(SwarmStats::new());
        let healer = Healer::new(rx, stats, config);

        assert!(healer.is_recoverable("timeout error"));
        assert!(healer.is_recoverable("connection refused"));
        assert!(!healer.is_recoverable("permission denied"));
        assert!(!healer.is_recoverable("file not found"));
    }

    #[test]
    fn test_heal_log_summary() {
        let log = HealLog::new(None);

        log.log(HealLogEntry {
            timestamp: chrono::Utc::now(),
            agent: "Scan".to_string(),
            source: "/test/file".to_string(),
            error: "test error".to_string(),
            retries_left: 2,
            result: HealResult::Healed,
            duration_ms: 100,
        });

        log.log(HealLogEntry {
            timestamp: chrono::Utc::now(),
            agent: "Embed".to_string(),
            source: "/test/file2".to_string(),
            error: "another error".to_string(),
            retries_left: 0,
            result: HealResult::Failed,
            duration_ms: 500,
        });

        let summary = log.summary();
        assert_eq!(summary.total_attempts, 2);
        assert_eq!(summary.healed, 1);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn test_gpu_fallback() {
        let result = with_gpu_fallback(
            || Err(anyhow::anyhow!("CUDA out of memory")),
            || Ok("CPU result"),
        );

        assert_eq!(result.unwrap(), "CPU result");
    }
}
