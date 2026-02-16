//! FileIndex - In-memory file index with serialization
//!
//! Provides fast lookup and persistence of file metadata.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BadSector, FileType};

/// A single file entry in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Full path to the file
    pub path: PathBuf,
    /// File size in bytes
    pub size: u64,
    /// File type category
    pub file_type: FileType,
    /// File extension (lowercase)
    pub extension: String,
    /// Last modified time
    pub modified: Option<DateTime<Utc>>,
    /// Creation time (if available)
    pub created: Option<DateTime<Utc>>,
    /// BLAKE3 hash (computed on demand)
    pub hash: Option<String>,
    /// Is this file in a bad sector region?
    pub has_bad_sectors: bool,
    /// Thumbnail path (if generated)
    pub thumbnail: Option<PathBuf>,
}

impl FileEntry {
    /// Create a new file entry from path and metadata
    pub fn new(path: PathBuf, metadata: &std::fs::Metadata) -> Self {
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let file_type = FileType::from_extension(&extension);

        let modified = metadata.modified().ok().map(DateTime::<Utc>::from);

        let created = metadata.created().ok().map(DateTime::<Utc>::from);

        Self {
            path,
            size: metadata.len(),
            file_type,
            extension,
            modified,
            created,
            hash: None,
            has_bad_sectors: false,
            thumbnail: None,
        }
    }

    /// Get display name (filename only)
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.to_string_lossy().to_string())
    }
}

/// Statistics about indexing
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_files: usize,
    pub total_bytes: u64,
    pub files_by_type: HashMap<FileType, usize>,
    pub bytes_by_type: HashMap<FileType, u64>,
    pub indexed_at: Option<DateTime<Utc>>,
    pub scan_duration_ms: u64,
    pub bad_sector_count: usize,
    pub error_count: usize,
}

/// The main file index
#[derive(Debug, Serialize, Deserialize)]
pub struct FileIndex {
    /// Source path this index was created from
    source: PathBuf,
    /// Version for compatibility
    version: u32,
    /// Index creation time
    created_at: DateTime<Utc>,
    /// Last update time
    updated_at: DateTime<Utc>,
    /// All file entries
    entries: Vec<FileEntry>,
    /// Bad sectors encountered during indexing
    #[serde(default)]
    bad_sectors: Vec<BadSector>,
    /// Path to entry index for fast lookup
    #[serde(skip)]
    path_index: HashMap<String, usize>,
    /// Total bytes
    #[serde(skip)]
    total_bytes: AtomicU64,
}

impl FileIndex {
    const VERSION: u32 = 1;

    /// Create a new empty index
    pub fn new(source: PathBuf) -> Self {
        Self {
            source,
            version: Self::VERSION,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            entries: Vec::new(),
            bad_sectors: Vec::new(),
            path_index: HashMap::new(),
            total_bytes: AtomicU64::new(0),
        }
    }

    /// Load index from file
    pub async fn load(path: &Path) -> Result<Self> {
        let owned_path = path.to_path_buf();
        let data = tokio::task::spawn_blocking(move || std::fs::read(&owned_path)).await??;
        let mut index: Self = bincode::deserialize(&data)?;

        // Rebuild path index
        index.path_index = index
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.path.to_string_lossy().to_string(), i))
            .collect();

        // Recalculate total bytes
        let total: u64 = index.entries.iter().map(|e| e.size).sum();
        index.total_bytes = AtomicU64::new(total);

        Ok(index)
    }

    /// Save index to file
    pub async fn save(&self, path: &Path) -> Result<()> {
        let owned_path = path.to_path_buf();
        let data = bincode::serialize(self)?;
        tokio::task::spawn_blocking(move || std::fs::write(&owned_path, data)).await??;
        Ok(())
    }

    /// Add a file entry
    pub fn add_entry(&mut self, entry: FileEntry) {
        let path_str = entry.path.to_string_lossy().to_string();

        // Update total bytes
        self.total_bytes.fetch_add(entry.size, Ordering::Relaxed);

        // Check if already exists
        if let Some(&idx) = self.path_index.get(&path_str) {
            // Update existing
            let old_size = self.entries[idx].size;
            self.total_bytes.fetch_sub(old_size, Ordering::Relaxed);
            self.entries[idx] = entry;
        } else {
            // Add new
            let idx = self.entries.len();
            self.path_index.insert(path_str, idx);
            self.entries.push(entry);
        }

        self.updated_at = Utc::now();
    }

    /// Get entry by path
    pub fn get_by_path(&self, path: &str) -> Option<&FileEntry> {
        self.path_index
            .get(path)
            .and_then(|&idx| self.entries.get(idx))
    }

    /// Get all entries iterator
    pub fn entries(&self) -> impl Iterator<Item = &FileEntry> {
        self.entries.iter()
    }

    /// Get entry count
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get total bytes
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }

    /// Get statistics
    pub fn stats(&self) -> IndexStats {
        let mut stats = IndexStats {
            total_files: self.entries.len(),
            total_bytes: self.total_bytes(),
            indexed_at: Some(self.updated_at),
            ..Default::default()
        };

        for entry in &self.entries {
            *stats.files_by_type.entry(entry.file_type).or_insert(0) += 1;
            *stats.bytes_by_type.entry(entry.file_type).or_insert(0) += entry.size;
            if entry.has_bad_sectors {
                stats.bad_sector_count += 1;
            }
        }

        stats
    }

    /// Filter entries by predicate
    pub fn filter<F>(&self, predicate: F) -> Vec<&FileEntry>
    where
        F: Fn(&FileEntry) -> bool,
    {
        self.entries.iter().filter(|e| predicate(e)).collect()
    }

    /// Get source path
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Add a bad sector record
    pub fn add_bad_sector(&mut self, bad_sector: BadSector) {
        self.bad_sectors.push(bad_sector);
        self.updated_at = Utc::now();
    }

    /// Add multiple bad sectors (appends to existing)
    pub fn add_bad_sectors(&mut self, sectors: Vec<BadSector>) {
        self.bad_sectors.extend(sectors);
        self.updated_at = Utc::now();
    }

    /// Replace all bad sectors (use when re-indexing to avoid duplicates)
    pub fn set_bad_sectors(&mut self, sectors: Vec<BadSector>) {
        self.bad_sectors = sectors;
        self.updated_at = Utc::now();
    }

    /// Clear all bad sectors
    pub fn clear_bad_sectors(&mut self) {
        self.bad_sectors.clear();
        self.updated_at = Utc::now();
    }

    /// Get bad sectors
    pub fn bad_sectors(&self) -> &[BadSector] {
        &self.bad_sectors
    }

    /// Get bad sector count
    pub fn bad_sector_count(&self) -> usize {
        self.bad_sectors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_file_type_from_extension() {
        assert_eq!(FileType::from_extension("jpg"), FileType::Image);
        assert_eq!(FileType::from_extension("MP4"), FileType::Video);
        assert_eq!(FileType::from_extension("rs"), FileType::Code);
        assert_eq!(FileType::from_extension("xyz"), FileType::Other);
    }

    #[tokio::test]
    async fn test_index_save_load() {
        let dir = tempdir().unwrap();
        let index_path = dir.path().join("test.idx");

        let mut index = FileIndex::new(PathBuf::from("/test"));

        // Create a mock entry
        let entry = FileEntry {
            path: PathBuf::from("/test/photo.jpg"),
            size: 1024,
            file_type: FileType::Image,
            extension: "jpg".to_string(),
            modified: Some(Utc::now()),
            created: None,
            hash: None,
            has_bad_sectors: false,
            thumbnail: None,
        };

        index.add_entry(entry);

        // Save
        index.save(&index_path).await.unwrap();

        // Load
        let loaded = FileIndex::load(&index_path).await.unwrap();

        assert_eq!(loaded.len(), 1);
        assert!(loaded.get_by_path("/test/photo.jpg").is_some());
    }

    #[tokio::test]
    async fn test_bad_sectors_persist() {
        let dir = tempdir().unwrap();
        let index_path = dir.path().join("test_bad.idx");

        let mut index = FileIndex::new(PathBuf::from("/test"));

        // Add entry with bad sectors
        let entry = FileEntry {
            path: PathBuf::from("/test/corrupted.bin"),
            size: 2048,
            file_type: FileType::Other,
            extension: "bin".to_string(),
            modified: Some(Utc::now()),
            created: None,
            hash: None,
            has_bad_sectors: true,
            thumbnail: None,
        };
        index.add_entry(entry);

        // Add bad sector records
        let bad1 = BadSector {
            file_path: PathBuf::from("/test/corrupted.bin"),
            offset: 512,
            length: 256,
            error: "Read error at sector".to_string(),
            detected_at: Utc::now(),
            retry_count: 0,
            block_size: 4096,
        };
        let bad2 = BadSector {
            file_path: PathBuf::from("/test/another.bin"),
            offset: 0,
            length: 1024,
            error: "Unreadable sector".to_string(),
            detected_at: Utc::now(),
            retry_count: 0,
            block_size: 4096,
        };
        index.add_bad_sectors(vec![bad1, bad2]);

        // Save
        index.save(&index_path).await.unwrap();

        // Load and verify bad sectors persisted
        let loaded = FileIndex::load(&index_path).await.unwrap();

        assert_eq!(loaded.bad_sector_count(), 2);
        assert_eq!(loaded.bad_sectors()[0].offset, 512);
        assert_eq!(loaded.bad_sectors()[1].length, 1024);

        // Stats should reflect bad sectors
        let stats = loaded.stats();
        assert_eq!(stats.bad_sector_count, 1); // From entries with has_bad_sectors=true
    }
}
