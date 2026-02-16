//! Swarm Agent System - 5-role parallel processing with auto-heal
//!
//! Implements the CaseStar Swarm Guardian pattern with:
//! - ScanAgent: Directory crawl with Rayon par_iter
//! - ChunkAgent: Document splitting with par_chunks
//! - EmbedAgent: Vectorization with GPU/CPU fallback
//! - HealAgent: Retry/fix failures with exponential backoff
//! - VerifyExportAgent: Validation and output generation
//!
//! Enhanced modules:
//! - Session: Persistent state with save/load/resume
//! - Chunker: Media-aware splitting for text/code/image/PDF
//! - Embedder: Adaptive GPU/CPU vector generation
//! - Searcher: Hybrid keyword + vector semantic search

mod agents;
mod chunker;
mod embedder;
mod heal;
mod orchestrator;
mod searcher;
mod session;

pub use agents::*;
pub use chunker::*;
pub use embedder::*;
pub use heal::*;
pub use orchestrator::*;
pub use searcher::*;
pub use session::*;
