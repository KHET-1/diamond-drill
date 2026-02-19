//! Scanner - Parallel file system scanner with bad sector handling
//!
//! Uses rayon for parallel traversal and handles I/O errors gracefully.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use crossbeam_channel;
use parking_lot::RwLock;
use rayon::prelude::*;
use tokio::sync::mpsc;
use walkdir::{DirEntry, WalkDir};

use super::index::FileEntry;
use super::BadSector;

/// Scanner configuration options
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Source path to scan
    pub source: PathBuf,
    /// Skip hidden files and directories
    pub skip_hidden: bool,
    /// Maximum traversal depth
    pub max_depth: Option<usize>,
    /// File extensions to include (None = all)
    pub extensions: Option<Vec<String>>,
    /// Number of parallel workers
    pub workers: usize,
    /// Stay on the same filesystem (avoid crossing mount points)
    pub same_file_system: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            source: PathBuf::from("."),
            skip_hidden: true,
            max_depth: None,
            extensions: None,
            workers: num_cpus::get(),
            same_file_system: false,
        }
    }
}

/// Statistics from a scan operation
#[derive(Debug, Default)]
pub struct ScanStats {
    pub files_found: usize,
    pub directories_found: usize,
    pub bytes_total: u64,
    pub errors: usize,
    pub bad_sectors: usize,
    pub duration_ms: u64,
}

/// Parallel file system scanner
pub struct Scanner {
    options: ScanOptions,
}

impl Scanner {
    /// Create a new scanner with options
    pub fn new(options: ScanOptions) -> Self {
        // Configure rayon thread pool
        rayon::ThreadPoolBuilder::new()
            .num_threads(options.workers)
            .build_global()
            .ok();

        Self { options }
    }

    /// Scan the source path and send entries through channel
    pub async fn scan_parallel(
        &self,
        tx: mpsc::Sender<FileEntry>,
        bad_sectors: Arc<RwLock<Vec<BadSector>>>,
    ) -> Result<ScanStats> {
        let start = Instant::now();
        let options = self.options.clone();

        // Counters
        let files_found = Arc::new(AtomicUsize::new(0));
        let dirs_found = Arc::new(AtomicUsize::new(0));
        let bytes_total = Arc::new(AtomicU64::new(0));
        let errors = Arc::new(AtomicUsize::new(0));
        let bad_sector_count = Arc::new(AtomicUsize::new(0));

        // Collect directory entries first (single-threaded walk)
        let entries: Vec<DirEntry> = {
            let mut walker = WalkDir::new(&options.source)
                .follow_links(false)
                .same_file_system(options.same_file_system);

            if let Some(depth) = options.max_depth {
                walker = walker.max_depth(depth);
            }

            let source_path = options.source.clone();
            walker
                .into_iter()
                .filter_entry(move |e| {
                    if options.skip_hidden {
                        e.path() == source_path || !is_hidden(e)
                    } else {
                        true
                    }
                })
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter(|e| {
                    if let Some(ref exts) = options.extensions {
                        e.path()
                            .extension()
                            .map(|ext| {
                                let ext_str = ext.to_string_lossy().to_lowercase();
                                exts.iter().any(|allowed| {
                                    allowed.to_lowercase().trim_start_matches('.') == ext_str
                                })
                            })
                            .unwrap_or(false)
                    } else {
                        true
                    }
                })
                .collect()
        };

        dirs_found.fetch_add(
            WalkDir::new(&options.source)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_dir())
                .count(),
            Ordering::Relaxed,
        );

        // Process entries in parallel with rayon
        let (sender, receiver) = crossbeam_channel::bounded::<FileEntry>(1000);

        // Spawn a task to forward from crossbeam to tokio mpsc
        let tx_clone = tx.clone();
        let forward_handle = tokio::spawn(async move {
            while let Ok(entry) = receiver.recv() {
                if tx_clone.send(entry).await.is_err() {
                    break;
                }
            }
        });

        // Process in parallel
        {
            let files_found = Arc::clone(&files_found);
            let bytes_total = Arc::clone(&bytes_total);
            let errors = Arc::clone(&errors);
            let bad_sector_count = Arc::clone(&bad_sector_count);
            let bad_sectors = Arc::clone(&bad_sectors);
            let sender = sender.clone();

            entries.par_iter().for_each(|entry| {
                match process_entry(entry, &bad_sectors, &bad_sector_count) {
                    Ok(file_entry) => {
                        files_found.fetch_add(1, Ordering::Relaxed);
                        bytes_total.fetch_add(file_entry.size, Ordering::Relaxed);
                        let _ = sender.send(file_entry);
                    }
                    Err(e) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!("Error processing {}: {}", entry.path().display(), e);
                    }
                }
            });
        }

        // Signal completion
        drop(sender);
        forward_handle.await?;

        let duration = start.elapsed();

        Ok(ScanStats {
            files_found: files_found.load(Ordering::Relaxed),
            directories_found: dirs_found.load(Ordering::Relaxed),
            bytes_total: bytes_total.load(Ordering::Relaxed),
            errors: errors.load(Ordering::Relaxed),
            bad_sectors: bad_sector_count.load(Ordering::Relaxed),
            duration_ms: duration.as_millis() as u64,
        })
    }
}

/// Process a single directory entry into a FileEntry
fn process_entry(
    entry: &DirEntry,
    bad_sectors: &Arc<RwLock<Vec<BadSector>>>,
    bad_sector_count: &Arc<AtomicUsize>,
) -> Result<FileEntry> {
    let path = entry.path().to_path_buf();

    // Try to read metadata - this may fail for bad sectors
    let metadata = match entry.metadata() {
        Ok(m) => m,
        Err(e) => {
            // Log bad sector
            let bad = BadSector {
                file_path: path.clone(),
                offset: 0,
                length: 0,
                error: e.to_string(),
                detected_at: Utc::now(),
                retry_count: 0,
                block_size: 4096,
            };
            bad_sectors.write().push(bad);
            bad_sector_count.fetch_add(1, Ordering::Relaxed);

            // Still try to get basic info
            std::fs::metadata(&path)?
        }
    };

    // Create file entry
    let mut file_entry = FileEntry::new(path.clone(), &metadata);

    // Check for read errors (potential bad sectors) by trying to read first bytes
    if let Err(e) = check_file_readable(&path) {
        file_entry.has_bad_sectors = true;

        let bad = BadSector {
            file_path: path,
            offset: 0,
            length: metadata.len(),
            error: e.to_string(),
            detected_at: Utc::now(),
            retry_count: 0,
            block_size: 4096,
        };
        bad_sectors.write().push(bad);
        bad_sector_count.fetch_add(1, Ordering::Relaxed);
    }

    Ok(file_entry)
}

/// Check if a file is readable (detect bad sectors)
fn check_file_readable(path: &std::path::Path) -> Result<()> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0u8; 4096]; // Read first 4KB
                                  // Use read_exact wrapped in a match to handle files smaller than 4KB
    match file.read_exact(&mut buffer) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(()), // File shorter than buffer is fine
        Err(e) => Err(e.into()),
    }
}

/// Check if entry is hidden (starts with .)
fn is_hidden(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_scanner_basic() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();

        // Create test files with enough content for check_file_readable
        std::fs::write(dir_path.join("test.txt"), "hello world test content").unwrap();
        std::fs::write(dir_path.join("test.jpg"), "fake image test content").unwrap();
        std::fs::create_dir_all(dir_path.join("subdir")).unwrap();
        std::fs::write(
            dir_path.join("subdir").join("nested.rs"),
            "fn main() { println!(); }",
        )
        .unwrap();

        let options = ScanOptions {
            source: dir_path,
            skip_hidden: false,
            max_depth: None,
            extensions: None,
            workers: 1,
            same_file_system: false,
        };

        let scanner = Scanner::new(options);
        let (tx, mut rx) = mpsc::channel(1000);
        let bad_sectors = Arc::new(RwLock::new(Vec::new()));

        let stats = scanner.scan_parallel(tx, bad_sectors).await.unwrap();

        assert_eq!(
            stats.files_found, 3,
            "Expected 3 files, found {}",
            stats.files_found
        );
        assert_eq!(stats.errors, 0);

        // Collect results
        let mut entries = Vec::new();
        while let Ok(entry) = rx.try_recv() {
            entries.push(entry);
        }
        assert_eq!(
            entries.len(),
            3,
            "Expected 3 entries, got {}",
            entries.len()
        );
    }

    #[tokio::test]
    async fn test_scanner_with_extension_filter() {
        let dir = tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();

        std::fs::write(dir_path.join("test.txt"), "hello world test content").unwrap();
        std::fs::write(dir_path.join("test.jpg"), "fake image test content").unwrap();
        std::fs::write(dir_path.join("test.rs"), "fn main() { println!(); }").unwrap();

        let options = ScanOptions {
            source: dir_path,
            skip_hidden: false,
            max_depth: None,
            extensions: Some(vec!["jpg".to_string(), "rs".to_string()]),
            workers: 1,
            same_file_system: false,
        };

        let scanner = Scanner::new(options);
        let (tx, _rx) = mpsc::channel(1000);
        let bad_sectors = Arc::new(RwLock::new(Vec::new()));

        let stats = scanner.scan_parallel(tx, bad_sectors).await.unwrap();

        assert_eq!(
            stats.files_found, 2,
            "Expected 2 files (jpg+rs), found {}",
            stats.files_found
        );
    }
}
