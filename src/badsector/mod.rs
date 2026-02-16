//! Bad Sector module - Enhanced sector-level error detection and reporting
//!
//! Provides block-level file reading with retry logic, exponential backoff,
//! and detailed error tracking for disk recovery operations.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Default block size for sector reads (4KB)
pub const DEFAULT_BLOCK_SIZE: usize = 4096;

/// Maximum retry attempts for transient I/O errors
pub const MAX_RETRIES: u8 = 3;

/// Base delay for exponential backoff (100ms)
const BASE_DELAY_MS: u64 = 100;

/// Status of a single block in the heatmap
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockStatus {
    /// Block was read successfully
    Good,
    /// Block had a permanent read error
    Bad,
    /// Block was skipped (e.g., beyond file boundary)
    Skipped,
}

/// Information about a single bad block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfo {
    /// Byte offset of the block start
    pub offset: u64,
    /// Length of the block
    pub length: u64,
    /// Error message
    pub error: String,
    /// Number of retry attempts before giving up
    pub retry_count: u8,
}

/// Map of all blocks in a file with their read status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorMap {
    /// File path
    pub path: PathBuf,
    /// Total number of blocks
    pub total_blocks: u64,
    /// Bad blocks with details
    pub bad_blocks: Vec<BlockInfo>,
    /// Total readable bytes
    pub good_bytes: u64,
    /// Total unreadable bytes
    pub bad_bytes: u64,
    /// File size
    pub file_size: u64,
    /// Block size used for scanning
    pub block_size: usize,
}

impl SectorMap {
    /// Check if the file has any bad sectors
    pub fn has_bad_sectors(&self) -> bool {
        !self.bad_blocks.is_empty()
    }

    /// Get percentage of file that is readable
    pub fn readable_percent(&self) -> f64 {
        if self.file_size == 0 {
            return 100.0;
        }
        (self.good_bytes as f64 / self.file_size as f64) * 100.0
    }

    /// Generate heatmap data for TUI visualization
    pub fn heatmap(&self) -> HeatMapData {
        let mut blocks = vec![BlockStatus::Good; self.total_blocks as usize];

        for bad in &self.bad_blocks {
            let block_idx = (bad.offset / self.block_size as u64) as usize;
            if block_idx < blocks.len() {
                blocks[block_idx] = BlockStatus::Bad;
            }
        }

        HeatMapData { blocks }
    }
}

/// Heatmap data for TUI visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatMapData {
    /// Status of each block (ordered by offset)
    pub blocks: Vec<BlockStatus>,
}

impl HeatMapData {
    /// Get a summary string for terminal display (compact)
    pub fn summary_bar(&self, width: usize) -> String {
        if self.blocks.is_empty() {
            return String::new();
        }

        let blocks_per_char = (self.blocks.len() as f64 / width as f64).ceil() as usize;
        let blocks_per_char = blocks_per_char.max(1);

        let mut bar = String::with_capacity(width);
        for chunk in self.blocks.chunks(blocks_per_char) {
            let has_bad = chunk.contains(&BlockStatus::Bad);
            if has_bad {
                bar.push('\u{2588}'); // Full block (bad)
            } else {
                bar.push('\u{2591}'); // Light shade (good)
            }
        }

        bar
    }
}

/// Comprehensive bad sector report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadSectorReport {
    /// Source path scanned
    pub source: PathBuf,
    /// When the scan was performed
    pub scan_time: DateTime<Utc>,
    /// Total files scanned
    pub total_files_scanned: usize,
    /// Files with bad sectors
    pub files_with_bad_sectors: usize,
    /// Total bad blocks across all files
    pub total_bad_blocks: u64,
    /// Total unreadable bytes
    pub total_bad_bytes: u64,
    /// Per-file sector maps (only for files with bad sectors)
    pub files: Vec<SectorMap>,
}

impl BadSectorReport {
    /// Generate a human-readable report
    pub fn to_human_string(&self) -> String {
        let mut out = String::new();

        out.push_str("\n  Diamond Drill Bad Sector Report\n");
        out.push_str("  ========================================\n\n");
        out.push_str(&format!("  Source:          {}\n", self.source.display()));
        out.push_str(&format!(
            "  Scan time:       {}\n",
            self.scan_time.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        out.push_str(&format!(
            "  Files scanned:   {}\n",
            self.total_files_scanned
        ));
        out.push_str(&format!(
            "  Files affected:  {}\n",
            self.files_with_bad_sectors
        ));
        out.push_str(&format!("  Bad blocks:      {}\n", self.total_bad_blocks));
        out.push_str(&format!(
            "  Unreadable:      {}\n\n",
            humansize::format_size(self.total_bad_bytes, humansize::BINARY)
        ));

        if self.files.is_empty() {
            out.push_str("  No bad sectors found. All files are clean.\n");
        } else {
            for (i, map) in self.files.iter().enumerate() {
                out.push_str(&format!(
                    "  File #{} - {} ({:.1}% readable)\n",
                    i + 1,
                    map.path.display(),
                    map.readable_percent()
                ));
                out.push_str(&format!(
                    "    Size: {} | Bad blocks: {} | Bad bytes: {}\n",
                    humansize::format_size(map.file_size, humansize::BINARY),
                    map.bad_blocks.len(),
                    humansize::format_size(map.bad_bytes, humansize::BINARY),
                ));

                for block in &map.bad_blocks {
                    out.push_str(&format!(
                        "    [offset 0x{:08X}, {} bytes, {} retries] {}\n",
                        block.offset, block.length, block.retry_count, block.error
                    ));
                }

                // Show heatmap
                let heatmap = map.heatmap();
                let bar = heatmap.summary_bar(40);
                out.push_str(&format!("    Heatmap: [{}]\n\n", bar));
            }
        }

        out
    }
}

/// Reads a file block-by-block with retry logic for bad sector detection
pub struct SectorReader {
    block_size: usize,
    max_retries: u8,
}

impl SectorReader {
    /// Create a new sector reader with default settings
    pub fn new() -> Self {
        Self {
            block_size: DEFAULT_BLOCK_SIZE,
            max_retries: MAX_RETRIES,
        }
    }

    /// Create with custom block size
    pub fn with_block_size(block_size: usize) -> Self {
        Self {
            block_size: block_size.max(512), // minimum 512 bytes
            max_retries: MAX_RETRIES,
        }
    }

    /// Read a file with sector-level tracking
    ///
    /// Returns a SectorMap with all bad block locations.
    /// For files that are entirely readable, the bad_blocks vec will be empty.
    pub fn read_with_sector_tracking(&self, path: &Path) -> Result<SectorMap> {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Failed to get metadata for {}", path.display()))?;

        let file_size = metadata.len();

        if file_size == 0 {
            return Ok(SectorMap {
                path: path.to_path_buf(),
                total_blocks: 0,
                bad_blocks: Vec::new(),
                good_bytes: 0,
                bad_bytes: 0,
                file_size: 0,
                block_size: self.block_size,
            });
        }

        let total_blocks = file_size.div_ceil(self.block_size as u64);
        let mut bad_blocks = Vec::new();
        let mut good_bytes = 0u64;
        let mut bad_bytes = 0u64;

        let mut file = std::fs::File::open(path)
            .with_context(|| format!("Failed to open {}", path.display()))?;

        let mut buffer = vec![0u8; self.block_size];

        for block_idx in 0..total_blocks {
            let offset = block_idx * self.block_size as u64;
            let remaining = file_size - offset;
            let read_size = remaining.min(self.block_size as u64) as usize;

            match self.read_block_with_retry(&mut file, offset, &mut buffer[..read_size]) {
                Ok(()) => {
                    good_bytes += read_size as u64;
                }
                Err((error, retry_count)) => {
                    bad_bytes += read_size as u64;
                    bad_blocks.push(BlockInfo {
                        offset,
                        length: read_size as u64,
                        error,
                        retry_count,
                    });
                }
            }
        }

        Ok(SectorMap {
            path: path.to_path_buf(),
            total_blocks,
            bad_blocks,
            good_bytes,
            bad_bytes,
            file_size,
            block_size: self.block_size,
        })
    }

    /// Read a single block with retry and exponential backoff
    ///
    /// Returns Ok(()) if the block was read successfully.
    /// Returns Err((error_message, retry_count)) on permanent failure.
    fn read_block_with_retry(
        &self,
        file: &mut std::fs::File,
        offset: u64,
        buf: &mut [u8],
    ) -> std::result::Result<(), (String, u8)> {
        for attempt in 0..self.max_retries {
            // Seek to position
            if let Err(e) = file.seek(SeekFrom::Start(offset)) {
                if Self::is_transient_error(&e) && attempt < self.max_retries - 1 {
                    let delay = Duration::from_millis(BASE_DELAY_MS * (4u64.pow(attempt as u32)));
                    std::thread::sleep(delay);
                    continue;
                }
                return Err((e.to_string(), attempt + 1));
            }

            // Read the block
            match file.read_exact(buf) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        // File is shorter than expected — not a bad sector
                        // Fill remaining with zeros
                        return Ok(());
                    }

                    if Self::is_transient_error(&e) && attempt < self.max_retries - 1 {
                        let delay =
                            Duration::from_millis(BASE_DELAY_MS * (4u64.pow(attempt as u32)));
                        std::thread::sleep(delay);
                        continue;
                    }

                    return Err((e.to_string(), attempt + 1));
                }
            }
        }

        Err(("Max retries exhausted".to_string(), self.max_retries))
    }

    /// Check if an I/O error is transient (worth retrying)
    fn is_transient_error(e: &std::io::Error) -> bool {
        matches!(
            e.kind(),
            std::io::ErrorKind::Interrupted
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::WouldBlock
        )
    }
}

impl Default for SectorReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Export a file with bad sector handling — copies readable blocks, zero-fills bad ones
pub fn export_with_bad_sector_handling(
    source: &Path,
    dest: &Path,
    sector_map: &SectorMap,
) -> Result<ExportBadSectorResult> {
    use std::io::Write;

    // Ensure parent directory exists
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut src_file = std::fs::File::open(source)
        .with_context(|| format!("Failed to open source: {}", source.display()))?;

    let mut dst_file = std::fs::File::create(dest)
        .with_context(|| format!("Failed to create dest: {}", dest.display()))?;

    let mut buffer = vec![0u8; sector_map.block_size];
    let zero_buffer = vec![0u8; sector_map.block_size];
    let mut bytes_copied = 0u64;
    let mut bytes_zeroed = 0u64;
    let mut hasher = blake3::Hasher::new();

    // Build a set of bad block offsets for O(1) lookup
    let bad_offsets: std::collections::HashSet<u64> =
        sector_map.bad_blocks.iter().map(|b| b.offset).collect();

    for block_idx in 0..sector_map.total_blocks {
        let offset = block_idx * sector_map.block_size as u64;
        let remaining = sector_map.file_size - offset;
        let read_size = remaining.min(sector_map.block_size as u64) as usize;

        if bad_offsets.contains(&offset) {
            // Zero-fill this block
            dst_file.write_all(&zero_buffer[..read_size])?;
            hasher.update(&zero_buffer[..read_size]);
            bytes_zeroed += read_size as u64;
        } else {
            // Copy the block
            src_file.seek(SeekFrom::Start(offset))?;
            match src_file.read_exact(&mut buffer[..read_size]) {
                Ok(()) => {
                    dst_file.write_all(&buffer[..read_size])?;
                    hasher.update(&buffer[..read_size]);
                    bytes_copied += read_size as u64;
                }
                Err(_) => {
                    // Unexpected error on previously-good block — zero-fill
                    dst_file.write_all(&zero_buffer[..read_size])?;
                    hasher.update(&zero_buffer[..read_size]);
                    bytes_zeroed += read_size as u64;
                }
            }
        }
    }

    dst_file.flush()?;

    let hash = hex::encode(hasher.finalize().as_bytes());

    Ok(ExportBadSectorResult {
        bytes_copied,
        bytes_zeroed,
        total_bytes: sector_map.file_size,
        blake3_hash: hash,
    })
}

/// Result of exporting a file with bad sector handling
#[derive(Debug, Clone)]
pub struct ExportBadSectorResult {
    /// Bytes successfully copied from source
    pub bytes_copied: u64,
    /// Bytes zero-filled due to bad sectors
    pub bytes_zeroed: u64,
    /// Total file size
    pub total_bytes: u64,
    /// Blake3 hash of the output (including zero-filled regions)
    pub blake3_hash: String,
}

/// Generate a report from multiple sector maps
pub fn generate_report(source: &Path, maps: &[SectorMap], total_scanned: usize) -> BadSectorReport {
    let files_with_bad: Vec<SectorMap> = maps
        .iter()
        .filter(|m| m.has_bad_sectors())
        .cloned()
        .collect();

    let total_bad_blocks: u64 = files_with_bad
        .iter()
        .map(|m| m.bad_blocks.len() as u64)
        .sum();
    let total_bad_bytes: u64 = files_with_bad.iter().map(|m| m.bad_bytes).sum();

    BadSectorReport {
        source: source.to_path_buf(),
        scan_time: Utc::now(),
        total_files_scanned: total_scanned,
        files_with_bad_sectors: files_with_bad.len(),
        total_bad_blocks,
        total_bad_bytes,
        files: files_with_bad,
    }
}

/// Write a bad sector report to disk
pub fn write_report(report: &BadSectorReport, path: &Path, json: bool) -> Result<()> {
    let content = if json {
        serde_json::to_string_pretty(report)?
    } else {
        report.to_human_string()
    };

    std::fs::write(path, content)
        .with_context(|| format!("Failed to write report to {}", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sector_reader_clean_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clean.txt");
        std::fs::write(&path, "Hello, this is a clean file with no bad sectors!").unwrap();

        let reader = SectorReader::new();
        let map = reader.read_with_sector_tracking(&path).unwrap();

        assert!(!map.has_bad_sectors());
        assert_eq!(map.bad_blocks.len(), 0);
        assert!(map.good_bytes > 0);
        assert_eq!(map.bad_bytes, 0);
        assert_eq!(map.readable_percent(), 100.0);
    }

    #[test]
    fn test_sector_reader_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();

        let reader = SectorReader::new();
        let map = reader.read_with_sector_tracking(&path).unwrap();

        assert_eq!(map.total_blocks, 0);
        assert_eq!(map.good_bytes, 0);
        assert_eq!(map.bad_bytes, 0);
    }

    #[test]
    fn test_sector_map_heatmap() {
        let map = SectorMap {
            path: PathBuf::from("/test"),
            total_blocks: 10,
            bad_blocks: vec![
                BlockInfo {
                    offset: 4096 * 2,
                    length: 4096,
                    error: "Read error".to_string(),
                    retry_count: 3,
                },
                BlockInfo {
                    offset: 4096 * 7,
                    length: 4096,
                    error: "Read error".to_string(),
                    retry_count: 3,
                },
            ],
            good_bytes: 8 * 4096,
            bad_bytes: 2 * 4096,
            file_size: 10 * 4096,
            block_size: 4096,
        };

        let heatmap = map.heatmap();
        assert_eq!(heatmap.blocks.len(), 10);
        assert_eq!(heatmap.blocks[0], BlockStatus::Good);
        assert_eq!(heatmap.blocks[2], BlockStatus::Bad);
        assert_eq!(heatmap.blocks[7], BlockStatus::Bad);
        assert_eq!(heatmap.blocks[9], BlockStatus::Good);
    }

    #[test]
    fn test_heatmap_summary_bar() {
        let heatmap = HeatMapData {
            blocks: vec![
                BlockStatus::Good,
                BlockStatus::Good,
                BlockStatus::Bad,
                BlockStatus::Good,
                BlockStatus::Good,
            ],
        };

        let bar = heatmap.summary_bar(5);
        assert_eq!(bar.chars().count(), 5);
    }

    #[test]
    fn test_export_with_bad_sector_handling() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let dest = dir.path().join("dest.bin");

        // Create a file with known content
        let content = vec![0xAA; 8192]; // 2 blocks of 4KB
        std::fs::write(&source, &content).unwrap();

        // Simulate a bad sector map (block 1 is bad)
        let map = SectorMap {
            path: source.clone(),
            total_blocks: 2,
            bad_blocks: vec![BlockInfo {
                offset: 4096,
                length: 4096,
                error: "Simulated bad sector".to_string(),
                retry_count: 3,
            }],
            good_bytes: 4096,
            bad_bytes: 4096,
            file_size: 8192,
            block_size: 4096,
        };

        let result = export_with_bad_sector_handling(&source, &dest, &map).unwrap();

        assert_eq!(result.bytes_copied, 4096); // First block copied
        assert_eq!(result.bytes_zeroed, 4096); // Second block zero-filled
        assert_eq!(result.total_bytes, 8192);
        assert!(!result.blake3_hash.is_empty());

        // Verify dest file contents
        let dest_content = std::fs::read(&dest).unwrap();
        assert_eq!(dest_content.len(), 8192);
        assert!(dest_content[..4096].iter().all(|&b| b == 0xAA)); // First block preserved
        assert!(dest_content[4096..].iter().all(|&b| b == 0x00)); // Second block zeroed
    }

    #[test]
    fn test_bad_sector_report_human() {
        let report = BadSectorReport {
            source: PathBuf::from("/test/drive"),
            scan_time: Utc::now(),
            total_files_scanned: 100,
            files_with_bad_sectors: 1,
            total_bad_blocks: 3,
            total_bad_bytes: 12288,
            files: vec![SectorMap {
                path: PathBuf::from("/test/drive/corrupted.bin"),
                total_blocks: 10,
                bad_blocks: vec![BlockInfo {
                    offset: 8192,
                    length: 4096,
                    error: "I/O error".to_string(),
                    retry_count: 3,
                }],
                good_bytes: 36864,
                bad_bytes: 4096,
                file_size: 40960,
                block_size: 4096,
            }],
        };

        let text = report.to_human_string();
        assert!(text.contains("Bad Sector Report"));
        assert!(text.contains("Files affected:  1"));
        assert!(text.contains("corrupted.bin"));
        assert!(text.contains("I/O error"));
    }

    #[test]
    fn test_bad_sector_report_json() {
        let report = BadSectorReport {
            source: PathBuf::from("/test"),
            scan_time: Utc::now(),
            total_files_scanned: 50,
            files_with_bad_sectors: 0,
            total_bad_blocks: 0,
            total_bad_bytes: 0,
            files: vec![],
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("\"total_files_scanned\": 50"));
        assert!(json.contains("\"files_with_bad_sectors\": 0"));
    }

    #[test]
    fn test_generate_report() {
        let maps = vec![
            SectorMap {
                path: PathBuf::from("/clean.txt"),
                total_blocks: 5,
                bad_blocks: vec![],
                good_bytes: 5 * 4096,
                bad_bytes: 0,
                file_size: 5 * 4096,
                block_size: 4096,
            },
            SectorMap {
                path: PathBuf::from("/bad.txt"),
                total_blocks: 10,
                bad_blocks: vec![BlockInfo {
                    offset: 0,
                    length: 4096,
                    error: "bad".to_string(),
                    retry_count: 3,
                }],
                good_bytes: 9 * 4096,
                bad_bytes: 4096,
                file_size: 10 * 4096,
                block_size: 4096,
            },
        ];

        let report = generate_report(Path::new("/source"), &maps, 100);
        assert_eq!(report.total_files_scanned, 100);
        assert_eq!(report.files_with_bad_sectors, 1);
        assert_eq!(report.total_bad_blocks, 1);
        assert_eq!(report.total_bad_bytes, 4096);
        assert_eq!(report.files.len(), 1); // Only bad file included
    }
}
