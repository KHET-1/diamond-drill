//! Dedup module - Content-addressable and fuzzy file deduplication
//!
//! Provides exact (Blake3) and near-duplicate detection with
//! intelligent master selection and purge/merge workflows.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::FileEntry;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Strategy for automatically selecting the "master" (keeper) in each group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum KeepStrategy {
    /// Keep the most recently modified file.
    #[default]
    Newest,
    /// Keep the largest file (most content).
    Largest,
    /// Keep the oldest file (original).
    Oldest,
    /// Keep the file whose name looks cleanest (no temp/backup suffixes).
    Cleanest,
}

/// A group of duplicate files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DupGroup {
    /// Blake3 content hash shared by exact duplicates (None for fuzzy groups).
    pub hash: Option<String>,
    /// Similarity score 0–100 (100 = exact duplicate).
    pub similarity: u8,
    /// The file chosen as the master/keeper.
    pub master: PathBuf,
    /// All other files in the group (candidates for deletion).
    pub duplicates: Vec<PathBuf>,
    /// Total bytes that would be freed by purging duplicates.
    pub wasted_bytes: u64,
}

/// Full dedup analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupReport {
    pub scanned_files: usize,
    pub unique_files: usize,
    pub duplicate_groups: usize,
    pub total_duplicates: usize,
    pub wasted_bytes: u64,
    pub groups: Vec<DupGroup>,
    pub generated_at: DateTime<Utc>,
    pub strategy: String,
    pub fuzzy_threshold: u8,
}

/// Options controlling the dedup analysis.
#[derive(Debug, Clone)]
pub struct DedupOptions {
    /// Keep strategy for master selection.
    pub strategy: KeepStrategy,
    /// Enable fuzzy (near-duplicate) detection.
    pub fuzzy: bool,
    /// Fuzzy similarity threshold 0–100 (default 85).
    pub fuzzy_threshold: u8,
    /// Minimum file size to consider (skip tiny files).
    pub min_size: u64,
}

impl Default for DedupOptions {
    fn default() -> Self {
        Self {
            strategy: KeepStrategy::Newest,
            fuzzy: false,
            fuzzy_threshold: 85,
            min_size: 1, // skip 0-byte files
        }
    }
}

// ---------------------------------------------------------------------------
// Temp/backup suffix detection
// ---------------------------------------------------------------------------

/// Returns true if the filename looks like a temp/backup copy.
fn is_temp_name(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let lower = name.to_lowercase();

    lower.ends_with("~")
        || lower.ends_with(".bak")
        || lower.ends_with(".tmp")
        || lower.ends_with(".swp")
        || lower.ends_with(".orig")
        || lower.contains("_old")
        || lower.contains("_backup")
        || lower.contains("_copy")
        || lower.contains(" - copy")
        || lower.contains("(1)")
        || lower.contains("(2)")
        || lower.contains("(3)")
        || lower.starts_with("~$") // Office temp files
}

// ---------------------------------------------------------------------------
// Blake3 exact hashing
// ---------------------------------------------------------------------------

/// Compute Blake3 hash of a file (streaming, 8 KB buffer).
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Compute a fast partial hash for large files:
/// first 4 MB + last 4 MB + file size → Blake3.
/// Falls back to full hash for files <= 8 MB.
pub fn hash_file_partial(path: &Path, size: u64) -> Result<String> {
    const CHUNK: u64 = 4 * 1024 * 1024; // 4 MB

    if size <= CHUNK * 2 {
        return hash_file(path);
    }

    use std::io::{Read as _, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();

    // Hash the size itself as disambiguation
    hasher.update(&size.to_le_bytes());

    // First 4 MB
    let mut buf = vec![0u8; CHUNK as usize];
    file.read_exact(&mut buf)?;
    hasher.update(&buf);

    // Last 4 MB
    file.seek(SeekFrom::End(-(CHUNK as i64)))?;
    file.read_exact(&mut buf)?;
    hasher.update(&buf);

    Ok(hasher.finalize().to_hex().to_string())
}

// ---------------------------------------------------------------------------
// Master selection
// ---------------------------------------------------------------------------

/// Pick the best file from a group of paths based on strategy.
fn select_master(
    paths: &[PathBuf],
    entries: &HashMap<String, &FileEntry>,
    strategy: KeepStrategy,
) -> PathBuf {
    if paths.len() == 1 {
        return paths[0].clone();
    }

    let mut scored: Vec<(i64, &PathBuf)> = paths
        .iter()
        .map(|p| {
            let key = p.to_string_lossy().to_string();
            let entry = entries.get(&key);

            let mut score: i64 = 0;

            match strategy {
                KeepStrategy::Newest => {
                    if let Some(e) = entry {
                        score += e.modified.map(|d| d.timestamp()).unwrap_or(0);
                    }
                }
                KeepStrategy::Largest => {
                    if let Some(e) = entry {
                        score += e.size as i64;
                    }
                }
                KeepStrategy::Oldest => {
                    if let Some(e) = entry {
                        // Negate so oldest sorts first
                        score -= e.modified.map(|d| d.timestamp()).unwrap_or(i64::MAX);
                    }
                }
                KeepStrategy::Cleanest => {
                    // Prefer non-temp names
                    if !is_temp_name(p) {
                        score += 1000;
                    }
                    // Tie-break by newest
                    if let Some(e) = entry {
                        score += e.modified.map(|d| d.timestamp() / 1_000_000).unwrap_or(0);
                    }
                }
            }

            // Universal bonus: prefer non-temp names as tiebreaker
            if strategy != KeepStrategy::Cleanest && !is_temp_name(p) {
                score += 1;
            }

            (score, p)
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored[0].1.clone()
}

// ---------------------------------------------------------------------------
// Exact dedup
// ---------------------------------------------------------------------------

/// Find exact duplicate groups by content hash.
/// Hashes are computed in parallel via rayon.
pub fn find_exact_duplicates(
    entries: &[FileEntry],
    options: &DedupOptions,
) -> Result<Vec<DupGroup>> {
    // Filter to eligible files
    let eligible: Vec<&FileEntry> = entries
        .iter()
        .filter(|e| e.size >= options.min_size)
        .collect();

    // First pass: group by size (files of different sizes can't be identical)
    let mut size_groups: HashMap<u64, Vec<&FileEntry>> = HashMap::new();
    for entry in &eligible {
        size_groups.entry(entry.size).or_default().push(entry);
    }

    // Only hash groups with 2+ files of same size
    let candidates: Vec<&FileEntry> = size_groups
        .values()
        .filter(|g| g.len() > 1)
        .flat_map(|g| g.iter().copied())
        .collect();

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Parallel hash computation
    let hashed: Vec<(PathBuf, u64, String)> = candidates
        .par_iter()
        .filter_map(|entry| {
            let hash = if entry.size > 8 * 1024 * 1024 {
                hash_file_partial(&entry.path, entry.size)
            } else {
                hash_file(&entry.path)
            };
            match hash {
                Ok(h) => Some((entry.path.clone(), entry.size, h)),
                Err(e) => {
                    tracing::warn!("Failed to hash {}: {}", entry.path.display(), e);
                    None
                }
            }
        })
        .collect();

    // Group by hash
    let mut hash_groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut size_map: HashMap<String, u64> = HashMap::new();
    for (path, size, hash) in hashed {
        hash_groups.entry(hash.clone()).or_default().push(path);
        size_map.insert(hash, size);
    }

    // Build lookup for master selection
    let entry_map: HashMap<String, &FileEntry> = eligible
        .iter()
        .map(|e| (e.path.to_string_lossy().to_string(), *e))
        .collect();

    // Build DupGroups for hashes with 2+ files
    let mut groups: Vec<DupGroup> = Vec::new();
    for (hash, paths) in hash_groups {
        if paths.len() < 2 {
            continue;
        }

        let master = select_master(&paths, &entry_map, options.strategy);
        let file_size = size_map.get(&hash).copied().unwrap_or(0);
        let duplicates: Vec<PathBuf> = paths.into_iter().filter(|p| p != &master).collect();
        let wasted = file_size * duplicates.len() as u64;

        groups.push(DupGroup {
            hash: Some(hash),
            similarity: 100,
            master,
            duplicates,
            wasted_bytes: wasted,
        });
    }

    // Sort by wasted bytes descending (biggest wins first)
    groups.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes));

    Ok(groups)
}

// ---------------------------------------------------------------------------
// Fuzzy dedup (name + size + mtime proximity)
// ---------------------------------------------------------------------------

/// Find near-duplicate groups by filename similarity + size proximity.
/// This catches renamed copies, timestamped backups, "(1)" copies, etc.
pub fn find_fuzzy_duplicates(
    entries: &[FileEntry],
    options: &DedupOptions,
) -> Result<Vec<DupGroup>> {
    let eligible: Vec<&FileEntry> = entries
        .iter()
        .filter(|e| e.size >= options.min_size)
        .collect();

    // Normalize filename → base name (strip common suffixes/prefixes)
    // Uses Unicode NFC normalization and regex for broad pattern matching
    fn normalize_name(path: &Path) -> String {
        use unicode_normalization::UnicodeNormalization;

        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // Apply Unicode NFC normalization first (handles accented chars etc.)
        let normalized: String = stem.nfc().collect();

        // Strip common copy patterns (static)
        let cleaned = normalized
            .replace("_old", "")
            .replace("_backup", "")
            .replace("_bak", "")
            .replace("_copy", "")
            .replace(" - Copy", "")
            .replace(" - copy", "")
            .replace("_final", "")
            .replace("_FINAL", "")
            .replace("_draft", "")
            .replace("-draft", "")
            .replace("~", "");

        // Regex-based stripping for numbered copies, timestamps, versions
        // e.g., "file (1)", "file (2)", "file_v2", "report_2024-01-15",
        //        "photo_20240115_143022", "doc_rev3"
        let patterns = [
            r"\s*\(\d+\)",         // " (1)", " (2)", " (42)"
            r"_v\d+",              // "_v2", "_v14"
            r"_rev\d+",            // "_rev3"
            r"_\d{4}-\d{2}-\d{2}", // "_2024-01-15"
            r"_\d{8}_\d{6}",       // "_20240115_143022"
            r"_\d{8}",             // "_20240115"
            r"\s*-\s*\d+$",        // " - 1", " - 2" at end
            r"_copy\d*",           // "_copy", "_copy2"
        ];

        let mut result = cleaned;
        for pattern in &patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                result = re.replace_all(&result, "").to_string();
            }
        }

        result = result.trim().to_lowercase();

        if result.is_empty() {
            // If stripping removed everything, use original stem
            result = stem.to_lowercase();
        }

        format!("{}.{}", result, ext)
    }

    // Group by normalized name
    let mut name_groups: HashMap<String, Vec<&FileEntry>> = HashMap::new();
    for entry in &eligible {
        let key = normalize_name(&entry.path);
        name_groups.entry(key).or_default().push(entry);
    }

    // Build lookup
    let entry_map: HashMap<String, &FileEntry> = eligible
        .iter()
        .map(|e| (e.path.to_string_lossy().to_string(), *e))
        .collect();

    let mut groups: Vec<DupGroup> = Vec::new();

    for (_name, group) in name_groups {
        if group.len() < 2 {
            continue;
        }

        // Within each name group, cluster by similar size (within 10%)
        let mut size_clusters: Vec<Vec<&FileEntry>> = Vec::new();

        for entry in &group {
            let mut found = false;
            for cluster in &mut size_clusters {
                let ref_size = cluster[0].size as f64;
                let this_size = entry.size as f64;
                let ratio = if ref_size > 0.0 {
                    (this_size / ref_size * 100.0) as u8
                } else {
                    100
                };
                let similarity = if ratio > 100 { 200 - ratio } else { ratio };

                if similarity >= options.fuzzy_threshold {
                    cluster.push(entry);
                    found = true;
                    break;
                }
            }
            if !found {
                size_clusters.push(vec![entry]);
            }
        }

        for cluster in size_clusters {
            if cluster.len() < 2 {
                continue;
            }

            let paths: Vec<PathBuf> = cluster.iter().map(|e| e.path.clone()).collect();
            let master = select_master(&paths, &entry_map, options.strategy);
            let duplicates: Vec<PathBuf> = paths.into_iter().filter(|p| p != &master).collect();
            let wasted: u64 = duplicates
                .iter()
                .filter_map(|p| {
                    let key = p.to_string_lossy().to_string();
                    entry_map.get(&key).map(|e| e.size)
                })
                .sum();

            // Compute average similarity
            let ref_size = cluster[0].size as f64;
            let avg_sim: u8 = if ref_size > 0.0 {
                let total: f64 = cluster
                    .iter()
                    .map(|e| {
                        let r = e.size as f64 / ref_size * 100.0;
                        if r > 100.0 {
                            200.0 - r
                        } else {
                            r
                        }
                    })
                    .sum();
                (total / cluster.len() as f64) as u8
            } else {
                100
            };

            groups.push(DupGroup {
                hash: None,
                similarity: avg_sim,
                master,
                duplicates,
                wasted_bytes: wasted,
            });
        }
    }

    groups.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes));
    Ok(groups)
}

// ---------------------------------------------------------------------------
// Combined analysis
// ---------------------------------------------------------------------------

/// Run full dedup analysis: exact first, then optionally fuzzy.
/// Exact groups take priority — files already in an exact group are excluded
/// from fuzzy analysis to avoid double-counting.
pub fn analyze(entries: &[FileEntry], options: &DedupOptions) -> Result<DedupReport> {
    let mut all_groups: Vec<DupGroup> = Vec::new();

    // Phase 1: exact
    let exact_groups = find_exact_duplicates(entries, options)?;
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for group in &exact_groups {
        seen.insert(group.master.clone());
        for dup in &group.duplicates {
            seen.insert(dup.clone());
        }
    }
    all_groups.extend(exact_groups);

    // Phase 2: fuzzy (on remaining files)
    if options.fuzzy {
        let remaining: Vec<FileEntry> = entries
            .iter()
            .filter(|e| !seen.contains(&e.path))
            .cloned()
            .collect();
        let fuzzy_groups = find_fuzzy_duplicates(&remaining, options)?;
        all_groups.extend(fuzzy_groups);
    }

    let total_dups: usize = all_groups.iter().map(|g| g.duplicates.len()).sum();
    let wasted: u64 = all_groups.iter().map(|g| g.wasted_bytes).sum();

    Ok(DedupReport {
        scanned_files: entries.len(),
        unique_files: entries.len() - total_dups,
        duplicate_groups: all_groups.len(),
        total_duplicates: total_dups,
        wasted_bytes: wasted,
        groups: all_groups,
        generated_at: Utc::now(),
        strategy: format!("{:?}", options.strategy),
        fuzzy_threshold: options.fuzzy_threshold,
    })
}

// ---------------------------------------------------------------------------
// Purge
// ---------------------------------------------------------------------------

/// Delete duplicate files (the non-master entries).
/// Returns (deleted_count, freed_bytes, errors).
pub fn purge_duplicates(groups: &[DupGroup], dry_run: bool) -> (usize, u64, Vec<String>) {
    let mut deleted = 0usize;
    let mut freed = 0u64;
    let mut errors = Vec::new();

    for group in groups {
        for dup in &group.duplicates {
            if dry_run {
                tracing::info!("[DRY RUN] Would delete: {}", dup.display());
                deleted += 1;
                if let Ok(meta) = std::fs::metadata(dup) {
                    freed += meta.len();
                }
            } else {
                match std::fs::metadata(dup) {
                    Ok(meta) => {
                        let size = meta.len();
                        match std::fs::remove_file(dup) {
                            Ok(()) => {
                                deleted += 1;
                                freed += size;
                                tracing::info!("Deleted: {}", dup.display());
                            }
                            Err(e) => {
                                errors.push(format!("{}: {}", dup.display(), e));
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("{}: {}", dup.display(), e));
                    }
                }
            }
        }
    }

    (deleted, freed, errors)
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

impl DedupReport {
    /// Format as human-readable summary.
    pub fn to_human_string(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!(
            "\n  Diamond Drill Dedup Report\n  {}\n\n",
            "=".repeat(40)
        ));
        out.push_str(&format!(
            "  Scanned:          {} files\n",
            self.scanned_files
        ));
        out.push_str(&format!(
            "  Unique:           {} files\n",
            self.unique_files
        ));
        out.push_str(&format!("  Duplicate groups: {}\n", self.duplicate_groups));
        out.push_str(&format!("  Total duplicates: {}\n", self.total_duplicates));
        out.push_str(&format!(
            "  Wasted space:     {}\n",
            humansize::format_size(self.wasted_bytes, humansize::BINARY)
        ));
        out.push_str(&format!("  Strategy:         {}\n", self.strategy));
        if self.fuzzy_threshold < 100 {
            out.push_str(&format!("  Fuzzy threshold:  {}%\n", self.fuzzy_threshold));
        }
        out.push_str(&format!(
            "  Generated:        {}\n\n",
            self.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));

        for (i, group) in self.groups.iter().enumerate() {
            let kind = if group.similarity == 100 {
                "EXACT"
            } else {
                "FUZZY"
            };
            out.push_str(&format!(
                "  Group #{} [{}] ({}% similar, {} wasted)\n",
                i + 1,
                kind,
                group.similarity,
                humansize::format_size(group.wasted_bytes, humansize::BINARY)
            ));
            out.push_str(&format!("    KEEP  {}\n", group.master.display()));
            for dup in &group.duplicates {
                out.push_str(&format!("    PURGE {}\n", dup.display()));
            }
            out.push('\n');
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::FileType;
    use tempfile::tempdir;

    fn make_entry(path: PathBuf, size: u64, modified: Option<DateTime<Utc>>) -> FileEntry {
        FileEntry {
            path,
            size,
            file_type: FileType::Document,
            extension: "txt".to_string(),
            modified,
            created: None,
            hash: None,
            has_bad_sectors: false,
            thumbnail: None,
        }
    }

    #[test]
    fn test_hash_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "Hello, Diamond Drill!").unwrap();

        let hash1 = hash_file(&path).unwrap();
        let hash2 = hash_file(&path).unwrap();

        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // Blake3 = 32 bytes = 64 hex chars
    }

    #[test]
    fn test_hash_file_partial_small() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("small.txt");
        std::fs::write(&path, "small file").unwrap();

        // Small file → partial falls back to full hash
        let full = hash_file(&path).unwrap();
        let partial = hash_file_partial(&path, 10).unwrap();
        assert_eq!(full, partial);
    }

    #[test]
    fn test_is_temp_name() {
        assert!(is_temp_name(Path::new("file.bak")));
        assert!(is_temp_name(Path::new("file.tmp")));
        assert!(is_temp_name(Path::new("file~")));
        assert!(is_temp_name(Path::new("file_old.txt")));
        assert!(is_temp_name(Path::new("file_backup.doc")));
        assert!(is_temp_name(Path::new("photo - Copy.jpg")));
        assert!(is_temp_name(Path::new("report (1).pdf")));
        assert!(is_temp_name(Path::new("~$document.docx")));
        assert!(!is_temp_name(Path::new("report.pdf")));
        assert!(!is_temp_name(Path::new("photo.jpg")));
    }

    #[test]
    fn test_exact_dedup_finds_identical_files() {
        let dir = tempdir().unwrap();

        let path1 = dir.path().join("a.txt");
        let path2 = dir.path().join("b.txt");
        let path3 = dir.path().join("unique.txt");

        std::fs::write(&path1, "identical content here").unwrap();
        std::fs::write(&path2, "identical content here").unwrap();
        std::fs::write(&path3, "different content").unwrap();

        let entries = vec![
            make_entry(path1.clone(), 22, Some(Utc::now())),
            make_entry(path2.clone(), 22, Some(Utc::now())),
            make_entry(path3, 17, Some(Utc::now())),
        ];

        let options = DedupOptions::default();
        let groups = find_exact_duplicates(&entries, &options).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].duplicates.len(), 1);
        assert_eq!(groups[0].similarity, 100);
    }

    #[test]
    fn test_exact_dedup_no_duplicates() {
        let dir = tempdir().unwrap();

        let path1 = dir.path().join("a.txt");
        let path2 = dir.path().join("b.txt");

        std::fs::write(&path1, "content one").unwrap();
        std::fs::write(&path2, "content two").unwrap();

        let entries = vec![
            make_entry(path1, 11, Some(Utc::now())),
            make_entry(path2, 11, Some(Utc::now())),
        ];

        let options = DedupOptions::default();
        let groups = find_exact_duplicates(&entries, &options).unwrap();

        assert!(groups.is_empty());
    }

    #[test]
    fn test_master_selection_newest() {
        let dir = tempdir().unwrap();
        let old_path = dir.path().join("old.txt");
        let new_path = dir.path().join("new.txt");

        std::fs::write(&old_path, "same").unwrap();
        std::fs::write(&new_path, "same").unwrap();

        let old_time = Utc::now() - chrono::Duration::hours(5);
        let new_time = Utc::now();

        let entries = vec![
            make_entry(old_path.clone(), 4, Some(old_time)),
            make_entry(new_path.clone(), 4, Some(new_time)),
        ];

        let options = DedupOptions {
            strategy: KeepStrategy::Newest,
            ..Default::default()
        };

        let groups = find_exact_duplicates(&entries, &options).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].master, new_path);
    }

    #[test]
    fn test_master_selection_cleanest() {
        let dir = tempdir().unwrap();
        let clean = dir.path().join("report.pdf");
        let messy = dir.path().join("report_backup.pdf");

        std::fs::write(&clean, "pdf content").unwrap();
        std::fs::write(&messy, "pdf content").unwrap();

        let entries = vec![
            make_entry(clean.clone(), 11, Some(Utc::now())),
            make_entry(messy.clone(), 11, Some(Utc::now())),
        ];

        let options = DedupOptions {
            strategy: KeepStrategy::Cleanest,
            ..Default::default()
        };

        let groups = find_exact_duplicates(&entries, &options).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].master, clean);
    }

    #[test]
    fn test_fuzzy_dedup_catches_copies() {
        let entries = vec![
            make_entry(PathBuf::from("/docs/report.txt"), 1000, Some(Utc::now())),
            make_entry(
                PathBuf::from("/docs/report_copy.txt"),
                1000,
                Some(Utc::now()),
            ),
            make_entry(
                PathBuf::from("/docs/report (1).txt"),
                1020,
                Some(Utc::now()),
            ),
        ];

        let options = DedupOptions {
            fuzzy: true,
            fuzzy_threshold: 85,
            ..Default::default()
        };

        let groups = find_fuzzy_duplicates(&entries, &options).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].duplicates.len(), 2); // 2 dups, 1 master
    }

    #[test]
    fn test_full_analyze() {
        let dir = tempdir().unwrap();

        let p1 = dir.path().join("a.txt");
        let p2 = dir.path().join("a_copy.txt");
        let p3 = dir.path().join("unique.txt");

        std::fs::write(&p1, "duplicate data").unwrap();
        std::fs::write(&p2, "duplicate data").unwrap();
        std::fs::write(&p3, "solo").unwrap();

        let entries = vec![
            make_entry(p1, 14, Some(Utc::now())),
            make_entry(p2, 14, Some(Utc::now())),
            make_entry(p3, 4, Some(Utc::now())),
        ];

        let options = DedupOptions::default();
        let report = analyze(&entries, &options).unwrap();

        assert_eq!(report.scanned_files, 3);
        assert_eq!(report.duplicate_groups, 1);
        assert_eq!(report.total_duplicates, 1);
        assert_eq!(report.unique_files, 2);
        assert_eq!(report.wasted_bytes, 14);
    }

    #[test]
    fn test_purge_dry_run() {
        let dir = tempdir().unwrap();
        let p1 = dir.path().join("keep.txt");
        let p2 = dir.path().join("delete.txt");

        std::fs::write(&p1, "keep").unwrap();
        std::fs::write(&p2, "delete").unwrap();

        let groups = vec![DupGroup {
            hash: Some("abc123".to_string()),
            similarity: 100,
            master: p1.clone(),
            duplicates: vec![p2.clone()],
            wasted_bytes: 6,
        }];

        let (deleted, _freed, errors) = purge_duplicates(&groups, true);
        assert_eq!(deleted, 1);
        assert!(errors.is_empty());
        // File should still exist (dry run)
        assert!(p2.exists());
    }

    #[test]
    fn test_purge_real() {
        let dir = tempdir().unwrap();
        let p1 = dir.path().join("keep.txt");
        let p2 = dir.path().join("delete.txt");

        std::fs::write(&p1, "keep").unwrap();
        std::fs::write(&p2, "delete").unwrap();

        let groups = vec![DupGroup {
            hash: Some("abc123".to_string()),
            similarity: 100,
            master: p1.clone(),
            duplicates: vec![p2.clone()],
            wasted_bytes: 6,
        }];

        let (deleted, freed, errors) = purge_duplicates(&groups, false);
        assert_eq!(deleted, 1);
        assert_eq!(freed, 6);
        assert!(errors.is_empty());
        assert!(!p2.exists()); // Actually deleted
        assert!(p1.exists()); // Master preserved
    }

    #[test]
    fn test_report_human_string() {
        let report = DedupReport {
            scanned_files: 100,
            unique_files: 90,
            duplicate_groups: 5,
            total_duplicates: 10,
            wasted_bytes: 1024 * 1024, // 1 MB
            groups: vec![],
            generated_at: Utc::now(),
            strategy: "Newest".to_string(),
            fuzzy_threshold: 85,
        };

        let output = report.to_human_string();
        assert!(output.contains("100 files"));
        assert!(output.contains("5"));
        assert!(output.contains("Newest"));
    }
}
