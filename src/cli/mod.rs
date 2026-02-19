//! CLI module - Command line interface definitions and handlers

pub mod easy_mode;
pub mod interactive;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Diamond Drill - Ultra-fast offline disk image recovery tool
///
/// Indexes, previews, searches, selects and exports files from disk images
/// with extreme speed and safety. All operations are READ-ONLY.
#[derive(Parser, Debug)]
#[command(name = "diamond-drill")]
#[command(author = "Ryan Cashmoney <tunclon@proton.me>")]
#[command(version)]
#[command(about = "💎 Ultra-fast offline disk image recovery tool", long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Enable grandma mode - simplified interactive workflow
    #[arg(long, short = 'E', global = true)]
    pub easy: bool,

    /// Verbose output
    #[arg(long, short, global = true)]
    pub verbose: bool,

    /// Output format for machine parsing
    #[arg(long, value_enum, global = true)]
    pub output: Option<OutputFormat>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Index a disk image or directory
    Index(IndexArgs),

    /// Search indexed files
    Search(SearchArgs),

    /// Preview files (images get thumbnails)
    Preview(PreviewArgs),

    /// Export selected files with blake3 verification
    Export(ExportArgs),

    /// Interactive TUI mode (default)
    Interactive(InteractiveArgs),

    /// Carve files from raw disk image by signature scanning
    Carve(CarveArgs),

    /// Find and manage duplicate files
    Dedup(DedupArgs),

    /// Verify a proof manifest against exported files
    Verify(VerifyArgs),

    /// Launch TUI mode (terminal UI with vim keybindings)
    Tui(TuiArgs),

    /// Run the 5-agent swarm pipeline for parallel document processing
    Swarm(SwarmArgs),

    /// Launch GUI mode (requires --features gui)
    #[cfg(feature = "gui")]
    Gui(GuiArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct IndexArgs {
    /// Source path - disk image, mounted volume, or directory
    #[arg(required = true)]
    pub source: PathBuf,

    /// Resume from previous index state
    #[arg(long, short)]
    pub resume: bool,

    /// Index file path (default: ~/.diamond-drill/indexes/<hash>.idx)
    #[arg(long, short)]
    pub index_file: Option<PathBuf>,

    /// Skip hidden files
    #[arg(long)]
    pub skip_hidden: bool,

    /// Maximum depth to traverse
    #[arg(long, short)]
    pub depth: Option<usize>,

    /// File extensions to include (e.g., jpg,png,pdf)
    #[arg(long, short = 'e', value_delimiter = ',')]
    pub extensions: Option<Vec<String>>,

    /// Generate thumbnails during indexing
    #[arg(long, short)]
    pub thumbnails: bool,

    /// Number of parallel workers (default: CPU count)
    #[arg(long, short)]
    pub workers: Option<usize>,

    /// Auto-save checkpoint every N files (0 = disabled)
    #[arg(long, default_value = "1000")]
    pub checkpoint_interval: usize,

    /// Write bad sector report to this path (JSON or human based on extension)
    #[arg(long)]
    pub bad_sector_report: Option<PathBuf>,

    /// Block size for bad sector detection in bytes (default: 4096)
    #[arg(long, default_value = "4096")]
    pub block_size: usize,
}

#[derive(Debug, Clone, Parser)]
pub struct SearchArgs {
    /// Source path or index file
    #[arg(required = true)]
    pub source: PathBuf,

    /// Search pattern (glob, regex with /pattern/, or fuzzy)
    #[arg(required = true)]
    pub pattern: String,

    /// Search type
    #[arg(long, value_enum, default_value = "fuzzy")]
    pub search_type: SearchType,

    /// Filter by file type
    #[arg(long, short, value_enum)]
    pub file_type: Option<FileTypeFilter>,

    /// Minimum file size
    #[arg(long)]
    pub min_size: Option<String>,

    /// Maximum file size
    #[arg(long)]
    pub max_size: Option<String>,

    /// Modified after date (YYYY-MM-DD)
    #[arg(long)]
    pub after: Option<String>,

    /// Modified before date (YYYY-MM-DD)
    #[arg(long)]
    pub before: Option<String>,

    /// Maximum results
    #[arg(long, short, default_value = "100")]
    pub limit: usize,
}

#[derive(Debug, Clone, Parser)]
pub struct PreviewArgs {
    /// Source path or index file
    #[arg(required = true)]
    pub source: PathBuf,

    /// Files to preview (paths or search pattern)
    pub files: Vec<String>,

    /// Thumbnail size (64, 128, 256, 512)
    #[arg(long, default_value = "256")]
    pub thumb_size: u32,

    /// Output directory for thumbnails
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Parser, Default)]
pub struct InteractiveArgs {
    /// Source path to start with
    pub source: Option<PathBuf>,

    /// State file to resume from
    #[arg(long)]
    pub state: Option<PathBuf>,

    /// Color theme (dark, light, auto)
    #[arg(long, default_value = "auto")]
    pub theme: String,
}

#[derive(Debug, Clone, Parser)]
pub struct ExportArgs {
    /// Source path or index file
    #[arg(required = true)]
    pub source: PathBuf,

    /// Destination directory
    #[arg(required = true)]
    pub dest: PathBuf,

    /// Files to export (paths, patterns, or "selected" for marked files)
    pub files: Vec<String>,

    /// Preserve directory structure
    #[arg(long, short)]
    pub preserve_structure: bool,

    /// Skip hash verification (faster but less safe)
    #[arg(long)]
    pub no_verify: bool,

    /// Continue on errors (log and skip bad files)
    #[arg(long, short)]
    pub continue_on_error: bool,

    /// Dry run - show what would be exported
    #[arg(long, short = 'n')]
    pub dry_run: bool,

    /// Create manifest file with hashes
    #[arg(long, short)]
    pub manifest: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct CarveArgs {
    /// Source raw disk image (dd, img, iso, or block device)
    #[arg(required = true)]
    pub source: PathBuf,

    /// Output directory for carved files
    #[arg(required = true)]
    pub output: PathBuf,

    /// Scan aligned to 512-byte sectors (faster for disk images)
    #[arg(long, default_value = "true")]
    pub sector_aligned: bool,

    /// Minimum file size to extract (e.g., 1KB, 512)
    #[arg(long, default_value = "512")]
    pub min_size: String,

    /// Only carve specific file types
    #[arg(long, short, value_enum, value_delimiter = ',')]
    pub file_type: Option<Vec<FileTypeFilter>>,

    /// Number of parallel workers (default: CPU count)
    #[arg(long, short)]
    pub workers: Option<usize>,

    /// Dry run - scan and report without extracting
    #[arg(long, short = 'n')]
    pub dry_run: bool,

    /// Skip file type verification with infer
    #[arg(long)]
    pub no_verify: bool,

    /// Output format (human, json)
    #[arg(long, value_enum)]
    pub output_format: Option<OutputFormat>,
}

#[cfg(feature = "gui")]
#[derive(Debug, Clone, Parser)]
pub struct GuiArgs {
    /// Source path to open
    pub source: Option<PathBuf>,

    /// Window size (WxH)
    #[arg(long, default_value = "1280x800")]
    pub size: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    /// Human readable (default)
    Human,
    /// JSON output
    Json,
    /// CSV output
    Csv,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SearchType {
    /// Fuzzy matching (typo-tolerant)
    Fuzzy,
    /// Glob patterns (*.jpg, photo_*)
    Glob,
    /// Regular expressions (/pattern/)
    Regex,
    /// Exact match
    Exact,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum FileTypeFilter {
    /// Images (jpg, png, gif, webp, etc.)
    Image,
    /// Videos (mp4, avi, mkv, etc.)
    Video,
    /// Audio (mp3, flac, wav, etc.)
    Audio,
    /// Documents (pdf, doc, txt, etc.)
    Document,
    /// Archives (zip, tar, 7z, etc.)
    Archive,
    /// Code files (rs, py, js, etc.)
    Code,
    /// All files
    #[default]
    All,
}

#[derive(Debug, Clone, Parser)]
pub struct DedupArgs {
    /// Source path or index file to scan for duplicates
    #[arg(required = true)]
    pub source: PathBuf,

    /// Keep strategy: newest, largest, oldest, cleanest
    #[arg(long, short, value_enum, default_value = "newest")]
    pub keep: DedupKeepStrategy,

    /// Enable fuzzy (near-duplicate) detection
    #[arg(long, short)]
    pub fuzzy: bool,

    /// Fuzzy similarity threshold 0–100 (default 85)
    #[arg(long, default_value = "85")]
    pub threshold: u8,

    /// Minimum file size to consider (bytes)
    #[arg(long, default_value = "1")]
    pub min_size: u64,

    /// Actually delete duplicate files (default: dry run report only)
    #[arg(long)]
    pub purge: bool,

    /// Output format for report
    #[arg(long, value_enum, default_value = "human")]
    pub report: DedupReportFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DedupKeepStrategy {
    /// Keep the most recently modified file
    Newest,
    /// Keep the largest file
    Largest,
    /// Keep the oldest file
    Oldest,
    /// Keep the file with the cleanest name (no _backup, .bak, etc.)
    Cleanest,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DedupReportFormat {
    /// Human-readable table
    Human,
    /// JSON output
    Json,
}

#[derive(Debug, Clone, Parser)]
pub struct VerifyArgs {
    /// Path to the proof manifest file (JSON)
    #[arg(required = true)]
    pub manifest: PathBuf,

    /// Output format for verification report
    #[arg(long, value_enum, default_value = "human")]
    pub report: VerifyReportFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum VerifyReportFormat {
    /// Human-readable report
    Human,
    /// JSON output
    Json,
}

#[derive(Debug, Clone, Parser)]
pub struct TuiArgs {
    /// Source path to index and browse
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, Parser)]
pub struct SwarmArgs {
    /// Source path - directory to process with swarm
    #[arg(required = true)]
    pub source: PathBuf,

    /// Output path for export manifest
    #[arg(long, short)]
    pub output: Option<PathBuf>,

    /// File extensions to process (e.g., pdf,txt,docx)
    #[arg(long, short = 'e', value_delimiter = ',')]
    pub extensions: Option<Vec<String>>,

    /// Chunk size in bytes
    #[arg(long, default_value = "1024")]
    pub chunk_size: usize,

    /// Chunk overlap in bytes
    #[arg(long, default_value = "128")]
    pub chunk_overlap: usize,

    /// Maximum retry attempts for failed operations
    #[arg(long, default_value = "3")]
    pub max_retries: u32,

    /// Skip hidden files and directories
    #[arg(long)]
    pub skip_hidden: bool,

    /// Enable silent heal (suppress recoverable error logs)
    #[arg(long)]
    pub silent_heal: bool,

    /// Path to save heal log for resume capability
    #[arg(long)]
    pub heal_log: Option<PathBuf>,

    /// Enable GPU to CPU fallback on compute failures
    #[arg(long)]
    pub gpu_fallback: bool,

    /// Output format for report
    #[arg(long, value_enum, default_value = "human")]
    pub report: SwarmReportFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SwarmReportFormat {
    /// Human-readable output
    Human,
    /// JSON output
    Json,
}
