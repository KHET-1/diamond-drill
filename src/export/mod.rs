//! Export module - Safe file export with verification
//!
//! Provides async copy with blake3 hash verification and manifest generation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};

use crate::core::{FileEntry, Progress};

/// Export configuration options
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Destination directory
    pub dest: PathBuf,
    /// Preserve original directory structure
    pub preserve_structure: bool,
    /// Verify file integrity with blake3 hash
    pub verify_hash: bool,
    /// Continue exporting on errors
    pub continue_on_error: bool,
    /// Create manifest file with hashes
    pub create_manifest: bool,
    /// Dry run mode
    pub dry_run: bool,
}

/// Result of an export operation
#[derive(Debug, Clone, Default)]
pub struct ExportResult {
    /// Number of successfully exported files
    pub successful: usize,
    /// Number of failed exports
    pub failed: usize,
    /// Total bytes exported
    pub total_bytes: u64,
    /// Path to manifest file if created
    pub manifest_path: Option<PathBuf>,
    /// Errors encountered
    pub errors: Vec<ExportError>,
}

/// Export error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportError {
    pub source_path: PathBuf,
    pub dest_path: PathBuf,
    pub error: String,
    pub recoverable: bool,
}

/// Manifest entry for exported file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub source_path: String,
    pub dest_path: String,
    pub size: u64,
    pub blake3_hash: String,
    pub exported_at: String,
    pub verified: bool,
}

/// Manifest file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    pub version: u32,
    pub created_at: String,
    pub source_root: String,
    pub dest_root: String,
    pub total_files: usize,
    pub total_bytes: u64,
    pub entries: Vec<ManifestEntry>,
}

impl ExportManifest {
    pub fn new(source_root: &Path, dest_root: &Path) -> Self {
        Self {
            version: 1,
            created_at: Utc::now().to_rfc3339(),
            source_root: source_root.to_string_lossy().to_string(),
            dest_root: dest_root.to_string_lossy().to_string(),
            total_files: 0,
            total_bytes: 0,
            entries: Vec::new(),
        }
    }
}

/// File exporter with async operations
pub struct Exporter {
    options: ExportOptions,
}

impl Exporter {
    /// Create a new exporter with options
    pub fn new(options: ExportOptions) -> Self {
        Self { options }
    }

    /// Export a batch of files with progress callback
    pub async fn export_batch<F>(
        &self,
        entries: &[FileEntry],
        progress_callback: F,
    ) -> Result<ExportResult>
    where
        F: Fn(Progress) + Send + Sync,
    {
        let mut result = ExportResult::default();
        let mut manifest = ExportManifest::new(
            &entries
                .first()
                .map(|e| e.path.parent().unwrap_or(&e.path).to_path_buf())
                .unwrap_or_default(),
            &self.options.dest,
        );

        // Ensure destination exists
        if !self.options.dry_run {
            fs::create_dir_all(&self.options.dest)
                .await
                .with_context(|| format!("Failed to create destination: {}", self.options.dest.display()))?;
        }

        let total = entries.len();
        let completed = Arc::new(AtomicUsize::new(0));
        let total_bytes = Arc::new(AtomicU64::new(0));
        let errors = Arc::new(AtomicUsize::new(0));

        // Process files concurrently with bounded concurrency
        let semaphore = Arc::new(tokio::sync::Semaphore::new(8));

        let mut handles = Vec::new();

        for entry in entries {
            let permit = semaphore.clone().acquire_owned().await?;
            let entry_clone = entry.clone();
            let options = self.options.clone();
            let completed_clone = Arc::clone(&completed);
            let total_bytes_clone = Arc::clone(&total_bytes);
            let errors_clone = Arc::clone(&errors);

            let handle = tokio::spawn(async move {
                let result = export_single_file(&entry_clone, &options).await;
                drop(permit);

                completed_clone.fetch_add(1, Ordering::Relaxed);

                match result {
                    Ok((bytes, hash)) => {
                        total_bytes_clone.fetch_add(bytes, Ordering::Relaxed);
                        Ok(ManifestEntry {
                            source_path: entry_clone.path.to_string_lossy().to_string(),
                            dest_path: get_dest_path(&entry_clone.path, &options)
                                .to_string_lossy()
                                .to_string(),
                            size: bytes,
                            blake3_hash: hash,
                            exported_at: Utc::now().to_rfc3339(),
                            verified: options.verify_hash,
                        })
                    }
                    Err(e) => {
                        errors_clone.fetch_add(1, Ordering::Relaxed);
                        Err(e)
                    }
                }
            });

            handles.push(handle);

            // Update progress
            let current_completed = completed.load(Ordering::Relaxed);
            progress_callback(Progress {
                total,
                completed: current_completed,
                current_file: entry.path.to_string_lossy().to_string(),
                bytes_processed: total_bytes.load(Ordering::Relaxed),
                errors: errors.load(Ordering::Relaxed),
                bad_sectors: 0,
            });
        }

        // Wait for all tasks
        for handle in handles {
            match handle.await {
                Ok(Ok(manifest_entry)) => {
                    result.successful += 1;
                    manifest.entries.push(manifest_entry);
                }
                Ok(Err(e)) => {
                    result.failed += 1;
                    if !self.options.continue_on_error {
                        return Err(e);
                    }
                    result.errors.push(ExportError {
                        source_path: PathBuf::new(),
                        dest_path: PathBuf::new(),
                        error: e.to_string(),
                        recoverable: true,
                    });
                }
                Err(e) => {
                    result.failed += 1;
                    result.errors.push(ExportError {
                        source_path: PathBuf::new(),
                        dest_path: PathBuf::new(),
                        error: format!("Task failed: {}", e),
                        recoverable: false,
                    });
                }
            }
        }

        result.total_bytes = total_bytes.load(Ordering::Relaxed);

        // Create manifest
        if self.options.create_manifest && !self.options.dry_run {
            manifest.total_files = result.successful;
            manifest.total_bytes = result.total_bytes;

            let manifest_path = self.options.dest.join("diamond-drill-manifest.json");
            let manifest_json = serde_json::to_string_pretty(&manifest)?;
            fs::write(&manifest_path, manifest_json).await?;
            result.manifest_path = Some(manifest_path);
        }

        Ok(result)
    }
}

/// Export a single file
async fn export_single_file(entry: &FileEntry, options: &ExportOptions) -> Result<(u64, String)> {
    let dest_path = get_dest_path(&entry.path, options);

    if options.dry_run {
        tracing::info!(
            "Would export: {} -> {}",
            entry.path.display(),
            dest_path.display()
        );
        return Ok((entry.size, String::new()));
    }

    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    // Copy file with hash computation
    let (bytes, hash) = copy_with_hash(&entry.path, &dest_path)
        .await
        .with_context(|| {
            format!(
                "Failed to copy {} to {}",
                entry.path.display(),
                dest_path.display()
            )
        })?;

    // Verify hash if requested
    if options.verify_hash {
        let dest_hash = compute_file_hash(&dest_path).await?;
        if hash != dest_hash {
            fs::remove_file(&dest_path).await.ok();
            anyhow::bail!(
                "Hash mismatch for {}: source={}, dest={}",
                entry.path.display(),
                hash,
                dest_hash
            );
        }
    }

    Ok((bytes, hash))
}

/// Get destination path for a file
fn get_dest_path(source: &Path, options: &ExportOptions) -> PathBuf {
    if options.preserve_structure {
        // Try to preserve directory structure
        if let Some(file_name) = source.file_name() {
            // Get relative path components
            let components: Vec<_> = source
                .components()
                .skip(1) // Skip root
                .collect();

            if components.len() > 1 {
                let mut dest = options.dest.clone();
                for comp in components {
                    dest.push(comp);
                }
                return dest;
            }

            options.dest.join(file_name)
        } else {
            options.dest.join(source.file_name().unwrap_or_default())
        }
    } else {
        options.dest.join(source.file_name().unwrap_or_default())
    }
}

/// Copy file and compute blake3 hash simultaneously
async fn copy_with_hash(source: &Path, dest: &Path) -> Result<(u64, String)> {
    let source_file = fs::File::open(source).await?;
    let dest_file = fs::File::create(dest).await?;

    let mut reader = BufReader::new(source_file);
    let mut writer = BufWriter::new(dest_file);
    let mut hasher = blake3::Hasher::new();

    let mut total_bytes = 0u64;
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer

    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
        writer.write_all(&buffer[..bytes_read]).await?;
        total_bytes += bytes_read as u64;
    }

    writer.flush().await?;

    let hash = hasher.finalize();
    let hash_hex = hex::encode(hash.as_bytes());

    Ok((total_bytes, hash_hex))
}

/// Compute blake3 hash of a file
async fn compute_file_hash(path: &Path) -> Result<String> {
    let file = fs::File::open(path).await?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();

    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let bytes_read = reader.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize();
    Ok(hex::encode(hash.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_copy_with_hash() {
        let source_dir = tempdir().unwrap();
        let dest_dir = tempdir().unwrap();

        let source_path = source_dir.path().join("test.txt");
        let dest_path = dest_dir.path().join("test.txt");

        // Create source file
        fs::write(&source_path, "Hello, Diamond Drill!")
            .await
            .unwrap();

        // Copy with hash
        let (bytes, hash) = copy_with_hash(&source_path, &dest_path).await.unwrap();

        assert_eq!(bytes, 21);
        assert!(!hash.is_empty());

        // Verify content
        let content = fs::read_to_string(&dest_path).await.unwrap();
        assert_eq!(content, "Hello, Diamond Drill!");

        // Verify hash matches
        let verify_hash = compute_file_hash(&dest_path).await.unwrap();
        assert_eq!(hash, verify_hash);
    }

    #[tokio::test]
    async fn test_exporter_basic() {
        let source_dir = tempdir().unwrap();
        let dest_dir = tempdir().unwrap();

        // Create test file
        let source_path = source_dir.path().join("test.txt");
        fs::write(&source_path, "test content").await.unwrap();

        let entry = FileEntry {
            path: source_path,
            size: 12,
            file_type: crate::core::FileType::Document,
            extension: "txt".to_string(),
            modified: None,
            created: None,
            hash: None,
            has_bad_sectors: false,
            thumbnail: None,
        };

        let options = ExportOptions {
            dest: dest_dir.path().to_path_buf(),
            preserve_structure: false,
            verify_hash: true,
            continue_on_error: false,
            create_manifest: true,
            dry_run: false,
        };

        let exporter = Exporter::new(options);
        let result = exporter.export_batch(&[entry], |_| {}).await.unwrap();

        assert_eq!(result.successful, 1);
        assert_eq!(result.failed, 0);
        assert!(result.manifest_path.is_some());
    }
}
