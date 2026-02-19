//! DrillEngine - The main engine orchestrating all operations
//!
//! Provides high-level API for indexing, searching, and exporting.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use parking_lot::RwLock;
use rayon::prelude::*;
use tokio::sync::mpsc;

use super::index::{FileEntry, FileIndex, IndexStats};
use super::scanner::{ScanOptions, Scanner};
use super::{FileType, Progress};
use crate::cli::IndexArgs;
use crate::export::{ExportOptions, ExportResult, Exporter};
use crate::preview::ThumbnailGenerator;

/// The main Diamond Drill engine
pub struct DrillEngine {
    /// Source path being indexed
    source: PathBuf,
    /// File index
    index: Arc<RwLock<FileIndex>>,
    /// Thumbnail generator
    thumbnail_gen: Arc<ThumbnailGenerator>,
    /// Bad sector log
    bad_sectors: Arc<RwLock<Vec<super::BadSector>>>,
    /// Index statistics
    stats: Arc<RwLock<IndexStats>>,
}

impl DrillEngine {
    /// Create a new engine for the given source path
    pub async fn new(source: PathBuf) -> Result<Self> {
        let source = source
            .canonicalize()
            .with_context(|| format!("Failed to resolve path: {}", source.display()))?;

        Ok(Self {
            source: source.clone(),
            index: Arc::new(RwLock::new(FileIndex::new(source))),
            thumbnail_gen: Arc::new(ThumbnailGenerator::new()),
            bad_sectors: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(IndexStats::default())),
        })
    }

    /// Load existing index or create new engine
    pub async fn load_or_create(source: &Path) -> Result<Self> {
        // Try to load existing index
        let index_path = Self::get_index_path(source);
        if index_path.exists() {
            if let Ok(index) = FileIndex::load(&index_path).await {
                // Reconstruct stats from loaded index
                let stats = index.stats();

                // Extract bad sectors from the loaded index
                let bad_sectors = index.bad_sectors().to_vec();

                return Ok(Self {
                    source: source.to_path_buf(),
                    index: Arc::new(RwLock::new(index)),
                    thumbnail_gen: Arc::new(ThumbnailGenerator::new()),
                    bad_sectors: Arc::new(RwLock::new(bad_sectors)),
                    stats: Arc::new(RwLock::new(stats)),
                });
            }
        }

        // Create new engine
        Self::new(source.to_path_buf()).await
    }

    /// Get the default index path for a source
    fn get_index_path(source: &Path) -> PathBuf {
        let hash = blake3::hash(source.to_string_lossy().as_bytes());
        let hex = hex::encode(&hash.as_bytes()[..8]);

        directories::ProjectDirs::from("com", "tunclon", "diamond-drill")
            .map(|dirs| dirs.data_dir().join(format!("{}.idx", hex)))
            .unwrap_or_else(|| PathBuf::from(format!(".diamond-drill-{}.idx", hex)))
    }

    /// Index with progress reporting
    pub async fn index_with_progress(&self, args: &IndexArgs) -> Result<()> {
        let options = ScanOptions {
            source: args.source.clone(),
            skip_hidden: args.skip_hidden,
            max_depth: args.depth,
            extensions: args.extensions.clone(),
            workers: args.workers.unwrap_or_else(num_cpus::get),
            same_file_system: false,
        };

        let scanner = Scanner::new(options);
        let (tx, mut rx) = mpsc::channel::<FileEntry>(1000);

        // Spawn scanner in background
        let scan_handle = {
            let bad_sectors = Arc::clone(&self.bad_sectors);
            tokio::spawn(async move { scanner.scan_parallel(tx, bad_sectors).await })
        };

        // Collect results
        let mut entries = Vec::new();
        while let Some(entry) = rx.recv().await {
            entries.push(entry);
        }

        // Wait for scanner to complete
        let scan_stats = scan_handle
            .await
            .context("Scanner task panicked")?
            .context("Scanner failed")?;

        // Update index
        {
            let mut index = self.index.write();
            for entry in entries {
                index.add_entry(entry);
            }
        }

        // Replace bad sectors in the index for persistence (not extend, to avoid duplicates)
        {
            let mut index = self.index.write();
            let bad_sectors = self.bad_sectors.read().clone();
            index.set_bad_sectors(bad_sectors);
        }

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_files = self.index.read().len();
            stats.total_bytes = self.index.read().total_bytes();
            stats.indexed_at = Some(Utc::now());
            stats.scan_duration_ms = scan_stats.duration_ms;
            stats.bad_sector_count = self.bad_sectors.read().len();
        }

        // Generate thumbnails if requested
        if args.thumbnails {
            self.generate_thumbnails_parallel().await?;
        }

        // Save index (now includes bad_sectors)
        // Clone index data before await to avoid holding lock across await point
        if let Some(ref index_path) = args.index_file {
            let index_data = bincode::serialize(&*self.index.read())
                .context("Failed to serialize index")?;
            let path = index_path.clone();
            tokio::task::spawn_blocking(move || std::fs::write(&path, index_data))
                .await
                .context("Index save task panicked")?
                .with_context(|| format!("Failed to write index to {}", index_path.display()))?;
        } else {
            let default_path = Self::get_index_path(&args.source);
            if let Some(parent) = default_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("Failed to create index directory: {}", parent.display()))?;
            }
            let index_data = bincode::serialize(&*self.index.read())
                .context("Failed to serialize index")?;
            let path = default_path.clone();
            tokio::task::spawn_blocking(move || std::fs::write(&path, index_data))
                .await
                .context("Index save task panicked")?
                .with_context(|| format!("Failed to write index to {}", default_path.display()))?;
        }

        Ok(())
    }

    /// Get total file count
    pub async fn file_count(&self) -> usize {
        self.index.read().len()
    }

    /// Get all files as path strings
    pub async fn get_all_files(&self) -> Result<Vec<String>> {
        Ok(self
            .index
            .read()
            .entries()
            .map(|e| e.path.to_string_lossy().to_string())
            .collect())
    }

    /// Get files by type
    pub async fn get_files_by_type(&self, type_name: &str) -> Result<Vec<String>> {
        let file_type = match type_name.to_lowercase().as_str() {
            "image" | "images" | "photo" | "photos" => FileType::Image,
            "video" | "videos" => FileType::Video,
            "audio" | "music" | "sound" => FileType::Audio,
            "document" | "documents" | "doc" | "docs" => FileType::Document,
            "archive" | "archives" | "compressed" => FileType::Archive,
            "code" | "source" => FileType::Code,
            _ => return Ok(Vec::new()),
        };

        Ok(self
            .index
            .read()
            .entries()
            .filter(|e| e.file_type == file_type)
            .map(|e| e.path.to_string_lossy().to_string())
            .collect())
    }

    /// Fuzzy search files
    pub async fn search_fuzzy(&self, pattern: &str) -> Result<Vec<String>> {
        use fuzzy_matcher::skim::SkimMatcherV2;
        use fuzzy_matcher::FuzzyMatcher;

        let matcher = SkimMatcherV2::default();
        let pattern_lower = pattern.to_lowercase();

        let mut matches: Vec<(i64, String)> = self
            .index
            .read()
            .entries()
            .filter_map(|e| {
                let path_str = e.path.to_string_lossy().to_string();
                let name = e
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();

                matcher
                    .fuzzy_match(&name, &pattern_lower)
                    .map(|score| (score, path_str))
            })
            .collect();

        // Sort by score descending
        matches.sort_by(|a, b| b.0.cmp(&a.0));

        Ok(matches.into_iter().map(|(_, path)| path).collect())
    }

    /// Search with interactive filtering
    pub async fn search_interactive(&self, args: &crate::cli::SearchArgs) -> Result<()> {
        let results = match args.search_type {
            crate::cli::SearchType::Fuzzy => self.search_fuzzy(&args.pattern).await?,
            crate::cli::SearchType::Glob => self.search_glob(&args.pattern).await?,
            crate::cli::SearchType::Regex => self.search_regex(&args.pattern).await?,
            crate::cli::SearchType::Exact => self.search_exact(&args.pattern).await?,
        };

        // Apply filters
        let filtered: Vec<_> = results.into_iter().take(args.limit).collect();

        for path in &filtered {
            println!("{}", path);
        }

        println!("\nFound {} matches", filtered.len());
        Ok(())
    }

    /// Glob pattern search
    pub async fn search_glob(&self, pattern: &str) -> Result<Vec<String>> {
        use globset::Glob;

        let glob = Glob::new(pattern)
            .with_context(|| format!("Invalid glob pattern: {}", pattern))?
            .compile_matcher();

        Ok(self
            .index
            .read()
            .entries()
            .filter(|e| {
                let name = e
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                glob.is_match(&name)
            })
            .map(|e| e.path.to_string_lossy().to_string())
            .collect())
    }

    /// Regex search
    pub async fn search_regex(&self, pattern: &str) -> Result<Vec<String>> {
        let regex = regex::Regex::new(pattern)
            .with_context(|| format!("Invalid regex pattern: {}", pattern))?;

        Ok(self
            .index
            .read()
            .entries()
            .filter(|e| {
                let path_str = e.path.to_string_lossy();
                regex.is_match(&path_str)
            })
            .map(|e| e.path.to_string_lossy().to_string())
            .collect())
    }

    /// Exact match search
    pub async fn search_exact(&self, pattern: &str) -> Result<Vec<String>> {
        let pattern_lower = pattern.to_lowercase();

        Ok(self
            .index
            .read()
            .entries()
            .filter(|e| {
                let name = e
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                name.contains(&pattern_lower)
            })
            .map(|e| e.path.to_string_lossy().to_string())
            .collect())
    }

    /// Preview files
    pub async fn preview_files(&self, args: &crate::cli::PreviewArgs) -> Result<()> {
        for file in &args.files {
            if let Some(entry) = self.index.read().get_by_path(file) {
                println!(
                    "{} {} ({}) - {}",
                    entry.file_type.icon(),
                    entry.path.display(),
                    humansize::format_size(entry.size, humansize::BINARY),
                    entry
                        .modified
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "Unknown".to_string())
                );
            }
        }
        Ok(())
    }

    /// Get file info
    pub async fn get_file_info(&self, path: &str) -> Result<FileEntry> {
        self.index
            .read()
            .get_by_path(path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("File not found in index: {}", path))
    }

    /// Summarize files by type
    pub async fn summarize_files(&self, files: &[String]) -> Result<Vec<(String, usize)>> {
        let mut counts: HashMap<FileType, usize> = HashMap::new();

        let index = self.index.read();
        for path in files {
            if let Some(entry) = index.get_by_path(path) {
                *counts.entry(entry.file_type).or_insert(0) += 1;
            }
        }

        Ok(counts
            .into_iter()
            .map(|(ft, count)| (format!("{} {:?}", ft.icon(), ft), count))
            .collect())
    }

    /// Export selected files
    pub async fn export_selected(&self, args: &crate::cli::ExportArgs) -> Result<()> {
        let options = ExportOptions {
            dest: args.dest.clone(),
            preserve_structure: args.preserve_structure,
            verify_hash: !args.no_verify,
            continue_on_error: args.continue_on_error,
            create_manifest: args.manifest,
            dry_run: args.dry_run,
        };

        let files: Vec<String> = if args.files.is_empty() {
            // Export all
            self.get_all_files().await?
        } else {
            args.files.clone()
        };

        let result = self
            .export_files_with_progress(&files, &options, |_| {})
            .await?;

        println!("\nExport complete:");
        println!("  Successful: {}", result.successful);
        println!("  Failed: {}", result.failed);
        println!(
            "  Total size: {}",
            humansize::format_size(result.total_bytes, humansize::BINARY)
        );

        Ok(())
    }

    /// Export files with progress callback
    pub async fn export_files_with_progress<F>(
        &self,
        files: &[String],
        options: &ExportOptions,
        progress_callback: F,
    ) -> Result<ExportResult>
    where
        F: Fn(Progress) + Send + Sync,
    {
        let exporter = Exporter::new(options.clone());

        let entries: Vec<_> = {
            let index = self.index.read();
            files
                .iter()
                .filter_map(|path| index.get_by_path(path).cloned())
                .collect()
        };

        exporter.export_batch(&entries, progress_callback).await
    }

    /// Generate thumbnails in parallel
    async fn generate_thumbnails_parallel(&self) -> Result<()> {
        let images: Vec<_> = self
            .index
            .read()
            .entries()
            .filter(|e| e.file_type == FileType::Image)
            .cloned()
            .collect();

        let thumb_gen = Arc::clone(&self.thumbnail_gen);

        // Process in parallel using rayon
        images.par_iter().for_each(|entry| {
            if let Err(e) = thumb_gen.generate_progressive(&entry.path, 64, 512) {
                tracing::warn!(
                    "Failed to generate thumbnail for {}: {}",
                    entry.path.display(),
                    e
                );
            }
        });

        Ok(())
    }

    /// Run deduplication analysis and optionally purge duplicates.
    pub async fn run_dedup(&self, args: &crate::cli::DedupArgs) -> Result<()> {
        use crate::dedup;

        println!("Diamond Drill Dedup Engine");
        println!("Scanning {}...\n", self.source.display());

        // If we have no index, do a quick scan first
        if self.index.read().is_empty() {
            let index_args = crate::cli::IndexArgs {
                source: self.source.clone(),
                resume: false,
                index_file: None,
                skip_hidden: true,
                depth: None,
                extensions: None,
                thumbnails: false,
                workers: None,
                checkpoint_interval: 1000,
                bad_sector_report: None,
                block_size: 4096,
            };
            self.index_with_progress(&index_args).await?;
        }

        let entries: Vec<FileEntry> = self.index.read().entries().cloned().collect();

        println!(
            "Indexed {} files. Running dedup analysis...\n",
            entries.len()
        );

        // Map CLI strategy to dedup strategy
        let strategy = match args.keep {
            crate::cli::DedupKeepStrategy::Newest => dedup::KeepStrategy::Newest,
            crate::cli::DedupKeepStrategy::Largest => dedup::KeepStrategy::Largest,
            crate::cli::DedupKeepStrategy::Oldest => dedup::KeepStrategy::Oldest,
            crate::cli::DedupKeepStrategy::Cleanest => dedup::KeepStrategy::Cleanest,
        };

        let options = dedup::DedupOptions {
            strategy,
            fuzzy: args.fuzzy,
            fuzzy_threshold: args.threshold,
            min_size: args.min_size,
        };

        let report = dedup::analyze(&entries, &options)?;

        // Output report
        match args.report {
            crate::cli::DedupReportFormat::Human => {
                print!("{}", report.to_human_string());
            }
            crate::cli::DedupReportFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }

        // Purge if requested
        if args.purge && !report.groups.is_empty() {
            println!("Purging {} duplicate files...\n", report.total_duplicates);
            let (deleted, freed, errors) = dedup::purge_duplicates(&report.groups, false);
            println!(
                "Purged {} files, freed {}",
                deleted,
                humansize::format_size(freed, humansize::BINARY)
            );
            if !errors.is_empty() {
                eprintln!("\nErrors:");
                for err in &errors {
                    eprintln!("  {}", err);
                }
            }
        } else if !report.groups.is_empty() && !args.purge {
            println!("Run with --purge to delete duplicate files.");
        }

        Ok(())
    }
}
