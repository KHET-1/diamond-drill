//! File carving module - Recover files from raw disk images by signature.
//!
//! Scans raw disk images (dd, img, iso, block devices) byte-by-byte using
//! memory-mapped I/O and parallel chunk processing to find file headers,
//! determine file boundaries, and extract intact files.
//!
//! # Design
//!
//! - **mmap**: Zero-copy access to multi-GB images via `memmap2`
//! - **Parallel chunks**: Image split into N chunks (one per CPU core),
//!   each scanned independently with rayon, overlapping by `max_header_size`
//!   to catch headers that straddle chunk boundaries
//! - **Signature dispatch**: First-byte index for O(1) candidate lookup,
//!   then full header match
//! - **Smart sizing**: Per-format size parsers read internal length fields
//!   (PNG chunks, RIFF sizes, BMP headers, ZIP EOCD) before falling back
//!   to footer scanning
//! - **Sector alignment**: Optional 512-byte alignment for true disk images

pub mod signatures;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Utc;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::{FileEntry, FileType};
use signatures::*;

/// A carved file found in a raw image
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarvedFile {
    /// Byte offset in the source image where this file starts
    pub offset: u64,
    /// Extracted file size in bytes
    pub size: u64,
    /// Signature that matched
    pub signature_name: String,
    /// Determined extension
    pub extension: String,
    /// File type category
    pub file_type: FileType,
    /// How the file end was determined
    pub boundary_method: BoundaryMethod,
    /// Blake3 hash of extracted content
    pub hash: Option<String>,
}

/// How the end of a carved file was determined
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum BoundaryMethod {
    /// Used the format's internal size fields (most accurate)
    InternalSize,
    /// Found the format's footer/trailer bytes
    FooterScan,
    /// Hit another file header (next-header boundary)
    NextHeader,
    /// Hit the max_size cap
    MaxSizeCap,
}

/// Options for a carve operation
#[derive(Debug, Clone)]
pub struct CarveOptions {
    /// Source raw image path
    pub source: PathBuf,
    /// Output directory for carved files
    pub output_dir: PathBuf,
    /// Align scanning to 512-byte sector boundaries
    pub sector_aligned: bool,
    /// Minimum file size to extract (skip tiny fragments)
    pub min_size: u64,
    /// Only carve these file types (None = all)
    pub file_types: Option<Vec<FileType>>,
    /// Number of parallel workers
    pub workers: usize,
    /// Don't write files, just scan and report
    pub dry_run: bool,
    /// Verify extracted files with infer crate
    pub verify: bool,
}

impl Default for CarveOptions {
    fn default() -> Self {
        Self {
            source: PathBuf::new(),
            output_dir: PathBuf::from("carved"),
            sector_aligned: true,
            min_size: 512,
            file_types: None,
            workers: num_cpus::get(),
            dry_run: false,
            verify: true,
        }
    }
}

/// Result of a carve operation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CarveResult {
    pub files_found: usize,
    pub files_extracted: usize,
    pub files_verified: usize,
    pub files_failed: usize,
    pub total_bytes_extracted: u64,
    pub image_size: u64,
    pub duration_ms: u64,
    pub by_type: std::collections::HashMap<String, usize>,
}

/// Progress updates emitted during carving
#[derive(Debug, Clone)]
pub enum CarveProgress {
    /// Scanning phase: bytes_scanned out of total
    Scanning { bytes_scanned: u64, total_bytes: u64 },
    /// Scan complete, N headers found
    ScanComplete { headers_found: usize },
    /// Extracting file i of total
    Extracting { current: usize, total: usize, extension: String },
    /// Done
    Done,
}

/// The file carver engine
pub struct Carver {
    options: CarveOptions,
    signatures: Vec<FileSignature>,
    first_byte_index: [Vec<usize>; 256],
    offset_sigs: Vec<(usize, usize)>,
}

impl Carver {
    pub fn new(options: CarveOptions) -> Self {
        let mut sigs = all_signatures();

        if let Some(ref types) = options.file_types {
            sigs.retain(|s| types.contains(&s.file_type));
        }

        let first_byte_index = build_first_byte_index(&sigs);
        let offset_sigs = build_offset_signatures(&sigs);

        Self {
            options,
            signatures: sigs,
            first_byte_index,
            offset_sigs,
        }
    }

    /// Carve with a progress callback. The callback is called from the
    /// extraction (sequential) phase and after the scan phase completes.
    pub async fn carve_with_progress<F>(
        &self,
        on_progress: F,
    ) -> Result<(Vec<CarvedFile>, CarveResult)>
    where
        F: Fn(CarveProgress) + Send + Sync,
    {
        let start = Instant::now();
        let source = &self.options.source;

        anyhow::ensure!(source.exists(), "Source image not found: {}", source.display());

        let file = std::fs::File::open(source)
            .with_context(|| format!("Failed to open image: {}", source.display()))?;
        let metadata = file.metadata()?;
        let image_size = metadata.len();

        anyhow::ensure!(image_size > 0, "Image file is empty");

        tracing::info!(
            source = %source.display(),
            image_size,
            signatures = self.signatures.len(),
            workers = self.options.workers,
            sector_aligned = self.options.sector_aligned,
            min_size = self.options.min_size,
            dry_run = self.options.dry_run,
            "Starting file carve"
        );

        let mmap = Arc::new(unsafe {
            memmap2::Mmap::map(&file)
                .with_context(|| format!("Failed to mmap image: {}", source.display()))?
        });

        if !self.options.dry_run {
            std::fs::create_dir_all(&self.options.output_dir)?;
        }

        let num_chunks = self.options.workers.max(1);
        let chunk_size = (image_size as usize) / num_chunks;
        let max_header_len = self.signatures.iter().map(|s| s.header.len() + s.header_offset).max().unwrap_or(16);
        let overlap = max_header_len.max(512);

        tracing::debug!(num_chunks, chunk_size, overlap, "Scan chunking configured");

        let scan_progress = Arc::new(AtomicU64::new(0));

        let sp = Arc::clone(&scan_progress);
        let all_hits: Vec<Vec<(u64, usize)>> = (0..num_chunks)
            .into_par_iter()
            .map(|chunk_idx| {
                let chunk_start = chunk_idx * chunk_size;
                let chunk_end = if chunk_idx == num_chunks - 1 {
                    image_size as usize
                } else {
                    ((chunk_idx + 1) * chunk_size) + overlap
                };
                let chunk_end = chunk_end.min(image_size as usize);

                let hits = self.scan_chunk(&mmap, chunk_start, chunk_end);
                sp.fetch_add((chunk_end - chunk_start) as u64, Ordering::Relaxed);
                hits
            })
            .collect();

        let mut hits: Vec<(u64, usize)> = Vec::new();
        for chunk_hits in all_hits {
            hits.extend(chunk_hits);
        }
        hits.sort_by_key(|&(offset, _)| offset);
        hits.dedup_by_key(|h| h.0);

        tracing::info!(
            headers_found = hits.len(),
            scan_ms = start.elapsed().as_millis() as u64,
            "Signature scan complete"
        );

        on_progress(CarveProgress::ScanComplete { headers_found: hits.len() });

        // Phase 2: determine boundaries
        let carved: Vec<CarvedFile> = hits
            .par_iter()
            .enumerate()
            .filter_map(|(i, &(offset, sig_idx))| {
                let sig = &self.signatures[sig_idx];
                let next_offset = hits.get(i + 1).map(|&(o, _)| o);

                match self.determine_size(&mmap, offset, sig, next_offset) {
                    Some(size) if size >= self.options.min_size => {
                        let mut carved = CarvedFile {
                            offset,
                            size,
                            signature_name: sig.name.to_string(),
                            extension: self.resolve_extension(&mmap, offset, sig),
                            file_type: sig.file_type,
                            boundary_method: BoundaryMethod::MaxSizeCap,
                            hash: None,
                        };

                        carved.boundary_method = self.classify_boundary(
                            &mmap, offset, size, sig, next_offset,
                        );

                        Some(carved)
                    }
                    _ => None,
                }
            })
            .collect();

        // Phase 3: extract to disk with progress
        let total_to_extract = carved.len();
        let mut result = CarveResult {
            files_found: total_to_extract,
            image_size,
            ..Default::default()
        };

        let mut final_carved = Vec::with_capacity(total_to_extract);

        for (i, mut cf) in carved.into_iter().enumerate() {
            on_progress(CarveProgress::Extracting {
                current: i + 1,
                total: total_to_extract,
                extension: cf.extension.clone(),
            });

            let end = (cf.offset + cf.size) as usize;
            if end > mmap.len() {
                result.files_failed += 1;
                continue;
            }

            let data = &mmap[cf.offset as usize..end];

            if self.options.verify {
                if let Some(kind) = infer::get(data) {
                    cf.extension = kind.extension().to_string();
                    result.files_verified += 1;
                }
            }

            let hash = blake3::hash(data);
            cf.hash = Some(hex::encode(hash.as_bytes()));

            if !self.options.dry_run {
                let filename = format!(
                    "{:08}_{:012x}.{}",
                    i, cf.offset, cf.extension
                );
                let out_path = self.options.output_dir.join(&filename);
                if let Err(e) = std::fs::write(&out_path, data) {
                    tracing::warn!(
                        path = %out_path.display(),
                        error = %e,
                        offset = cf.offset,
                        size = cf.size,
                        "Failed to write carved file"
                    );
                    result.files_failed += 1;
                    continue;
                }
                result.files_extracted += 1;
            } else {
                result.files_extracted += 1;
            }

            *result.by_type.entry(cf.extension.clone()).or_insert(0) += 1;
            result.total_bytes_extracted += cf.size;
            final_carved.push(cf);
        }

        on_progress(CarveProgress::Done);
        result.duration_ms = start.elapsed().as_millis() as u64;

        tracing::info!(
            files_found = result.files_found,
            files_extracted = result.files_extracted,
            files_verified = result.files_verified,
            files_failed = result.files_failed,
            total_bytes = result.total_bytes_extracted,
            duration_ms = result.duration_ms,
            "Carve complete"
        );

        Ok((final_carved, result))
    }

    /// Convenience wrapper without progress (for tests and non-interactive use)
    pub async fn carve(&self) -> Result<(Vec<CarvedFile>, CarveResult)> {
        self.carve_with_progress(|_| {}).await
    }

    /// Scan a chunk of the mmap for file headers. Returns (offset, signature_index) pairs.
    ///
    /// When sector_aligned=true, the main loop steps by 512 bytes for offset-0
    /// signatures. For offset-based signatures (ftyp at +4, ustar at +257,
    /// CD001 at +32769), we probe at `pos + header_offset` from each sector
    /// boundary so files starting at sector boundaries are always found.
    fn scan_chunk(&self, data: &[u8], start: usize, end: usize) -> Vec<(u64, usize)> {
        let mut hits = Vec::new();
        let step = if self.options.sector_aligned { 512 } else { 1 };
        let end = end.min(data.len());

        let mut pos = start;
        if self.options.sector_aligned {
            pos = (pos + 511) & !511;
        }

        while pos < end {
            // Fast path: first-byte index lookup for signatures at offset 0
            let byte = data[pos];
            for &sig_idx in &self.first_byte_index[byte as usize] {
                let sig = &self.signatures[sig_idx];
                let header_end = pos + sig.header.len();
                if header_end <= data.len() && data[pos..header_end] == *sig.header {
                    hits.push((pos as u64, sig_idx));
                    break;
                }
            }

            // For offset-based signatures, probe at pos + header_offset.
            // This means: "if a file starts at `pos`, check whether its magic
            // bytes at `pos + hdr_off` match." This correctly finds MP4 (ftyp
            // at +4), TAR (ustar at +257), and ISO (CD001 at +32769) when the
            // file starts at a sector boundary.
            for &(sig_idx, hdr_off) in &self.offset_sigs {
                let probe = pos + hdr_off;
                if probe + self.signatures[sig_idx].header.len() > data.len() {
                    continue;
                }
                let sig = &self.signatures[sig_idx];
                if data[probe..probe + sig.header.len()] == *sig.header {
                    let file_start = pos as u64;
                    if !hits.iter().any(|&(o, _)| o == file_start) {
                        hits.push((file_start, sig_idx));
                    }
                }
            }

            pos += step;
        }

        hits
    }

    /// Determine the size of a carved file using (in order):
    /// 1. Internal size parser
    /// 2. Footer scan
    /// 3. Next-header boundary
    /// 4. max_size cap
    fn determine_size(
        &self,
        data: &[u8],
        offset: u64,
        sig: &FileSignature,
        next_header: Option<u64>,
    ) -> Option<u64> {
        let start = offset as usize;
        if start >= data.len() {
            return None;
        }
        let max_end = (start as u64 + sig.max_size).min(data.len() as u64) as usize;

        // 1. Internal size parser (most precise, uses format-specific fields)
        let slice_full = &data[start..max_end];
        if let Some(parser) = sig.size_parser {
            if let Some(size) = parser(slice_full) {
                if size >= self.options.min_size && (start + size as usize) <= data.len() {
                    return Some(size);
                }
            }
        }

        // 2. Footer scan -- but cap the search region to the next header
        // to avoid matching another file's footer bytes
        if let Some(footer) = sig.footer {
            let scan_limit = match next_header {
                Some(next) if next > offset => ((next - offset) as usize).min(max_end - start),
                _ => max_end - start,
            };
            let scan_slice = &data[start..start + scan_limit];
            if let Some(footer_pos) = find_footer(scan_slice, footer, self.options.min_size as usize) {
                let size = (footer_pos + footer.len()) as u64;
                return Some(size);
            }
        }

        // 3. Next-header boundary (clamped to max_size)
        if let Some(next) = next_header {
            if next > offset {
                let size = (next - offset).min(sig.max_size);
                if size >= self.options.min_size {
                    return Some(size);
                }
            }
        }

        None
    }

    fn classify_boundary(
        &self,
        data: &[u8],
        offset: u64,
        size: u64,
        sig: &FileSignature,
        next_header: Option<u64>,
    ) -> BoundaryMethod {
        let start = offset as usize;
        if start >= data.len() {
            return BoundaryMethod::MaxSizeCap;
        }
        let max_end = (start as u64 + sig.max_size).min(data.len() as u64) as usize;
        let slice_full = &data[start..max_end];

        if let Some(parser) = sig.size_parser {
            if let Some(parsed_size) = parser(slice_full) {
                if parsed_size == size {
                    return BoundaryMethod::InternalSize;
                }
            }
        }

        if let Some(footer) = sig.footer {
            let scan_limit = match next_header {
                Some(next) if next > offset => ((next - offset) as usize).min(max_end - start),
                _ => max_end - start,
            };
            let scan_slice = &data[start..start + scan_limit];
            if let Some(footer_pos) = find_footer(scan_slice, footer, self.options.min_size as usize) {
                if (footer_pos + footer.len()) as u64 == size {
                    return BoundaryMethod::FooterScan;
                }
            }
        }

        if let Some(next) = next_header {
            if next > offset && (next - offset).min(sig.max_size) == size {
                return BoundaryMethod::NextHeader;
            }
        }

        BoundaryMethod::MaxSizeCap
    }

    /// Resolve extension with sub-type discrimination (RIFF → wav/avi/webp, ftyp → mp4/m4a/mov)
    fn resolve_extension(&self, data: &[u8], offset: u64, sig: &FileSignature) -> String {
        let start = offset as usize;
        let avail = data.len().saturating_sub(start);
        let slice = &data[start..start + avail.min(64)];

        if sig.header == b"RIFF" {
            if let Some(ext) = discriminate_riff(slice) {
                return ext.to_string();
            }
        }

        if sig.header == b"ftyp" && sig.header_offset == 4 {
            if let Some(ext) = discriminate_ftyp(slice) {
                return ext.to_string();
            }
        }

        sig.extension.to_string()
    }

    /// Convert carved files into FileEntry objects for the main index.
    pub fn to_file_entries(&self, carved: &[CarvedFile], base_dir: &Path) -> Vec<FileEntry> {
        carved
            .iter()
            .enumerate()
            .map(|(i, cf)| {
                let filename = format!(
                    "{:08}_{:012x}.{}",
                    i, cf.offset, cf.extension
                );
                let path = base_dir.join(&filename);

                FileEntry {
                    path,
                    size: cf.size,
                    file_type: cf.file_type,
                    extension: cf.extension.clone(),
                    modified: None,
                    created: Some(Utc::now()),
                    hash: cf.hash.clone(),
                    has_bad_sectors: false,
                    thumbnail: None,
                }
            })
            .collect()
    }
}

/// Scan forward in `data` for `footer` bytes.
/// Search begins at `min_offset` (the footer can't appear before the file
/// has reached min_offset bytes, so there's no point scanning earlier).
fn find_footer(data: &[u8], footer: &[u8], min_offset: usize) -> Option<usize> {
    let flen = footer.len();
    if flen == 0 || data.len() < flen {
        return None;
    }

    // Start searching at min_offset (file must be at least this big)
    let search_start = min_offset;
    // Last valid start position for a footer match
    let search_end = data.len() - flen + 1;

    if search_start >= search_end {
        return None;
    }

    for i in search_start..search_end {
        if data[i] == footer[0] && data[i..i + flen] == *footer {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Helper: build a carver with specific options quickly ===
    fn carver_default() -> Carver {
        Carver::new(CarveOptions::default())
    }

    fn carver_byte_level() -> Carver {
        Carver::new(CarveOptions {
            sector_aligned: false,
            min_size: 10,
            ..Default::default()
        })
    }

    fn run_carve(opts: CarveOptions) -> (Vec<CarvedFile>, CarveResult) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { Carver::new(opts).carve().await.unwrap() })
    }

    fn write_img(dir: &std::path::Path, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    // =====================================================================
    // Scenario 1: Signature index construction
    // =====================================================================

    #[test]
    fn scenario_1_first_byte_index_covers_all_zero_offset_sigs() {
        let sigs = all_signatures();
        let idx = build_first_byte_index(&sigs);
        for (i, sig) in sigs.iter().enumerate() {
            if sig.header_offset == 0 {
                assert!(
                    idx[sig.header[0] as usize].contains(&i),
                    "Signature {} missing from first_byte_index at byte 0x{:02X}",
                    sig.name,
                    sig.header[0]
                );
            }
        }
    }

    #[test]
    fn scenario_1_offset_sigs_all_have_nonzero_offset() {
        let sigs = all_signatures();
        let off = build_offset_signatures(&sigs);
        for &(idx, offset) in &off {
            assert!(offset > 0, "Sig {} in offset_sigs with offset=0", sigs[idx].name);
            assert_eq!(sigs[idx].header_offset, offset);
        }
    }

    // =====================================================================
    // Scenario 2: Size parsers — every parser returns correct values
    // =====================================================================

    #[test]
    fn scenario_2_png_chunk_walk() {
        let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        // IHDR: 13 bytes
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&[0u8; 13]);
        data.extend_from_slice(&[0u8; 4]); // CRC
        // IEND: 0 bytes
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"IEND");
        data.extend_from_slice(&[0u8; 4]); // CRC

        assert_eq!(parse_png_size(&data), Some(data.len() as u64));
    }

    #[test]
    fn scenario_2_bmp_header_size() {
        let mut d = vec![0x42, 0x4D];
        d.extend_from_slice(&2048u32.to_le_bytes());
        d.resize(2048, 0);
        assert_eq!(parse_bmp_size(&d), Some(2048));
    }

    #[test]
    fn scenario_2_riff_wav_size() {
        let mut d = b"RIFF".to_vec();
        d.extend_from_slice(&500u32.to_le_bytes());
        d.extend_from_slice(b"WAVE");
        d.resize(508, 0);
        assert_eq!(parse_riff_size(&d), Some(508)); // 500 + 8
    }

    #[test]
    fn scenario_2_bmp_rejects_garbage_size() {
        let d = vec![0x42, 0x4D, 0, 0, 0, 0]; // size = 0
        assert_eq!(parse_bmp_size(&d), None);
    }

    #[test]
    fn scenario_2_riff_rejects_tiny() {
        let d = vec![0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0]; // size = 0 → total = 8
        assert_eq!(parse_riff_size(&d), None); // total must be > 12
    }

    #[test]
    fn scenario_2_png_truncated() {
        let data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // just header
        assert_eq!(parse_png_size(&data), None);
    }

    // =====================================================================
    // Scenario 3: find_footer edge cases
    // =====================================================================

    #[test]
    fn scenario_3_footer_at_exact_min_offset() {
        let mut data = vec![0u8; 600];
        data[512] = 0xFF;
        data[513] = 0xD9;
        assert_eq!(find_footer(&data, &[0xFF, 0xD9], 512), Some(512));
    }

    #[test]
    fn scenario_3_footer_before_min_offset_not_found() {
        let mut data = vec![0u8; 600];
        data[100] = 0xFF;
        data[101] = 0xD9;
        assert_eq!(find_footer(&data, &[0xFF, 0xD9], 512), None);
    }

    #[test]
    fn scenario_3_footer_at_last_byte() {
        let mut data = vec![0u8; 100];
        data[98] = 0xFF;
        data[99] = 0xD9;
        assert_eq!(find_footer(&data, &[0xFF, 0xD9], 0), Some(98));
    }

    #[test]
    fn scenario_3_single_byte_footer() {
        let mut data = vec![0u8; 100];
        data[50] = 0x3B; // GIF trailer
        assert_eq!(find_footer(&data, &[0x3B], 10), Some(50));
    }

    #[test]
    fn scenario_3_empty_data() {
        assert_eq!(find_footer(&[], &[0xFF, 0xD9], 0), None);
    }

    #[test]
    fn scenario_3_footer_longer_than_data() {
        assert_eq!(find_footer(&[0xFF], &[0xFF, 0xD9], 0), None);
    }

    // =====================================================================
    // Scenario 4: scan_chunk — sector-aligned and byte-level
    // =====================================================================

    #[test]
    fn scenario_4_sector_aligned_finds_at_boundary() {
        let c = carver_default();
        let mut data = vec![0u8; 2048];
        data[512] = 0xFF; data[513] = 0xD8; data[514] = 0xFF; // JPEG at sector 1
        let hits = c.scan_chunk(&data, 0, data.len());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 512);
    }

    #[test]
    fn scenario_4_sector_aligned_misses_unaligned() {
        let c = carver_default();
        let mut data = vec![0u8; 2048];
        data[100] = 0xFF; data[101] = 0xD8; data[102] = 0xFF; // JPEG at byte 100, not aligned
        let hits = c.scan_chunk(&data, 0, data.len());
        assert!(hits.is_empty(), "Sector-aligned scan should skip byte 100");
    }

    #[test]
    fn scenario_4_sector_aligned_finds_mp4_ftyp() {
        let c = carver_default();
        let mut data = vec![0u8; 2048];
        // MP4 file starting at sector 0: ftyp box at offset 4
        // box_size(4) + "ftyp"(4) + brand(4) = typical ftyp header
        data[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x1C]); // box size 28
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"isom");
        let hits = c.scan_chunk(&data, 0, data.len());
        let mp4_hit = hits.iter().find(|&&(off, _)| off == 0);
        assert!(mp4_hit.is_some(), "Should find MP4 at sector 0 via ftyp probe at byte 4");
    }

    #[test]
    fn scenario_4_sector_aligned_finds_tar() {
        let c = carver_default();
        let mut data = vec![0u8; 2048];
        // TAR file starting at sector 0: "ustar" at offset 257
        data[257..262].copy_from_slice(b"ustar");
        let hits = c.scan_chunk(&data, 0, data.len());
        let tar_hit = hits.iter().find(|&&(off, _)| off == 0);
        assert!(tar_hit.is_some(), "Should find TAR at sector 0 via ustar probe at byte 257");
    }

    #[test]
    fn scenario_4_byte_level_finds_unaligned() {
        let c = carver_byte_level();
        let mut data = vec![0u8; 2048];
        data[100] = 0xFF; data[101] = 0xD8; data[102] = 0xFF;
        let hits = c.scan_chunk(&data, 0, data.len());
        assert!(!hits.is_empty());
        assert_eq!(hits[0].0, 100);
    }

    #[test]
    fn scenario_4_multiple_signatures_same_image() {
        let c = carver_byte_level();
        let mut data = vec![0u8; 4096];
        // JPEG at 0
        data[0] = 0xFF; data[1] = 0xD8; data[2] = 0xFF;
        // PDF at 2048
        data[2048] = b'%'; data[2049] = b'P'; data[2050] = b'D'; data[2051] = b'F';
        let hits = c.scan_chunk(&data, 0, data.len());
        assert!(hits.len() >= 2, "Should find JPEG and PDF, found {}", hits.len());
    }

    #[test]
    fn scenario_4_empty_range() {
        let c = carver_default();
        let data = vec![0u8; 1024];
        let hits = c.scan_chunk(&data, 512, 512); // zero-length range
        assert!(hits.is_empty());
    }

    // =====================================================================
    // Scenario 5: RIFF/ftyp sub-type discrimination
    // =====================================================================

    #[test]
    fn scenario_5_riff_wav_avi_webp() {
        assert_eq!(discriminate_riff(b"RIFF\x00\x00\x00\x00WAVE"), Some("wav"));
        assert_eq!(discriminate_riff(b"RIFF\x00\x00\x00\x00AVI "), Some("avi"));
        assert_eq!(discriminate_riff(b"RIFF\x00\x00\x00\x00WEBP"), Some("webp"));
        assert_eq!(discriminate_riff(b"RIFF\x00\x00\x00\x00XXXX"), Some("riff"));
    }

    #[test]
    fn scenario_5_ftyp_brands() {
        assert_eq!(discriminate_ftyp(b"\x00\x00\x00\x1Cftypisom"), Some("mp4"));
        assert_eq!(discriminate_ftyp(b"\x00\x00\x00\x1CftypM4A "), Some("m4a"));
        assert_eq!(discriminate_ftyp(b"\x00\x00\x00\x1Cftypqt  "), Some("mov"));
        assert_eq!(discriminate_ftyp(b"\x00\x00\x00\x1Cftyp3gp5"), Some("3gp"));
    }

    #[test]
    fn scenario_5_short_data_returns_none() {
        assert_eq!(discriminate_riff(b"RIFF"), None);
        assert_eq!(discriminate_ftyp(b"ftyp"), None);
    }

    // =====================================================================
    // Scenario 6: Full carve — dry run with JPEG (footer scan path)
    // =====================================================================

    #[test]
    fn scenario_6_jpeg_footer_scan_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = vec![0u8; 4096];
        img[0] = 0xFF; img[1] = 0xD8; img[2] = 0xFF; img[3] = 0xE0;
        img[2000] = 0xFF; img[2001] = 0xD9;
        let path = write_img(dir.path(), "test.img", &img);

        let (carved, result) = run_carve(CarveOptions {
            source: path,
            output_dir: dir.path().join("out"),
            sector_aligned: false,
            min_size: 100,
            dry_run: true,
            verify: false,
            ..Default::default()
        });

        assert_eq!(result.files_found, 1);
        assert_eq!(result.files_extracted, 1);
        assert_eq!(carved[0].offset, 0);
        assert_eq!(carved[0].extension, "jpg");
        assert_eq!(carved[0].boundary_method, BoundaryMethod::FooterScan);
        assert!(carved[0].hash.is_some());
        assert_eq!(carved[0].size, 2002); // footer at 2000, + 2 bytes for FFD9
    }

    // =====================================================================
    // Scenario 7: Full carve — BMP with internal size parser path
    // =====================================================================

    #[test]
    fn scenario_7_bmp_internal_size() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = vec![0u8; 8192];
        // BMP at sector 0: BM header + size=1024
        img[0] = 0x42; img[1] = 0x4D;
        img[2..6].copy_from_slice(&1024u32.to_le_bytes());
        let path = write_img(dir.path(), "bmp.img", &img);

        let (carved, result) = run_carve(CarveOptions {
            source: path,
            output_dir: dir.path().join("out"),
            sector_aligned: false,
            min_size: 100,
            dry_run: true,
            verify: false,
            ..Default::default()
        });

        assert_eq!(result.files_found, 1);
        assert_eq!(carved[0].extension, "bmp");
        assert_eq!(carved[0].size, 1024);
        assert_eq!(carved[0].boundary_method, BoundaryMethod::InternalSize);
    }

    // =====================================================================
    // Scenario 8: Full carve — actual file extraction to disk
    // =====================================================================

    #[test]
    fn scenario_8_extract_to_disk_with_verify() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = vec![0u8; 4096];
        img[0] = 0xFF; img[1] = 0xD8; img[2] = 0xFF; img[3] = 0xE0;
        img[2000] = 0xFF; img[2001] = 0xD9;
        let path = write_img(dir.path(), "extract.img", &img);
        let out = dir.path().join("extracted");

        let (carved, result) = run_carve(CarveOptions {
            source: path,
            output_dir: out.clone(),
            sector_aligned: false,
            min_size: 100,
            dry_run: false,
            verify: true,
            ..Default::default()
        });

        assert_eq!(result.files_extracted, 1);
        assert!(out.exists());

        // Check output file exists and has correct size
        let entries: Vec<_> = std::fs::read_dir(&out).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x != "json").unwrap_or(false))
            .collect();
        assert_eq!(entries.len(), 1, "Should have 1 extracted file");

        let content = std::fs::read(entries[0].path()).unwrap();
        assert_eq!(content.len(), carved[0].size as usize);
        assert_eq!(content[0], 0xFF); // starts with JPEG header
        assert_eq!(content[1], 0xD8);
    }

    // =====================================================================
    // Scenario 9: Multiple files — next-header boundary path
    // =====================================================================

    #[test]
    fn scenario_9_two_jpegs_next_header_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = vec![0u8; 8192];
        // JPEG 1 at byte 0, no footer placed
        img[0] = 0xFF; img[1] = 0xD8; img[2] = 0xFF; img[3] = 0xE0;
        // JPEG 2 at byte 4096, with footer
        img[4096] = 0xFF; img[4097] = 0xD8; img[4098] = 0xFF; img[4099] = 0xE0;
        img[6000] = 0xFF; img[6001] = 0xD9;

        let path = write_img(dir.path(), "two.img", &img);

        let (carved, result) = run_carve(CarveOptions {
            source: path,
            output_dir: dir.path().join("out"),
            sector_aligned: false,
            min_size: 100,
            dry_run: true,
            verify: false,
            ..Default::default()
        });

        assert_eq!(result.files_found, 2);
        assert_eq!(carved[0].offset, 0);
        // First JPEG: no footer, so it uses next-header boundary
        assert_eq!(carved[0].boundary_method, BoundaryMethod::NextHeader);
        assert_eq!(carved[0].size, 4096);

        assert_eq!(carved[1].offset, 4096);
        assert_eq!(carved[1].boundary_method, BoundaryMethod::FooterScan);
    }

    // =====================================================================
    // Scenario 10: File type filter — only carve images
    // =====================================================================

    #[test]
    fn scenario_10_type_filter_images_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = vec![0u8; 8192];
        // JPEG at 0
        img[0] = 0xFF; img[1] = 0xD8; img[2] = 0xFF;
        img[2000] = 0xFF; img[2001] = 0xD9;
        // PDF at 4096
        img[4096] = b'%'; img[4097] = b'P'; img[4098] = b'D'; img[4099] = b'F';
        // %%EOF
        img[6000] = b'%'; img[6001] = b'%'; img[6002] = b'E';
        img[6003] = b'O'; img[6004] = b'F';

        let path = write_img(dir.path(), "mixed.img", &img);

        let (carved, result) = run_carve(CarveOptions {
            source: path,
            output_dir: dir.path().join("out"),
            sector_aligned: false,
            min_size: 100,
            dry_run: true,
            verify: false,
            file_types: Some(vec![FileType::Image]),
            ..Default::default()
        });

        // Should only find the JPEG, not the PDF
        assert_eq!(result.files_found, 1);
        assert_eq!(carved[0].extension, "jpg");
    }

    // =====================================================================
    // Scenario 11: Empty image — should return 0 files, no panic
    // =====================================================================

    #[test]
    fn scenario_11_empty_image_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_img(dir.path(), "empty.img", &[]);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            Carver::new(CarveOptions {
                source: path,
                output_dir: dir.path().join("out"),
                ..Default::default()
            })
            .carve()
            .await
        });

        assert!(result.is_err(), "Empty image should return error");
    }

    // =====================================================================
    // Scenario 12: Nonexistent image — should error cleanly
    // =====================================================================

    #[test]
    fn scenario_12_missing_image_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            Carver::new(CarveOptions {
                source: PathBuf::from("/nonexistent/image.dd"),
                output_dir: PathBuf::from("/tmp/out"),
                ..Default::default()
            })
            .carve()
            .await
        });

        assert!(result.is_err());
    }

    // =====================================================================
    // Scenario 13: File below min_size threshold — skipped
    // =====================================================================

    #[test]
    fn scenario_13_below_min_size_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut img = vec![0u8; 2048];
        // Tiny JPEG: header + footer only 10 bytes apart
        img[0] = 0xFF; img[1] = 0xD8; img[2] = 0xFF;
        img[10] = 0xFF; img[11] = 0xD9;
        let path = write_img(dir.path(), "tiny.img", &img);

        let (carved, _) = run_carve(CarveOptions {
            source: path,
            output_dir: dir.path().join("out"),
            sector_aligned: false,
            min_size: 100, // file is only 12 bytes
            dry_run: true,
            verify: false,
            ..Default::default()
        });

        assert!(carved.is_empty(), "File below min_size should be skipped");
    }

    // =====================================================================
    // Scenario 14: to_file_entries conversion
    // =====================================================================

    #[test]
    fn scenario_14_carved_to_file_entries() {
        let carved = vec![
            CarvedFile {
                offset: 0,
                size: 2002,
                signature_name: "JPEG".to_string(),
                extension: "jpg".to_string(),
                file_type: FileType::Image,
                boundary_method: BoundaryMethod::FooterScan,
                hash: Some("abc123".to_string()),
            },
            CarvedFile {
                offset: 4096,
                size: 1024,
                signature_name: "BMP".to_string(),
                extension: "bmp".to_string(),
                file_type: FileType::Image,
                boundary_method: BoundaryMethod::InternalSize,
                hash: Some("def456".to_string()),
            },
        ];

        let carver = carver_default();
        let entries = carver.to_file_entries(&carved, std::path::Path::new("/out"));

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].size, 2002);
        assert_eq!(entries[0].extension, "jpg");
        assert_eq!(entries[0].file_type, FileType::Image);
        assert_eq!(entries[0].hash.as_deref(), Some("abc123"));
        assert!(entries[0].path.to_string_lossy().contains("00000000_"));
        assert!(entries[1].path.to_string_lossy().contains("00000001_"));
    }

    // =====================================================================
    // Scenario 15: Zeroed image — no false positives
    // =====================================================================

    #[test]
    fn scenario_15_zeroed_image_no_false_positives() {
        let dir = tempfile::tempdir().unwrap();
        let img = vec![0u8; 65536]; // 64KB of zeros
        let path = write_img(dir.path(), "zeros.img", &img);

        let (carved, _) = run_carve(CarveOptions {
            source: path,
            output_dir: dir.path().join("out"),
            sector_aligned: true,
            min_size: 512,
            dry_run: true,
            verify: false,
            ..Default::default()
        });

        assert!(carved.is_empty(), "Zeroed image should produce no carved files");
    }
}
