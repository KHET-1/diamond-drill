//! Media-Aware Chunker - Intelligent document splitting for text/code/image/PDF
//!
//! Implements adaptive chunking strategies based on content type:
//! - Text: Sentence/paragraph boundaries with overlap
//! - Code: Function/class/block boundaries with syntax awareness
//! - Image: Metadata extraction (no chunking, single "chunk")
//! - PDF: Page-based extraction with text flow preservation

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

// ============================================================================
// Core Types
// ============================================================================

/// Media type detection result
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaType {
    Text,
    Markdown,
    Code,
    Image,
    Pdf,
    Binary,
    Unknown,
}

impl MediaType {
    /// Detect media type from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // Text
            "txt" | "text" | "log" | "csv" => MediaType::Text,

            // Markdown
            "md" | "markdown" | "mdx" | "rst" => MediaType::Markdown,

            // Code
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp" | "h"
            | "hpp" | "cs" | "rb" | "php" | "swift" | "kt" | "scala" | "r" | "sql" | "sh"
            | "bash" | "zsh" | "ps1" | "psm1" | "lua" | "zig" | "nim" | "v" | "d" | "ml" | "hs"
            | "elm" | "ex" | "exs" | "clj" | "cljs" | "erl" | "fs" | "fsx" | "toml" | "yaml"
            | "yml" | "json" | "xml" | "html" | "css" | "scss" | "sass" | "less" | "vue"
            | "svelte" => MediaType::Code,

            // Images
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tiff" | "tif"
            | "heic" | "heif" | "avif" => MediaType::Image,

            // PDF
            "pdf" => MediaType::Pdf,

            // Binary
            "exe" | "dll" | "so" | "dylib" | "bin" | "o" | "a" | "lib" | "wasm" | "zip" | "tar"
            | "gz" | "bz2" | "xz" | "7z" | "rar" => MediaType::Binary,

            _ => MediaType::Unknown,
        }
    }

    /// Detect from path
    pub fn from_path(path: &Path) -> Self {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(MediaType::Unknown)
    }

    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaType::Text => "text",
            MediaType::Markdown => "markdown",
            MediaType::Code => "code",
            MediaType::Image => "image",
            MediaType::Pdf => "pdf",
            MediaType::Binary => "binary",
            MediaType::Unknown => "unknown",
        }
    }
}

/// A chunk of content with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique chunk ID (file_path + chunk_index)
    pub id: String,
    /// Source file path
    pub source: PathBuf,
    /// Chunk index within the file (0-based)
    pub index: usize,
    /// Total chunks in the file
    pub total: usize,
    /// The actual content
    pub content: String,
    /// Byte offset in original file
    pub byte_start: usize,
    /// Byte end offset
    pub byte_end: usize,
    /// Media type of source
    pub media_type: MediaType,
    /// Additional metadata (language, page number, etc.)
    pub metadata: HashMap<String, String>,
}

impl Chunk {
    /// Create a new chunk
    pub fn new(
        source: PathBuf,
        index: usize,
        total: usize,
        content: String,
        byte_start: usize,
        byte_end: usize,
        media_type: MediaType,
    ) -> Self {
        let id = format!("{}:{}", source.display(), index);
        Self {
            id,
            source,
            index,
            total,
            content,
            byte_start,
            byte_end,
            media_type,
            metadata: HashMap::new(),
        }
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Content length in bytes
    pub fn len(&self) -> usize {
        self.content.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Content with metadata prefix for embedding (improves retrieval accuracy).
    ///
    /// Format:
    /// ```text
    /// File: report.pdf | Type: pdf | Chunk: 3/12 | Bytes: 1024-2048
    /// ---
    /// [actual content]
    /// ```
    pub fn content_with_prefix(&self) -> String {
        let filename = self
            .source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let mut prefix = format!(
            "File: {} | Type: {} | Chunk: {}/{}",
            filename,
            self.media_type.as_str(),
            self.index + 1,
            if self.total > 0 { self.total } else { 1 },
        );

        // Add byte range
        prefix.push_str(&format!(" | Bytes: {}-{}", self.byte_start, self.byte_end));

        // Add any extra metadata (language, heading, page, etc.)
        for (key, value) in &self.metadata {
            if key != "size_bytes" && key != "requires_extraction" {
                prefix.push_str(&format!(" | {}: {}", key, value));
            }
        }

        format!("{}\n---\n{}", prefix, self.content)
    }
}

/// Chunking configuration
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Target chunk size in bytes
    pub chunk_size: usize,
    /// Overlap between chunks in bytes
    pub overlap: usize,
    /// Minimum chunk size (skip smaller)
    pub min_chunk_size: usize,
    /// Maximum chunk size (hard limit)
    pub max_chunk_size: usize,
    /// Preserve sentence boundaries for text
    pub preserve_sentences: bool,
    /// Preserve code block boundaries
    pub preserve_code_blocks: bool,
    /// Include file metadata in chunks
    pub include_metadata: bool,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_size: 1024,
            overlap: 128,
            min_chunk_size: 64,
            max_chunk_size: 8192,
            preserve_sentences: true,
            preserve_code_blocks: true,
            include_metadata: true,
        }
    }
}

// ============================================================================
// Chunker Trait & Implementations
// ============================================================================

/// Trait for media-specific chunking strategies
pub trait ChunkStrategy: Send + Sync {
    /// Chunk the content
    fn chunk(&self, path: &Path, content: &str, config: &ChunkConfig) -> Result<Vec<Chunk>>;

    /// Supported media types
    fn supported_types(&self) -> &[MediaType];
}

/// Text chunker - sentence/paragraph aware
pub struct TextChunker;

impl TextChunker {
    /// Find sentence boundaries
    fn find_sentence_end(text: &str, start: usize, target: usize) -> usize {
        let search_range = &text[start..std::cmp::min(text.len(), target + 200)];

        // Look for sentence endings near target
        let sentence_ends = [
            ". ", ".\n", "! ", "!\n", "? ", "?\n", ".\r\n", "!\r\n", "?\r\n",
        ];

        let mut best_end = target.saturating_sub(start);
        let mut best_distance = usize::MAX;

        for end in &sentence_ends {
            if let Some(pos) = search_range.find(end) {
                let actual_pos = pos + end.len() - 1; // Include the punctuation
                let distance = if actual_pos > target.saturating_sub(start) {
                    actual_pos - (target - start)
                } else {
                    (target - start) - actual_pos
                };

                if distance < best_distance {
                    best_distance = distance;
                    best_end = actual_pos;
                }
            }
        }

        start + best_end
    }

    /// Find paragraph boundary
    fn find_paragraph_end(text: &str, start: usize, target: usize) -> usize {
        let search_range = &text[start..std::cmp::min(text.len(), target + 500)];

        // Look for paragraph breaks
        if let Some(pos) = search_range.find("\n\n") {
            if pos <= target - start + 200 {
                return start + pos + 2;
            }
        }
        if let Some(pos) = search_range.find("\r\n\r\n") {
            if pos <= target - start + 200 {
                return start + pos + 4;
            }
        }

        // Fallback to sentence boundary
        Self::find_sentence_end(text, start, target)
    }
}

impl ChunkStrategy for TextChunker {
    fn chunk(&self, path: &Path, content: &str, config: &ChunkConfig) -> Result<Vec<Chunk>> {
        if content.is_empty() {
            return Ok(vec![]);
        }

        let mut chunks = Vec::new();
        let mut start = 0;
        let content_len = content.len();

        while start < content_len {
            let target_end = std::cmp::min(start + config.chunk_size, content_len);

            // Find natural boundary
            let end = if config.preserve_sentences && target_end < content_len {
                Self::find_paragraph_end(content, start, target_end)
            } else {
                target_end
            };

            // Ensure we don't exceed max
            let end = std::cmp::min(end, start + config.max_chunk_size);
            let end = std::cmp::min(end, content_len);

            let chunk_content = content[start..end].to_string();

            // Skip if too small (unless it's the only/last chunk)
            if chunk_content.len() >= config.min_chunk_size
                || chunks.is_empty()
                || end == content_len
            {
                chunks.push(Chunk::new(
                    path.to_path_buf(),
                    chunks.len(),
                    0, // Will be updated after
                    chunk_content,
                    start,
                    end,
                    MediaType::Text,
                ));
            }

            // Move start with overlap, but ALWAYS advance by at least 1
            start = if end >= content_len {
                content_len
            } else {
                let next = end.saturating_sub(config.overlap);
                // Guard: never go backwards or stall
                std::cmp::max(next, start + 1)
            };
        }

        // Update total count
        let total = chunks.len();
        for chunk in &mut chunks {
            chunk.total = total;
        }

        Ok(chunks)
    }

    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Text, MediaType::Unknown]
    }
}

/// Markdown chunker - preserves headers and code blocks
pub struct MarkdownChunker;

impl MarkdownChunker {
    /// Find heading level
    fn heading_level(line: &str) -> Option<usize> {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            if hashes <= 6 && trimmed.chars().nth(hashes) == Some(' ') {
                return Some(hashes);
            }
        }
        None
    }

    /// Check if line starts/ends code block
    fn is_code_fence(line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.starts_with("```") || trimmed.starts_with("~~~")
    }
}

impl ChunkStrategy for MarkdownChunker {
    fn chunk(&self, path: &Path, content: &str, config: &ChunkConfig) -> Result<Vec<Chunk>> {
        if content.is_empty() {
            return Ok(vec![]);
        }

        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut current_start = 0;
        let mut in_code_block = false;
        let mut current_heading: Option<String> = None;

        for line in content.lines() {
            let line_with_newline = format!("{}\n", line);

            // Track code blocks
            if Self::is_code_fence(line) {
                in_code_block = !in_code_block;
            }

            // Check for heading (only outside code blocks)
            if !in_code_block {
                if let Some(_level) = Self::heading_level(line) {
                    // If we have content and it's big enough, emit chunk
                    if current_chunk.len() >= config.min_chunk_size {
                        let byte_end = current_start + current_chunk.len();
                        let mut chunk = Chunk::new(
                            path.to_path_buf(),
                            chunks.len(),
                            0,
                            current_chunk.clone(),
                            current_start,
                            byte_end,
                            MediaType::Markdown,
                        );
                        if let Some(ref h) = current_heading {
                            chunk = chunk.with_metadata("heading", h.clone());
                        }
                        chunks.push(chunk);
                        current_start = byte_end;
                        current_chunk.clear();
                    }
                    current_heading = Some(line.trim().to_string());
                }
            }

            current_chunk.push_str(&line_with_newline);

            // Check size limit (but don't break code blocks if preserve is on)
            let should_split = current_chunk.len() >= config.chunk_size
                && (!config.preserve_code_blocks || !in_code_block);

            if should_split {
                let byte_end = current_start + current_chunk.len();
                let mut chunk = Chunk::new(
                    path.to_path_buf(),
                    chunks.len(),
                    0,
                    current_chunk.clone(),
                    current_start,
                    byte_end,
                    MediaType::Markdown,
                );
                if let Some(ref h) = current_heading {
                    chunk = chunk.with_metadata("heading", h.clone());
                }
                chunks.push(chunk);

                // Start new chunk with overlap
                let overlap_start = current_chunk.len().saturating_sub(config.overlap);
                current_chunk = current_chunk[overlap_start..].to_string();
                current_start = byte_end - current_chunk.len();
            }
        }

        // Final chunk
        if !current_chunk.is_empty() && current_chunk.len() >= config.min_chunk_size {
            let byte_end = current_start + current_chunk.len();
            let mut chunk = Chunk::new(
                path.to_path_buf(),
                chunks.len(),
                0,
                current_chunk,
                current_start,
                byte_end,
                MediaType::Markdown,
            );
            if let Some(ref h) = current_heading {
                chunk = chunk.with_metadata("heading", h.clone());
            }
            chunks.push(chunk);
        }

        // Update totals
        let total = chunks.len();
        for chunk in &mut chunks {
            chunk.total = total;
        }

        Ok(chunks)
    }

    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Markdown]
    }
}

/// Code chunker - function/class/block aware
pub struct CodeChunker;

impl CodeChunker {
    /// Detect programming language from extension
    fn detect_language(path: &Path) -> &'static str {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| match ext.to_lowercase().as_str() {
                "rs" => "rust",
                "py" => "python",
                "js" | "jsx" => "javascript",
                "ts" | "tsx" => "typescript",
                "go" => "go",
                "java" => "java",
                "c" | "h" => "c",
                "cpp" | "hpp" | "cc" | "cxx" => "cpp",
                "cs" => "csharp",
                "rb" => "ruby",
                "php" => "php",
                "swift" => "swift",
                "kt" => "kotlin",
                "scala" => "scala",
                "r" => "r",
                "sql" => "sql",
                "sh" | "bash" | "zsh" => "shell",
                "ps1" | "psm1" => "powershell",
                "lua" => "lua",
                "zig" => "zig",
                _ => "unknown",
            })
            .unwrap_or("unknown")
    }

    /// Find block boundaries (functions, classes, etc.)
    fn find_block_end(content: &str, start: usize, target: usize) -> usize {
        let search_range = &content[start..std::cmp::min(content.len(), target + 500)];

        // Look for common block endings
        let block_patterns = [
            "\n}\n",   // Most C-like languages
            "\n}\r\n", // Windows
            "\n\n",    // Blank line (Python, etc.)
            "\nend\n", // Ruby, Lua
            "\nend\r\n",
        ];

        let mut best_end = target.saturating_sub(start);
        let mut best_distance = usize::MAX;

        for pattern in &block_patterns {
            if let Some(pos) = search_range.find(pattern) {
                let actual_pos = pos + pattern.len();
                let distance = if actual_pos > target.saturating_sub(start) {
                    actual_pos - (target - start)
                } else {
                    (target - start) - actual_pos
                };

                if distance < best_distance && actual_pos <= target - start + 300 {
                    best_distance = distance;
                    best_end = actual_pos;
                }
            }
        }

        start + best_end
    }
}

impl ChunkStrategy for CodeChunker {
    fn chunk(&self, path: &Path, content: &str, config: &ChunkConfig) -> Result<Vec<Chunk>> {
        if content.is_empty() {
            return Ok(vec![]);
        }

        let language = Self::detect_language(path);
        let mut chunks = Vec::new();
        let mut start = 0;
        let content_len = content.len();

        while start < content_len {
            let target_end = std::cmp::min(start + config.chunk_size, content_len);

            // Find natural code boundary
            let end = if config.preserve_code_blocks && target_end < content_len {
                Self::find_block_end(content, start, target_end)
            } else {
                target_end
            };

            let end = std::cmp::min(end, start + config.max_chunk_size);
            let end = std::cmp::min(end, content_len);

            let chunk_content = content[start..end].to_string();

            if chunk_content.len() >= config.min_chunk_size
                || chunks.is_empty()
                || end == content_len
            {
                let chunk = Chunk::new(
                    path.to_path_buf(),
                    chunks.len(),
                    0,
                    chunk_content,
                    start,
                    end,
                    MediaType::Code,
                )
                .with_metadata("language", language);

                chunks.push(chunk);
            }

            start = if end >= content_len {
                content_len
            } else {
                let next = end.saturating_sub(config.overlap);
                std::cmp::max(next, start + 1)
            };
        }

        let total = chunks.len();
        for chunk in &mut chunks {
            chunk.total = total;
        }

        Ok(chunks)
    }

    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Code]
    }
}

/// Image chunker - returns metadata only (no text chunking)
pub struct ImageChunker;

impl ChunkStrategy for ImageChunker {
    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Image]
    }

    fn chunk(&self, path: &Path, _content: &str, _config: &ChunkConfig) -> Result<Vec<Chunk>> {
        // For images, we create a single "chunk" with metadata
        // In production, this would extract EXIF data, run OCR, etc.

        let metadata = fs::metadata(path).ok();
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown");

        let description = format!(
            "[Image: {} ({} bytes)]",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown"),
            size
        );

        let chunk = Chunk::new(
            path.to_path_buf(),
            0,
            1,
            description,
            0,
            size as usize,
            MediaType::Image,
        )
        .with_metadata("format", ext.to_string())
        .with_metadata("size_bytes", size.to_string());

        Ok(vec![chunk])
    }
}

/// PDF chunker - page-based with text extraction via pdf-extract
pub struct PdfChunker;

impl PdfChunker {
    /// Extract text from a PDF file using pdf-extract
    fn extract_text(path: &Path) -> Result<String> {
        let bytes =
            fs::read(path).with_context(|| format!("Failed to read PDF: {}", path.display()))?;

        pdf_extract::extract_text_from_mem(&bytes)
            .with_context(|| format!("Failed to extract text from PDF: {}", path.display()))
    }
}

impl ChunkStrategy for PdfChunker {
    fn chunk(&self, path: &Path, content: &str, config: &ChunkConfig) -> Result<Vec<Chunk>> {
        // If content is provided (pre-extracted text), use text chunker
        let text = if !content.is_empty() {
            content.to_string()
        } else {
            // Extract text from the PDF file
            match Self::extract_text(path) {
                Ok(extracted) if !extracted.trim().is_empty() => extracted,
                Ok(_) => {
                    // PDF exists but has no extractable text (scanned/image-only)
                    let metadata = fs::metadata(path).ok();
                    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

                    let description = format!(
                        "[PDF: {} ({} bytes) - no extractable text (scanned/image-only)]",
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown"),
                        size
                    );

                    let chunk = Chunk::new(
                        path.to_path_buf(),
                        0,
                        1,
                        description,
                        0,
                        size as usize,
                        MediaType::Pdf,
                    )
                    .with_metadata("size_bytes", size.to_string())
                    .with_metadata("scanned", "true");

                    return Ok(vec![chunk]);
                }
                Err(e) => {
                    // Extraction failed — create a fallback metadata chunk
                    tracing::warn!("PDF extraction failed for {}: {}", path.display(), e);
                    let metadata = fs::metadata(path).ok();
                    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

                    let description = format!(
                        "[PDF: {} ({} bytes) - extraction failed: {}]",
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown"),
                        size,
                        e
                    );

                    let chunk = Chunk::new(
                        path.to_path_buf(),
                        0,
                        1,
                        description,
                        0,
                        size as usize,
                        MediaType::Pdf,
                    )
                    .with_metadata("size_bytes", size.to_string())
                    .with_metadata("extraction_error", e.to_string());

                    return Ok(vec![chunk]);
                }
            }
        };

        // Chunk the extracted text using the text chunker
        let text_chunker = TextChunker;
        let mut chunks = text_chunker.chunk(path, &text, config)?;

        // Update media type to PDF
        for chunk in &mut chunks {
            chunk.media_type = MediaType::Pdf;
        }

        Ok(chunks)
    }

    fn supported_types(&self) -> &[MediaType] {
        &[MediaType::Pdf]
    }
}

// ============================================================================
// Unified Chunker
// ============================================================================

/// Unified chunker that dispatches to media-specific strategies
pub struct MediaAwareChunker {
    strategies: HashMap<MediaType, Arc<dyn ChunkStrategy>>,
    config: ChunkConfig,
}

impl MediaAwareChunker {
    /// Create with default strategies
    pub fn new(config: ChunkConfig) -> Self {
        let mut strategies: HashMap<MediaType, Arc<dyn ChunkStrategy>> = HashMap::new();

        strategies.insert(MediaType::Text, Arc::new(TextChunker));
        strategies.insert(MediaType::Markdown, Arc::new(MarkdownChunker));
        strategies.insert(MediaType::Code, Arc::new(CodeChunker));
        strategies.insert(MediaType::Image, Arc::new(ImageChunker));
        strategies.insert(MediaType::Pdf, Arc::new(PdfChunker));
        strategies.insert(MediaType::Unknown, Arc::new(TextChunker)); // Fallback

        Self { strategies, config }
    }

    /// Register a custom strategy
    pub fn register_strategy(&mut self, media_type: MediaType, strategy: Arc<dyn ChunkStrategy>) {
        self.strategies.insert(media_type, strategy);
    }

    /// Chunk a single file
    pub fn chunk_file(&self, path: &Path) -> Result<Vec<Chunk>> {
        let media_type = MediaType::from_path(path);

        // Skip binary files
        if media_type == MediaType::Binary {
            return Ok(vec![]);
        }

        // Read content (for non-image files)
        let content = if media_type == MediaType::Image {
            String::new()
        } else {
            fs::read_to_string(path)
                .with_context(|| format!("Failed to read file: {}", path.display()))?
        };

        // Get strategy
        let strategy = self
            .strategies
            .get(&media_type)
            .or_else(|| self.strategies.get(&MediaType::Unknown))
            .ok_or_else(|| anyhow::anyhow!("No chunking strategy for {:?}", media_type))?;

        strategy.chunk(path, &content, &self.config)
    }

    /// Chunk multiple files in parallel
    pub fn chunk_files(&self, paths: &[PathBuf]) -> Vec<Result<Vec<Chunk>>> {
        paths.par_iter().map(|path| self.chunk_file(path)).collect()
    }

    /// Chunk a directory recursively
    pub fn chunk_directory(&self, dir: &Path, extensions: Option<&[&str]>) -> Result<Vec<Chunk>> {
        let mut files = Vec::new();
        self.collect_files(dir, extensions, &mut files)?;

        let results: Vec<_> = self.chunk_files(&files);

        let mut all_chunks = Vec::new();
        for result in results {
            match result {
                Ok(chunks) => all_chunks.extend(chunks),
                Err(e) => {
                    // Log but continue
                    eprintln!("Chunking error: {}", e);
                }
            }
        }

        Ok(all_chunks)
    }

    /// Recursively collect files
    fn collect_files(
        &self,
        dir: &Path,
        extensions: Option<&[&str]>,
        files: &mut Vec<PathBuf>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Skip hidden directories
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(false)
                {
                    continue;
                }
                self.collect_files(&path, extensions, files)?;
            } else if path.is_file() {
                // Filter by extension if specified
                if let Some(exts) = extensions {
                    let file_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if !exts.iter().any(|e| e.eq_ignore_ascii_case(file_ext)) {
                        continue;
                    }
                }
                files.push(path);
            }
        }

        Ok(())
    }
}

impl Default for MediaAwareChunker {
    fn default() -> Self {
        Self::new(ChunkConfig::default())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_media_type_detection() {
        assert_eq!(MediaType::from_extension("rs"), MediaType::Code);
        assert_eq!(MediaType::from_extension("py"), MediaType::Code);
        assert_eq!(MediaType::from_extension("md"), MediaType::Markdown);
        assert_eq!(MediaType::from_extension("txt"), MediaType::Text);
        assert_eq!(MediaType::from_extension("png"), MediaType::Image);
        assert_eq!(MediaType::from_extension("pdf"), MediaType::Pdf);
        assert_eq!(MediaType::from_extension("exe"), MediaType::Binary);
        assert_eq!(MediaType::from_extension("xyz"), MediaType::Unknown);
    }

    #[test]
    fn test_text_chunker() {
        let chunker = TextChunker;
        let config = ChunkConfig {
            chunk_size: 100,
            overlap: 20,
            min_chunk_size: 10,
            ..Default::default()
        };

        let content = "This is sentence one. This is sentence two. This is sentence three. This is sentence four.";
        let path = Path::new("test.txt");

        let chunks = chunker.chunk(path, content, &config).unwrap();
        assert!(!chunks.is_empty());

        // Verify overlap exists between consecutive chunks
        if chunks.len() > 1 {
            let chunk0 = &chunks[0].content;
            let chunk1 = &chunks[1].content;
            // The end of chunk 0 should appear at the start of chunk 1
            let tail = &chunk0[chunk0.len().saturating_sub(20)..];
            assert!(
                chunk1.starts_with(tail) || chunk1.contains(tail),
                "Expected overlap between chunks"
            );
        }
    }

    #[test]
    fn test_markdown_chunker() {
        let chunker = MarkdownChunker;
        let config = ChunkConfig {
            chunk_size: 50,
            min_chunk_size: 10,
            ..Default::default()
        };

        let content = "# Header One\n\nSome content here.\n\n## Header Two\n\nMore content.\n";
        let path = Path::new("test.md");

        let chunks = chunker.chunk(path, content, &config).unwrap();
        assert!(!chunks.is_empty());

        // Check that heading metadata is preserved
        let has_heading = chunks.iter().any(|c| c.metadata.contains_key("heading"));
        assert!(has_heading);
    }

    #[test]
    fn test_code_chunker() {
        let chunker = CodeChunker;
        let config = ChunkConfig {
            chunk_size: 100,
            min_chunk_size: 10,
            ..Default::default()
        };

        let content = r#"
fn main() {
    println!("Hello");
}

fn other() {
    println!("World");
}
"#;
        let path = Path::new("test.rs");

        let chunks = chunker.chunk(path, content, &config).unwrap();
        assert!(!chunks.is_empty());

        // Check language metadata
        assert_eq!(
            chunks[0].metadata.get("language"),
            Some(&"rust".to_string())
        );
    }

    #[test]
    fn test_media_aware_chunker() {
        let temp_dir = TempDir::new().unwrap();

        // Create test files
        let txt_path = temp_dir.path().join("test.txt");
        let mut txt_file = fs::File::create(&txt_path).unwrap();
        writeln!(txt_file, "This is a test file with some content.").unwrap();

        let md_path = temp_dir.path().join("test.md");
        let mut md_file = fs::File::create(&md_path).unwrap();
        writeln!(md_file, "# Test\n\nSome markdown content with enough text to meet the minimum chunk size threshold.").unwrap();

        let chunker = MediaAwareChunker::default();

        // Test single file
        let txt_chunks = chunker.chunk_file(&txt_path).unwrap();
        assert!(!txt_chunks.is_empty());
        assert_eq!(txt_chunks[0].media_type, MediaType::Text);

        let md_chunks = chunker.chunk_file(&md_path).unwrap();
        assert!(!md_chunks.is_empty());
        assert_eq!(md_chunks[0].media_type, MediaType::Markdown);

        // Test directory
        let all_chunks = chunker.chunk_directory(temp_dir.path(), None).unwrap();
        assert!(all_chunks.len() >= 2);
    }

    #[test]
    fn test_chunk_id_generation() {
        let chunk = Chunk::new(
            PathBuf::from("/path/to/file.txt"),
            0,
            5,
            "content".to_string(),
            0,
            7,
            MediaType::Text,
        );

        assert!(chunk.id.contains("file.txt"));
        assert!(chunk.id.contains(":0"));
    }

    #[test]
    fn test_pdf_chunker_with_preextracted_text() {
        let chunker = PdfChunker;
        let config = ChunkConfig {
            chunk_size: 200,
            min_chunk_size: 10,
            ..Default::default()
        };

        // Simulate pre-extracted PDF text
        let content = "This is page one of a forensic medical report. \
            The patient presented with multiple contusions on the left forearm. \
            Photographs were taken and documented in the case file. \
            Page two continues with the detailed examination findings.";
        let path = Path::new("medical_report.pdf");

        let chunks = chunker.chunk(path, content, &config).unwrap();
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].media_type, MediaType::Pdf);
        assert!(chunks[0].content.contains("forensic"));
    }

    #[test]
    fn test_pdf_chunker_fallback_on_missing_file() {
        let chunker = PdfChunker;
        let config = ChunkConfig::default();
        let path = Path::new("nonexistent.pdf");

        // Should not panic — creates a fallback metadata chunk
        let chunks = chunker.chunk(path, "", &config).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].media_type, MediaType::Pdf);
        assert!(chunks[0].metadata.contains_key("extraction_error"));
    }
}
