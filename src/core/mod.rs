//! Core module - The heart of Diamond Drill
//!
//! Contains the main engine, indexing, and file operations.

mod engine;
mod index;
mod scanner;

pub use engine::DrillEngine;
pub use index::{FileEntry, FileIndex, IndexStats};
pub use scanner::{ScanOptions, Scanner};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// File type categories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileType {
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Code,
    Executable,
    Database,
    Other,
}

impl FileType {
    /// Determine file type from extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // Images
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tiff" | "tif"
            | "raw" | "cr2" | "nef" | "arw" | "dng" | "heic" | "heif" => FileType::Image,

            // Videos
            "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpeg" | "mpg"
            | "3gp" | "vob" => FileType::Video,

            // Audio
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "wma" | "aiff" | "opus" | "alac" => {
                FileType::Audio
            }

            // Documents
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "txt" | "md" | "rtf" | "csv" | "epub" | "mobi" => FileType::Document,

            // Archives
            "zip" | "tar" | "gz" | "7z" | "rar" | "bz2" | "xz" | "lz" | "lzma" | "cab" | "iso"
            | "dmg" => FileType::Archive,

            // Code
            "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "c" | "cpp" | "h" | "hpp" | "java"
            | "go" | "rb" | "php" | "swift" | "kt" | "scala" | "cs" | "fs" | "html" | "css"
            | "scss" | "sass" | "less" | "json" | "yaml" | "yml" | "toml" | "xml" | "sql"
            | "sh" | "bash" | "ps1" | "bat" | "cmd" => FileType::Code,

            // Executables
            "exe" | "dll" | "so" | "dylib" | "app" | "msi" | "deb" | "rpm" | "apk" => {
                FileType::Executable
            }

            // Databases
            "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" => FileType::Database,

            _ => FileType::Other,
        }
    }

    /// Get color for terminal display
    pub fn color_code(&self) -> &'static str {
        match self {
            FileType::Image => "\x1b[35m",      // Magenta
            FileType::Video => "\x1b[36m",      // Cyan
            FileType::Audio => "\x1b[33m",      // Yellow
            FileType::Document => "\x1b[32m",   // Green
            FileType::Archive => "\x1b[34m",    // Blue
            FileType::Code => "\x1b[31m",       // Red
            FileType::Executable => "\x1b[91m", // Bright Red
            FileType::Database => "\x1b[94m",   // Bright Blue
            FileType::Other => "\x1b[37m",      // White
        }
    }

    /// Get emoji icon
    pub fn icon(&self) -> &'static str {
        match self {
            FileType::Image => "🖼 ",
            FileType::Video => "🎬",
            FileType::Audio => "🎵",
            FileType::Document => "📄",
            FileType::Archive => "📦",
            FileType::Code => "💻",
            FileType::Executable => "⚡",
            FileType::Database => "🗃 ",
            FileType::Other => "📁",
        }
    }
}

/// Bad sector information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadSector {
    /// File path where bad sector was encountered
    pub file_path: PathBuf,
    /// Byte offset where the bad sector starts
    pub offset: u64,
    /// Length of the bad sector region
    pub length: u64,
    /// Error message
    pub error: String,
    /// Timestamp when detected
    pub detected_at: DateTime<Utc>,
    /// Number of retry attempts before giving up
    #[serde(default)]
    pub retry_count: u8,
    /// Block size used during the read attempt
    #[serde(default = "default_block_size")]
    pub block_size: u64,
}

fn default_block_size() -> u64 {
    4096
}

/// Progress information for callbacks
#[derive(Debug, Clone)]
pub struct Progress {
    /// Total items to process
    pub total: usize,
    /// Completed items
    pub completed: usize,
    /// Current file being processed
    pub current_file: String,
    /// Bytes processed
    pub bytes_processed: u64,
    /// Errors encountered
    pub errors: usize,
    /// Bad sectors found
    pub bad_sectors: usize,
}

impl Progress {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            completed: 0,
            current_file: String::new(),
            bytes_processed: 0,
            errors: 0,
            bad_sectors: 0,
        }
    }

    pub fn percentage(&self) -> f32 {
        if self.total == 0 {
            100.0
        } else {
            (self.completed as f32 / self.total as f32) * 100.0
        }
    }
}
