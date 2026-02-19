//! Diamond Drill Library
//!
//! Ultra-fast offline disk image recovery tool - indexes, previews,
//! searches, selects and exports files from disk images/clones with
//! extreme speed and safety.
//!
//! # Features
//!
//! - **Parallel Indexing**: Uses rayon for multi-threaded file scanning
//! - **Progressive Thumbnails**: Fast 64x64 preview, background 512x512 upscale
//! - **Read-Only Safe**: Never modifies source data
//! - **Blake3 Verification**: Cryptographic hash verification on exports
//! - **Bad Sector Handling**: Graceful skip with offset logging
//!
//! # Example
//!
//! ```no_run
//! use diamond_drill::core::DrillEngine;
//! use std::path::PathBuf;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create engine for a source path
//!     let engine = DrillEngine::new(PathBuf::from("E:\\Backup")).await?;
//!
//!     // Search for files
//!     let images = engine.get_files_by_type("image").await?;
//!
//!     println!("Found {} images", images.len());
//!     Ok(())
//! }
//! ```

pub mod badsector;
pub mod carve;
pub mod checkpoint;
pub mod cli;
pub mod config;
pub mod core;
pub mod dedup;
pub mod export;
pub mod preview;
pub mod proof;
pub mod readonly;
pub mod spinner;
pub mod swarm;
pub mod tui;

#[cfg(feature = "gui")]
pub mod gui;

// Re-export commonly used types
pub use carve::{CarveOptions, CarveProgress, CarveResult, CarvedFile, Carver};
pub use config::Config;
pub use core::{DrillEngine, FileEntry, FileIndex, FileType};
pub use dedup::{analyze, DedupOptions, DedupReport, DupGroup, KeepStrategy};
pub use export::{ExportOptions, ExportResult, Exporter};
pub use preview::ThumbnailGenerator;
pub use readonly::{
    is_readonly_enforced, open_readonly, run_safety_checks, safe_copy, warn_if_writable,
};
pub use spinner::{DiamondSpinner, PulseProgress, StatusIcons};
pub use swarm::{
    run_swarm, run_swarm_async, run_swarm_with_config, with_gpu_fallback, with_retry,
    with_retry_async, AgentRole, HealConfig, HealResult, Healer, SwarmBuilder, SwarmConfig,
    SwarmOrchestrator, SwarmStats, SwarmSummary,
};
