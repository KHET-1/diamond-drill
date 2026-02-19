//! File signature database for carving.
//!
//! Each signature defines header magic bytes, optional footer, max file size,
//! and an optional internal size parser that reads the file's own length fields
//! for precise extraction without needing a footer scan.

use crate::core::FileType;

/// A file format signature for carving
#[derive(Debug, Clone)]
pub struct FileSignature {
    pub name: &'static str,
    pub extension: &'static str,
    pub file_type: FileType,
    /// Magic bytes at the start of the file
    pub header: &'static [u8],
    /// Offset from start where header appears (usually 0)
    pub header_offset: usize,
    /// Optional footer bytes marking end of file
    pub footer: Option<&'static [u8]>,
    /// Maximum expected file size (caps extraction to avoid runaway reads)
    pub max_size: u64,
    /// If set, a function that reads the data starting at the header and returns
    /// the total file length. This avoids expensive footer scans.
    pub size_parser: Option<fn(&[u8]) -> Option<u64>>,
}

/// Parse JPEG: scan for FFD9 footer (JPEG has no internal length for the full file)
pub(crate) fn parse_jpeg_size(_data: &[u8]) -> Option<u64> {
    None // JPEG requires footer scan
}

/// Parse PNG: read chunks until IEND
pub(crate) fn parse_png_size(data: &[u8]) -> Option<u64> {
    if data.len() < 8 {
        return None;
    }
    let mut pos = 8; // skip 8-byte PNG header
    loop {
        if pos + 12 > data.len() {
            return None;
        }
        let chunk_len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let chunk_type = &data[pos + 4..pos + 8];
        // 4 len + 4 type + data + 4 crc
        let total_chunk = 4 + 4 + chunk_len + 4;
        if chunk_type == b"IEND" {
            return Some((pos + total_chunk) as u64);
        }
        pos += total_chunk;
        if pos > data.len() || pos > 100_000_000 {
            return None;
        }
    }
}

/// Parse GIF: scan for trailer byte 0x3B
pub(crate) fn parse_gif_size(_data: &[u8]) -> Option<u64> {
    None // use footer scan
}

/// Parse PDF: %PDF header, scan for %%EOF footer
pub(crate) fn parse_pdf_size(_data: &[u8]) -> Option<u64> {
    None // use footer scan
}

/// Parse ZIP: find end-of-central-directory record
pub(crate) fn parse_zip_size(data: &[u8]) -> Option<u64> {
    // EOCD signature: 50 4B 05 06, search backward from end
    let search_len = data.len().min(65_536 + 22);
    let start = if data.len() > search_len { data.len() - search_len } else { 0 };
    for i in (start..data.len().saturating_sub(21)).rev() {
        if data[i..].starts_with(&[0x50, 0x4B, 0x05, 0x06]) {
            if i + 22 <= data.len() {
                let comment_len = u16::from_le_bytes([data[i + 20], data[i + 21]]) as usize;
                return Some((i + 22 + comment_len) as u64);
            }
        }
    }
    None
}

/// Parse BMP: size at bytes 2-5 (little-endian u32)
pub(crate) fn parse_bmp_size(data: &[u8]) -> Option<u64> {
    if data.len() < 6 {
        return None;
    }
    let size = u32::from_le_bytes([data[2], data[3], data[4], data[5]]);
    if size > 14 && (size as u64) < 200_000_000 {
        Some(size as u64)
    } else {
        None
    }
}

/// Parse WAV/RIFF: size at bytes 4-7 + 8
pub(crate) fn parse_riff_size(data: &[u8]) -> Option<u64> {
    if data.len() < 8 {
        return None;
    }
    let size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as u64;
    let total = size + 8; // RIFF header is 8 bytes before the declared size
    if total > 12 && total < 2_000_000_000 {
        Some(total)
    } else {
        None
    }
}

/// Parse MP4/MOV: walk ftyp/moov/mdat boxes
pub(crate) fn parse_mp4_size(data: &[u8]) -> Option<u64> {
    let mut pos = 0u64;
    let len = data.len() as u64;
    loop {
        if pos + 8 > len {
            break;
        }
        let p = pos as usize;
        let box_size = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as u64;
        if box_size == 0 {
            return Some(len); // box extends to end
        }
        if box_size < 8 {
            break;
        }
        pos += box_size;
        if pos >= len {
            return Some(pos);
        }
    }
    if pos > 8 { Some(pos) } else { None }
}

/// Parse FLAC: 4-byte magic + walk metadata blocks to find total
pub(crate) fn parse_flac_size(_data: &[u8]) -> Option<u64> {
    None // FLAC has no simple total-size field, use max_size cap
}

/// All known signatures, ordered by frequency for faster matching
pub fn all_signatures() -> Vec<FileSignature> {
    vec![
        // === Images ===
        FileSignature {
            name: "JPEG",
            extension: "jpg",
            file_type: FileType::Image,
            header: &[0xFF, 0xD8, 0xFF],
            header_offset: 0,
            footer: Some(&[0xFF, 0xD9]),
            max_size: 50 * 1024 * 1024, // 50 MB
            size_parser: Some(parse_jpeg_size),
        },
        FileSignature {
            name: "PNG",
            extension: "png",
            file_type: FileType::Image,
            header: &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            header_offset: 0,
            footer: Some(&[0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]),
            max_size: 100 * 1024 * 1024,
            size_parser: Some(parse_png_size),
        },
        FileSignature {
            name: "GIF87a",
            extension: "gif",
            file_type: FileType::Image,
            header: b"GIF87a",
            header_offset: 0,
            footer: Some(&[0x00, 0x3B]),
            max_size: 50 * 1024 * 1024,
            size_parser: Some(parse_gif_size),
        },
        FileSignature {
            name: "GIF89a",
            extension: "gif",
            file_type: FileType::Image,
            header: b"GIF89a",
            header_offset: 0,
            footer: Some(&[0x00, 0x3B]),
            max_size: 50 * 1024 * 1024,
            size_parser: Some(parse_gif_size),
        },
        FileSignature {
            name: "BMP",
            extension: "bmp",
            file_type: FileType::Image,
            header: &[0x42, 0x4D],
            header_offset: 0,
            footer: None,
            max_size: 200 * 1024 * 1024,
            size_parser: Some(parse_bmp_size),
        },
        FileSignature {
            name: "TIFF-LE",
            extension: "tiff",
            file_type: FileType::Image,
            header: &[0x49, 0x49, 0x2A, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "TIFF-BE",
            extension: "tiff",
            file_type: FileType::Image,
            header: &[0x4D, 0x4D, 0x00, 0x2A],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "WebP",
            extension: "webp",
            file_type: FileType::Image,
            header: b"RIFF",
            header_offset: 0,
            // Discriminated by bytes 8-11 = "WEBP"
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: Some(parse_riff_size),
        },
        FileSignature {
            name: "HEIF/HEIC",
            extension: "heic",
            file_type: FileType::Image,
            // ftyp box with "heic" or "heix" brand; header_offset=4 catches the ftyp marker
            header: b"ftyp",
            header_offset: 4,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },

        // === Video ===
        FileSignature {
            name: "MP4",
            extension: "mp4",
            file_type: FileType::Video,
            // ftyp box: offset 4 = "ftyp"
            header: b"ftyp",
            header_offset: 4,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024, // 4 GB
            size_parser: None, // handled by mp4 box walker at carver level
        },
        FileSignature {
            name: "AVI",
            extension: "avi",
            file_type: FileType::Video,
            header: b"RIFF",
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: Some(parse_riff_size),
        },
        FileSignature {
            name: "MKV",
            extension: "mkv",
            file_type: FileType::Video,
            header: &[0x1A, 0x45, 0xDF, 0xA3],
            header_offset: 0,
            footer: None,
            max_size: 8u64 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "FLV",
            extension: "flv",
            file_type: FileType::Video,
            header: b"FLV\x01",
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },

        // === Audio ===
        FileSignature {
            name: "MP3-ID3",
            extension: "mp3",
            file_type: FileType::Audio,
            header: b"ID3",
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "MP3-Sync",
            extension: "mp3",
            file_type: FileType::Audio,
            header: &[0xFF, 0xFB],
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "WAV",
            extension: "wav",
            file_type: FileType::Audio,
            header: b"RIFF",
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: Some(parse_riff_size),
        },
        FileSignature {
            name: "FLAC",
            extension: "flac",
            file_type: FileType::Audio,
            header: b"fLaC",
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: Some(parse_flac_size),
        },
        FileSignature {
            name: "OGG",
            extension: "ogg",
            file_type: FileType::Audio,
            header: b"OggS",
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "M4A/AAC",
            extension: "m4a",
            file_type: FileType::Audio,
            header: b"ftyp",
            header_offset: 4,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },

        // === Documents ===
        FileSignature {
            name: "PDF",
            extension: "pdf",
            file_type: FileType::Document,
            header: b"%PDF",
            header_offset: 0,
            footer: Some(b"%%EOF"),
            max_size: 500 * 1024 * 1024,
            size_parser: Some(parse_pdf_size),
        },
        FileSignature {
            name: "DOCX/XLSX/PPTX",
            extension: "docx",
            file_type: FileType::Document,
            header: &[0x50, 0x4B, 0x03, 0x04],
            header_offset: 0,
            footer: None,
            max_size: 200 * 1024 * 1024,
            size_parser: Some(parse_zip_size),
        },
        FileSignature {
            name: "DOC/XLS/PPT",
            extension: "doc",
            file_type: FileType::Document,
            header: &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
            header_offset: 0,
            footer: None,
            max_size: 200 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "RTF",
            extension: "rtf",
            file_type: FileType::Document,
            header: b"{\\rtf",
            header_offset: 0,
            footer: Some(b"}"),
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },

        // === Archives ===
        FileSignature {
            name: "ZIP",
            extension: "zip",
            file_type: FileType::Archive,
            header: &[0x50, 0x4B, 0x03, 0x04],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: Some(parse_zip_size),
        },
        FileSignature {
            name: "RAR5",
            extension: "rar",
            file_type: FileType::Archive,
            header: &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "RAR4",
            extension: "rar",
            file_type: FileType::Archive,
            header: &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "7z",
            extension: "7z",
            file_type: FileType::Archive,
            header: &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "GZIP",
            extension: "gz",
            file_type: FileType::Archive,
            header: &[0x1F, 0x8B, 0x08],
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "XZ",
            extension: "xz",
            file_type: FileType::Archive,
            header: &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "BZIP2",
            extension: "bz2",
            file_type: FileType::Archive,
            header: &[0x42, 0x5A, 0x68],
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },

        // === Executables ===
        FileSignature {
            name: "ELF",
            extension: "elf",
            file_type: FileType::Executable,
            header: &[0x7F, 0x45, 0x4C, 0x46],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "PE/EXE",
            extension: "exe",
            file_type: FileType::Executable,
            header: &[0x4D, 0x5A],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },

        // === Database ===
        FileSignature {
            name: "SQLite",
            extension: "sqlite",
            file_type: FileType::Database,
            header: b"SQLite format 3\x00",
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },

        // ==================================================================
        // Extended signatures — camera raw, design, ebook, font, misc
        // ==================================================================

        // --- Camera RAW ---
        FileSignature {
            name: "Canon CR2",
            extension: "cr2",
            file_type: FileType::Image,
            header: &[0x49, 0x49, 0x2A, 0x00, 0x10, 0x00, 0x00, 0x00, 0x43, 0x52],
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "Nikon NEF",
            extension: "nef",
            file_type: FileType::Image,
            header: &[0x4D, 0x4D, 0x00, 0x2A],
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "Sony ARW",
            extension: "arw",
            file_type: FileType::Image,
            header: &[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "Adobe DNG",
            extension: "dng",
            file_type: FileType::Image,
            header: &[0x49, 0x49, 0x2A, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 200 * 1024 * 1024,
            size_parser: None,
        },

        // --- Design / Creative ---
        FileSignature {
            name: "Photoshop PSD",
            extension: "psd",
            file_type: FileType::Image,
            header: b"8BPS",
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "Adobe Illustrator / EPS",
            extension: "eps",
            file_type: FileType::Image,
            header: b"%!PS-Adobe",
            header_offset: 0,
            footer: Some(b"%%EOF"),
            max_size: 200 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "SVG",
            extension: "svg",
            file_type: FileType::Image,
            header: b"<?xml",
            header_offset: 0,
            footer: Some(b"</svg>"),
            max_size: 50 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "ICO",
            extension: "ico",
            file_type: FileType::Image,
            header: &[0x00, 0x00, 0x01, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 10 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "GIMP XCF",
            extension: "xcf",
            file_type: FileType::Image,
            header: b"gimp xcf",
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },

        // --- Video (extended) ---
        FileSignature {
            name: "WebM",
            extension: "webm",
            file_type: FileType::Video,
            header: &[0x1A, 0x45, 0xDF, 0xA3],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "MPEG-TS",
            extension: "ts",
            file_type: FileType::Video,
            header: &[0x47],
            header_offset: 0,
            footer: None,
            max_size: 8u64 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "MPEG-PS",
            extension: "mpg",
            file_type: FileType::Video,
            header: &[0x00, 0x00, 0x01, 0xBA],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "WMV/ASF",
            extension: "wmv",
            file_type: FileType::Video,
            header: &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },

        // --- Audio (extended) ---
        FileSignature {
            name: "AIFF",
            extension: "aiff",
            file_type: FileType::Audio,
            header: b"FORM",
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "MIDI",
            extension: "mid",
            file_type: FileType::Audio,
            header: b"MThd",
            header_offset: 0,
            footer: None,
            max_size: 10 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "WMA",
            extension: "wma",
            file_type: FileType::Audio,
            header: &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "Opus",
            extension: "opus",
            file_type: FileType::Audio,
            header: b"OggS",
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },

        // --- Documents (extended) ---
        FileSignature {
            name: "EPUB",
            extension: "epub",
            file_type: FileType::Document,
            header: &[0x50, 0x4B, 0x03, 0x04],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: Some(parse_zip_size),
        },
        FileSignature {
            name: "OpenDocument ODT",
            extension: "odt",
            file_type: FileType::Document,
            header: &[0x50, 0x4B, 0x03, 0x04],
            header_offset: 0,
            footer: None,
            max_size: 200 * 1024 * 1024,
            size_parser: Some(parse_zip_size),
        },
        FileSignature {
            name: "XML",
            extension: "xml",
            file_type: FileType::Document,
            header: b"<?xml",
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "HTML",
            extension: "html",
            file_type: FileType::Code,
            header: b"<!DOCTYPE html",
            header_offset: 0,
            footer: Some(b"</html>"),
            max_size: 50 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "HTML-lower",
            extension: "html",
            file_type: FileType::Code,
            header: b"<html",
            header_offset: 0,
            footer: Some(b"</html>"),
            max_size: 50 * 1024 * 1024,
            size_parser: None,
        },

        // --- Archives (extended) ---
        FileSignature {
            name: "TAR",
            extension: "tar",
            file_type: FileType::Archive,
            header: b"ustar",
            header_offset: 257,
            footer: None,
            max_size: 8u64 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "ISO 9660",
            extension: "iso",
            file_type: FileType::Archive,
            header: b"CD001",
            header_offset: 32769,
            footer: None,
            max_size: 8u64 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "ZSTD",
            extension: "zst",
            file_type: FileType::Archive,
            header: &[0x28, 0xB5, 0x2F, 0xFD],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "LZ4",
            extension: "lz4",
            file_type: FileType::Archive,
            header: &[0x04, 0x22, 0x4D, 0x18],
            header_offset: 0,
            footer: None,
            max_size: 4 * 1024 * 1024 * 1024,
            size_parser: None,
        },

        // --- Executables / system (extended) ---
        FileSignature {
            name: "Mach-O 64",
            extension: "macho",
            file_type: FileType::Executable,
            header: &[0xFE, 0xED, 0xFA, 0xCF],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "Mach-O 32",
            extension: "macho",
            file_type: FileType::Executable,
            header: &[0xFE, 0xED, 0xFA, 0xCE],
            header_offset: 0,
            footer: None,
            max_size: 500 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "Java Class",
            extension: "class",
            file_type: FileType::Executable,
            header: &[0xCA, 0xFE, 0xBA, 0xBE],
            header_offset: 0,
            footer: None,
            max_size: 50 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "DEX (Android)",
            extension: "dex",
            file_type: FileType::Executable,
            header: b"dex\n",
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "WASM",
            extension: "wasm",
            file_type: FileType::Executable,
            header: &[0x00, 0x61, 0x73, 0x6D],
            header_offset: 0,
            footer: None,
            max_size: 100 * 1024 * 1024,
            size_parser: None,
        },

        // --- Fonts ---
        FileSignature {
            name: "TrueType Font",
            extension: "ttf",
            file_type: FileType::Other,
            header: &[0x00, 0x01, 0x00, 0x00, 0x00],
            header_offset: 0,
            footer: None,
            max_size: 50 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "OpenType/WOFF2",
            extension: "woff2",
            file_type: FileType::Other,
            header: b"wOF2",
            header_offset: 0,
            footer: None,
            max_size: 50 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "WOFF",
            extension: "woff",
            file_type: FileType::Other,
            header: b"wOFF",
            header_offset: 0,
            footer: None,
            max_size: 50 * 1024 * 1024,
            size_parser: None,
        },

        // --- Crypto / certs ---
        FileSignature {
            name: "PEM Certificate",
            extension: "pem",
            file_type: FileType::Other,
            header: b"-----BEGIN",
            header_offset: 0,
            footer: Some(b"-----END"),
            max_size: 10 * 1024 * 1024,
            size_parser: None,
        },

        // --- Misc ---
        FileSignature {
            name: "PCAP",
            extension: "pcap",
            file_type: FileType::Other,
            header: &[0xD4, 0xC3, 0xB2, 0xA1],
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },
        FileSignature {
            name: "PCAPNG",
            extension: "pcapng",
            file_type: FileType::Other,
            header: &[0x0A, 0x0D, 0x0D, 0x0A],
            header_offset: 0,
            footer: None,
            max_size: 2 * 1024 * 1024 * 1024,
            size_parser: None,
        },
    ]
}

/// RIFF sub-type discriminator: checks bytes 8-11 to distinguish WAV/AVI/WEBP
pub fn discriminate_riff(data: &[u8]) -> Option<&'static str> {
    if data.len() < 12 {
        return None;
    }
    match &data[8..12] {
        b"WAVE" => Some("wav"),
        b"AVI " => Some("avi"),
        b"WEBP" => Some("webp"),
        _ => Some("riff"),
    }
}

/// MP4/M4A discriminator: check ftyp brand
pub fn discriminate_ftyp(data: &[u8]) -> Option<&'static str> {
    if data.len() < 12 {
        return None;
    }
    let brand = &data[8..12];
    match brand {
        b"M4A " | b"M4B " => Some("m4a"),
        b"mp41" | b"mp42" | b"isom" | b"MSNV" | b"avc1" | b"dash" => Some("mp4"),
        b"qt  " => Some("mov"),
        b"3gp4" | b"3gp5" | b"3gp6" => Some("3gp"),
        _ => Some("mp4"),
    }
}

/// Build a fast lookup: for each possible first byte, which signatures start with it
pub fn build_first_byte_index(sigs: &[FileSignature]) -> [Vec<usize>; 256] {
    let mut index: [Vec<usize>; 256] = std::array::from_fn(|_| Vec::new());
    for (i, sig) in sigs.iter().enumerate() {
        if sig.header_offset == 0 {
            index[sig.header[0] as usize].push(i);
        }
    }
    index
}

/// Build index for signatures with non-zero header_offset (e.g. MP4 ftyp at offset 4)
pub fn build_offset_signatures(sigs: &[FileSignature]) -> Vec<(usize, usize)> {
    sigs.iter()
        .enumerate()
        .filter(|(_, s)| s.header_offset > 0)
        .map(|(i, s)| (i, s.header_offset))
        .collect()
}
