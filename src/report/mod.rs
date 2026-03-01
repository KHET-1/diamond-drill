//! HTML & PDF Recovery Report Generator for Diamond Drill
//!
//! Produces self-contained, visually stunning recovery reports with:
//! - Dark glassmorphic theme (deep navy, sapphire accents, frosted glass)
//! - Summary statistics with speed calculations
//! - CSS-only file type distribution bar chart
//! - Thumbnail gallery grid for recovered media
//! - Chain of custody / forensic provenance section
//! - PDF export with title page, stats, and hash verification
//!
//! Reports are fully offline and contain no external dependencies.

use std::fmt::Write as FmtWrite;
use std::path::Path;

use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// All data needed to generate a recovery report.
#[derive(Debug, Clone)]
pub struct ReportData {
    /// Report title (e.g. "Case #2024-0042 Recovery")
    pub title: String,
    /// Unique case identifier
    pub case_id: String,
    /// Source disk image or directory path
    pub source_path: String,
    /// Destination / export path
    pub dest_path: String,
    /// Human-readable timestamp of the recovery operation
    pub timestamp: String,
    /// Wall-clock duration of the recovery in seconds
    pub duration_secs: f64,
    /// Number of files successfully recovered
    pub files_recovered: usize,
    /// Number of files that failed to recover
    pub files_failed: usize,
    /// Total bytes recovered
    pub total_bytes: u64,
    /// Number of bad sectors encountered
    pub bad_sectors: usize,
    /// Per-type breakdown: (type_name, count, total_bytes)
    pub file_type_counts: Vec<(String, usize, u64)>,
    /// Thumbnail entries for the gallery section
    pub thumbnails: Vec<ThumbnailEntry>,
    /// Error messages collected during recovery
    pub errors: Vec<String>,
    /// Operator name / identification
    pub operator: String,
    /// Machine identification
    pub machine: String,
    /// Blake3 root hash of the recovered file tree
    pub root_hash: String,
}

/// A single thumbnail entry for the recovered-files gallery.
#[derive(Debug, Clone)]
pub struct ThumbnailEntry {
    /// Display name of the recovered file
    pub name: String,
    /// Full path to the recovered file
    pub path: String,
    /// Optional relative path to a generated thumbnail image.
    /// If `None`, a placeholder icon is shown instead.
    pub thumb_path: Option<String>,
    /// File size in bytes
    pub size: u64,
    /// Human-readable file type label (e.g. "JPEG", "PDF")
    pub file_type: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format bytes into a human-readable string (KiB / MiB / GiB / TiB).
fn format_bytes(bytes: u64) -> String {
    humansize::format_size(bytes, humansize::BINARY)
}

/// Format a duration in seconds to a friendly string.
fn format_duration(secs: f64) -> String {
    if secs < 1.0 {
        format!("{:.0} ms", secs * 1000.0)
    } else if secs < 60.0 {
        format!("{:.1} s", secs)
    } else if secs < 3600.0 {
        let m = (secs / 60.0).floor();
        let s = secs - m * 60.0;
        format!("{:.0}m {:.0}s", m, s)
    } else {
        let h = (secs / 3600.0).floor();
        let m = ((secs - h * 3600.0) / 60.0).floor();
        format!("{:.0}h {:.0}m", h, m)
    }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ---------------------------------------------------------------------------
// CSS theme
// ---------------------------------------------------------------------------

/// Inline CSS: deep-space dark with sapphire glows, frosted glass cards,
/// monospace technical data, responsive grid.
fn css() -> &'static str {
    r##"
:root {
    --bg-deep:       #0a0e1a;
    --bg-card:       rgba(15, 23, 42, 0.72);
    --bg-card-hover: rgba(20, 30, 55, 0.85);
    --glass-border:  rgba(56, 97, 251, 0.18);
    --glass-shadow:  rgba(30, 64, 175, 0.15);
    --sapphire:      #3b82f6;
    --sapphire-dim:  #1e40af;
    --sapphire-glow: rgba(59, 130, 246, 0.35);
    --cyan:          #22d3ee;
    --emerald:       #10b981;
    --rose:          #f43f5e;
    --amber:         #f59e0b;
    --text-primary:  #e2e8f0;
    --text-muted:    #94a3b8;
    --text-dim:      #64748b;
    --mono:          'Cascadia Code', 'Fira Code', 'JetBrains Mono', 'Consolas', monospace;
    --sans:          'Inter', 'Segoe UI', system-ui, -apple-system, sans-serif;
}

*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

html {
    font-size: 15px;
    scroll-behavior: smooth;
}

body {
    font-family: var(--sans);
    background: var(--bg-deep);
    color: var(--text-primary);
    line-height: 1.6;
    min-height: 100vh;
    background-image:
        radial-gradient(ellipse 80% 60% at 50% 0%, rgba(30, 64, 175, 0.18) 0%, transparent 70%),
        radial-gradient(circle at 20% 80%, rgba(59, 130, 246, 0.07) 0%, transparent 50%);
}

.container {
    max-width: 1120px;
    margin: 0 auto;
    padding: 2.5rem 1.5rem 4rem;
}

/* ---- Glass card ---- */
.card {
    background: var(--bg-card);
    border: 1px solid var(--glass-border);
    border-radius: 16px;
    padding: 1.75rem 2rem;
    margin-bottom: 1.75rem;
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    box-shadow:
        0 4px 30px var(--glass-shadow),
        inset 0 1px 0 rgba(255, 255, 255, 0.04);
    transition: background 0.2s;
}
.card:hover {
    background: var(--bg-card-hover);
}

/* ---- Header ---- */
.report-header {
    text-align: center;
    padding: 3rem 2rem 2.5rem;
    margin-bottom: 2rem;
    position: relative;
    overflow: hidden;
}
.report-header::before {
    content: '';
    position: absolute;
    top: -40%;
    left: 50%;
    transform: translateX(-50%);
    width: 500px;
    height: 500px;
    background: radial-gradient(circle, var(--sapphire-glow) 0%, transparent 70%);
    pointer-events: none;
    z-index: 0;
}
.report-header * { position: relative; z-index: 1; }
.report-header h1 {
    font-size: 2.2rem;
    font-weight: 700;
    letter-spacing: -0.02em;
    background: linear-gradient(135deg, var(--sapphire) 0%, var(--cyan) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    margin-bottom: 0.5rem;
}
.report-header .diamond-icon {
    font-size: 2.8rem;
    display: block;
    margin-bottom: 0.75rem;
    filter: drop-shadow(0 0 12px var(--sapphire-glow));
}
.report-header .meta {
    font-family: var(--mono);
    font-size: 0.85rem;
    color: var(--text-muted);
}
.report-header .meta span { margin: 0 0.75rem; }

/* ---- Section titles ---- */
.section-title {
    font-size: 1.15rem;
    font-weight: 600;
    margin-bottom: 1.25rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--text-primary);
}
.section-title .icon { font-size: 1.1rem; }

/* ---- Stats grid ---- */
.stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 1rem;
}
.stat-cell {
    background: rgba(15, 23, 42, 0.55);
    border: 1px solid rgba(56, 97, 251, 0.10);
    border-radius: 12px;
    padding: 1.1rem 1.25rem;
    text-align: center;
}
.stat-cell .value {
    font-family: var(--mono);
    font-size: 1.65rem;
    font-weight: 700;
    line-height: 1.2;
}
.stat-cell .label {
    font-size: 0.78rem;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-top: 0.35rem;
}
.stat-cell.recovered .value { color: var(--emerald); }
.stat-cell.failed .value    { color: var(--rose); }
.stat-cell.size .value      { color: var(--cyan); }
.stat-cell.speed .value     { color: var(--amber); }
.stat-cell.duration .value  { color: var(--sapphire); }
.stat-cell.sectors .value   { color: var(--rose); }

/* ---- Bar chart ---- */
.chart-container { margin-top: 0.5rem; }
.chart-row {
    display: grid;
    grid-template-columns: 120px 1fr 100px;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.6rem;
    font-size: 0.88rem;
}
.chart-row .type-label {
    font-family: var(--mono);
    font-size: 0.82rem;
    color: var(--text-muted);
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.chart-row .bar-track {
    height: 22px;
    background: rgba(30, 64, 175, 0.12);
    border-radius: 6px;
    overflow: hidden;
    position: relative;
}
.chart-row .bar-fill {
    height: 100%;
    border-radius: 6px;
    background: linear-gradient(90deg, var(--sapphire-dim), var(--sapphire));
    box-shadow: 0 0 10px var(--sapphire-glow);
    min-width: 2px;
    transition: width 0.4s ease;
}
.chart-row .bar-count {
    font-family: var(--mono);
    font-size: 0.8rem;
    color: var(--text-dim);
}

/* ---- Thumbnail gallery ---- */
.gallery-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 0.9rem;
}
.thumb-card {
    background: rgba(15, 23, 42, 0.6);
    border: 1px solid rgba(56, 97, 251, 0.08);
    border-radius: 10px;
    overflow: hidden;
    transition: transform 0.15s, box-shadow 0.15s;
}
.thumb-card:hover {
    transform: translateY(-2px);
    box-shadow: 0 6px 24px var(--glass-shadow);
}
.thumb-card .thumb-img {
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    display: block;
    background: rgba(30, 64, 175, 0.08);
}
.thumb-card .thumb-placeholder {
    width: 100%;
    aspect-ratio: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(30, 64, 175, 0.08);
    color: var(--text-dim);
    font-size: 2rem;
}
.thumb-card .thumb-info {
    padding: 0.55rem 0.65rem;
}
.thumb-card .thumb-name {
    font-size: 0.75rem;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.thumb-card .thumb-meta {
    font-family: var(--mono);
    font-size: 0.68rem;
    color: var(--text-dim);
    margin-top: 0.15rem;
}

/* ---- Errors ---- */
.error-list {
    list-style: none;
    max-height: 320px;
    overflow-y: auto;
}
.error-list li {
    font-family: var(--mono);
    font-size: 0.8rem;
    color: var(--rose);
    padding: 0.45rem 0.75rem;
    border-left: 3px solid var(--rose);
    margin-bottom: 0.4rem;
    background: rgba(244, 63, 94, 0.06);
    border-radius: 0 6px 6px 0;
    word-break: break-all;
}

/* ---- Chain of custody ---- */
.custody-table {
    width: 100%;
    border-collapse: collapse;
}
.custody-table tr { border-bottom: 1px solid rgba(56, 97, 251, 0.08); }
.custody-table tr:last-child { border-bottom: none; }
.custody-table th {
    text-align: left;
    font-size: 0.78rem;
    font-weight: 500;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 0.6rem 0.75rem;
    width: 160px;
    vertical-align: top;
}
.custody-table td {
    font-family: var(--mono);
    font-size: 0.85rem;
    color: var(--text-primary);
    padding: 0.6rem 0.75rem;
    word-break: break-all;
}

/* ---- Paths ---- */
.path-row {
    display: grid;
    grid-template-columns: 90px 1fr;
    gap: 0.5rem;
    margin-bottom: 0.4rem;
    font-size: 0.85rem;
}
.path-row .path-label {
    color: var(--text-dim);
    font-size: 0.78rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding-top: 0.15rem;
}
.path-row .path-value {
    font-family: var(--mono);
    color: var(--text-muted);
    word-break: break-all;
}

/* ---- Footer ---- */
.report-footer {
    text-align: center;
    padding-top: 2rem;
    margin-top: 1rem;
    border-top: 1px solid rgba(56, 97, 251, 0.10);
    font-size: 0.78rem;
    color: var(--text-dim);
}
.report-footer .brand {
    font-weight: 600;
    background: linear-gradient(135deg, var(--sapphire), var(--cyan));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
}

/* ---- Responsive ---- */
@media (max-width: 680px) {
    .container { padding: 1.25rem 0.75rem 2rem; }
    .card { padding: 1.25rem; border-radius: 12px; }
    .report-header h1 { font-size: 1.5rem; }
    .stats-grid { grid-template-columns: repeat(2, 1fr); }
    .chart-row { grid-template-columns: 80px 1fr 70px; }
    .gallery-grid { grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); }
}

@media print {
    body { background: #fff; color: #111; }
    .card { border: 1px solid #ddd; background: #fafafa; box-shadow: none; }
    .stat-cell .value { color: #111 !important; }
    .report-header h1 {
        -webkit-text-fill-color: #1e40af;
        background: none;
    }
}
"##
}

// ---------------------------------------------------------------------------
// HTML generation
// ---------------------------------------------------------------------------

/// Generate a self-contained HTML recovery report.
///
/// The returned string is a complete HTML document with all styles inlined.
/// No external resources, JavaScript, or network requests are required.
pub fn generate_html_report(data: &ReportData) -> String {
    let mut h = String::with_capacity(32_768);

    // ---- Document open ----
    let _ = write!(
        h,
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title} - Diamond Drill Recovery Report</title>
<style>{css}</style>
</head>
<body>
<div class="container">
"#,
        title = html_escape(&data.title),
        css = css(),
    );

    // ---- Header ----
    let _ = write!(
        h,
        r#"<div class="card report-header">
<span class="diamond-icon">&#x1F48E;</span>
<h1>Diamond Drill Recovery Report</h1>
<div class="meta">
<span>Case {case_id}</span>
<span>|</span>
<span>{timestamp}</span>
</div>
</div>
"#,
        case_id = html_escape(&data.case_id),
        timestamp = html_escape(&data.timestamp),
    );

    // ---- Paths ----
    let _ = write!(
        h,
        r#"<div class="card">
<div class="section-title"><span class="icon">&#x1F4C1;</span> Operation Paths</div>
<div class="path-row"><span class="path-label">Source</span><span class="path-value">{source}</span></div>
<div class="path-row"><span class="path-label">Dest</span><span class="path-value">{dest}</span></div>
</div>
"#,
        source = html_escape(&data.source_path),
        dest = html_escape(&data.dest_path),
    );

    // ---- Summary stats ----
    let speed = if data.duration_secs > 0.0 {
        format_bytes((data.total_bytes as f64 / data.duration_secs) as u64)
    } else {
        "N/A".to_string()
    };

    let _ = write!(
        h,
        r#"<div class="card">
<div class="section-title"><span class="icon">&#x1F4CA;</span> Recovery Summary</div>
<div class="stats-grid">
<div class="stat-cell recovered"><div class="value">{recovered}</div><div class="label">Recovered</div></div>
<div class="stat-cell failed"><div class="value">{failed}</div><div class="label">Failed</div></div>
<div class="stat-cell size"><div class="value">{total_size}</div><div class="label">Total Size</div></div>
<div class="stat-cell duration"><div class="value">{duration}</div><div class="label">Duration</div></div>
<div class="stat-cell speed"><div class="value">{speed}/s</div><div class="label">Throughput</div></div>
<div class="stat-cell sectors"><div class="value">{bad_sectors}</div><div class="label">Bad Sectors</div></div>
</div>
</div>
"#,
        recovered = data.files_recovered,
        failed = data.files_failed,
        total_size = format_bytes(data.total_bytes),
        duration = format_duration(data.duration_secs),
        speed = speed,
        bad_sectors = data.bad_sectors,
    );

    // ---- File type distribution (CSS bar chart) ----
    if !data.file_type_counts.is_empty() {
        let max_count = data
            .file_type_counts
            .iter()
            .map(|(_, c, _)| *c)
            .max()
            .unwrap_or(1)
            .max(1);

        let _ = write!(
            h,
            r#"<div class="card">
<div class="section-title"><span class="icon">&#x1F4C2;</span> File Type Distribution</div>
<div class="chart-container">
"#,
        );

        for (type_name, count, bytes) in &data.file_type_counts {
            let pct = (*count as f64 / max_count as f64) * 100.0;
            let _ = write!(
                h,
                r#"<div class="chart-row">
<span class="type-label" title="{type_name}">{type_name}</span>
<div class="bar-track"><div class="bar-fill" style="width:{pct:.1}%"></div></div>
<span class="bar-count">{count} ({bytes})</span>
</div>
"#,
                type_name = html_escape(type_name),
                pct = pct,
                count = count,
                bytes = format_bytes(*bytes),
            );
        }

        h.push_str("</div>\n</div>\n");
    }

    // ---- Thumbnail gallery ----
    if !data.thumbnails.is_empty() {
        let _ = write!(
            h,
            r#"<div class="card">
<div class="section-title"><span class="icon">&#x1F5BC;</span> Recovered Files Gallery</div>
<div class="gallery-grid">
"#,
        );

        for thumb in &data.thumbnails {
            let _ = write!(h, r#"<div class="thumb-card">"#);

            if let Some(ref tp) = thumb.thumb_path {
                let _ = write!(
                    h,
                    r#"<img class="thumb-img" src="{src}" alt="{alt}" loading="lazy">"#,
                    src = html_escape(tp),
                    alt = html_escape(&thumb.name),
                );
            } else {
                // File-type placeholder icon
                let icon = match thumb.file_type.to_lowercase().as_str() {
                    "jpeg" | "jpg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "ico" | "svg" => {
                        "&#x1F5BC;"
                    }
                    "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" => "&#x1F3AC;",
                    "mp3" | "wav" | "flac" | "ogg" | "aac" | "wma" => "&#x1F3B5;",
                    "pdf" => "&#x1F4D1;",
                    "doc" | "docx" | "odt" | "rtf" | "txt" => "&#x1F4DD;",
                    "xls" | "xlsx" | "csv" => "&#x1F4CA;",
                    "zip" | "rar" | "7z" | "tar" | "gz" => "&#x1F4E6;",
                    _ => "&#x1F4C4;",
                };
                let _ = write!(
                    h,
                    r#"<div class="thumb-placeholder">{icon}</div>"#,
                    icon = icon,
                );
            }

            let _ = write!(
                h,
                r#"<div class="thumb-info">
<div class="thumb-name" title="{full_name}">{name}</div>
<div class="thumb-meta">{ftype} &middot; {size}</div>
</div>
</div>
"#,
                full_name = html_escape(&thumb.name),
                name = html_escape(&thumb.name),
                ftype = html_escape(&thumb.file_type),
                size = format_bytes(thumb.size),
            );
        }

        h.push_str("</div>\n</div>\n");
    }

    // ---- Errors ----
    if !data.errors.is_empty() {
        let _ = write!(
            h,
            r#"<div class="card">
<div class="section-title"><span class="icon">&#x26A0;</span> Errors ({count})</div>
<ul class="error-list">
"#,
            count = data.errors.len(),
        );

        for err in &data.errors {
            let _ = write!(h, "<li>{}</li>\n", html_escape(err));
        }

        h.push_str("</ul>\n</div>\n");
    }

    // ---- Chain of custody ----
    let now_utc = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let _ = write!(
        h,
        r#"<div class="card">
<div class="section-title"><span class="icon">&#x1F512;</span> Chain of Custody</div>
<table class="custody-table">
<tr><th>Operator</th><td>{operator}</td></tr>
<tr><th>Machine</th><td>{machine}</td></tr>
<tr><th>Root Hash</th><td>{root_hash}</td></tr>
<tr><th>Report Generated</th><td>{gen_time}</td></tr>
</table>
</div>
"#,
        operator = html_escape(&data.operator),
        machine = html_escape(&data.machine),
        root_hash = html_escape(&data.root_hash),
        gen_time = html_escape(&now_utc),
    );

    // ---- Footer ----
    let _ = write!(
        h,
        r#"<div class="report-footer">
<p>Generated by <span class="brand">Diamond Drill</span></p>
<p style="margin-top:0.3rem">Forensic recovery &middot; Cryptographic verification &middot; Zero data modification</p>
</div>
"#,
    );

    // ---- Document close ----
    h.push_str("</div>\n</body>\n</html>\n");
    h
}

// ---------------------------------------------------------------------------
// Save HTML
// ---------------------------------------------------------------------------

/// Write the HTML report to disk.
///
/// The parent directory is created if it does not exist.
/// If `open_in_browser` is true, attempts to open the report in the default browser.
pub fn save_html_report(data: &ReportData, path: &Path, open_in_browser: bool) -> Result<()> {
    let html = generate_html_report(data);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    std::fs::write(path, html.as_bytes())
        .with_context(|| format!("Failed to write HTML report to {}", path.display()))?;

    tracing::info!("HTML report saved to {}", path.display());

    if open_in_browser {
        if let Err(e) = opener::open(path) {
            tracing::warn!("Could not open report in browser: {e}");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// PDF generation (lopdf)
// ---------------------------------------------------------------------------

/// Generate a PDF recovery report.
///
/// The PDF contains:
/// - Title page with "Diamond Drill Recovery Report"
/// - Summary statistics as formatted text
/// - Blake3 root hash for chain-of-custody verification
/// - Timestamp
///
/// Uses the `lopdf` crate to build the PDF structure manually.
pub fn generate_pdf_report(data: &ReportData, path: &Path) -> Result<()> {
    use lopdf::dictionary;
    use lopdf::{Document, Object, Stream};

    // -- Build text content for the PDF page --
    let speed_str = if data.duration_secs > 0.0 {
        format!(
            "{}/s",
            format_bytes((data.total_bytes as f64 / data.duration_secs) as u64)
        )
    } else {
        "N/A".to_string()
    };

    let now_utc = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    // PDF text lines (each will be rendered with Tf/Td/Tj operators)
    let lines: Vec<(f32, &str, String)> = vec![
        (24.0, "title", "Diamond Drill Recovery Report".to_string()),
        (10.0, "spacer", String::new()),
        (
            12.0,
            "body",
            format!("Case ID:          {}", data.case_id),
        ),
        (
            12.0,
            "body",
            format!("Timestamp:        {}", data.timestamp),
        ),
        (
            12.0,
            "body",
            format!("Source:           {}", data.source_path),
        ),
        (
            12.0,
            "body",
            format!("Destination:      {}", data.dest_path),
        ),
        (10.0, "spacer", String::new()),
        (14.0, "title", "Recovery Summary".to_string()),
        (10.0, "spacer", String::new()),
        (
            12.0,
            "body",
            format!("Files Recovered:  {}", data.files_recovered),
        ),
        (
            12.0,
            "body",
            format!("Files Failed:     {}", data.files_failed),
        ),
        (
            12.0,
            "body",
            format!("Total Size:       {}", format_bytes(data.total_bytes)),
        ),
        (
            12.0,
            "body",
            format!("Duration:         {}", format_duration(data.duration_secs)),
        ),
        (12.0, "body", format!("Throughput:       {}", speed_str)),
        (
            12.0,
            "body",
            format!("Bad Sectors:      {}", data.bad_sectors),
        ),
        (10.0, "spacer", String::new()),
        (14.0, "title", "Chain of Custody".to_string()),
        (10.0, "spacer", String::new()),
        (
            12.0,
            "body",
            format!("Operator:         {}", data.operator),
        ),
        (12.0, "body", format!("Machine:          {}", data.machine)),
        (
            12.0,
            "body",
            format!("Root Hash:        {}", data.root_hash),
        ),
        (
            12.0,
            "body",
            format!("Report Generated: {}", now_utc),
        ),
        (10.0, "spacer", String::new()),
        (10.0, "spacer", String::new()),
        (
            9.0,
            "watermark",
            "Generated by Diamond Drill".to_string(),
        ),
    ];

    // -- Build PDF stream content --
    let mut stream_content = String::with_capacity(4096);
    let mut y: f32 = 760.0; // Start near top of A4 (792 pt height)

    for (size, style, text) in &lines {
        if *style == "spacer" {
            y -= size * 1.2;
            continue;
        }

        let font_tag = if *style == "title" { "/F1" } else { "/F2" };

        // Escape PDF string special chars
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");

        let _ = write!(
            stream_content,
            "BT\n{font_tag} {size} Tf\n50 {y:.1} Td\n({escaped}) Tj\nET\n",
            font_tag = font_tag,
            size = size,
            y = y,
            escaped = escaped,
        );
        y -= size * 1.5;
    }

    // -- Assemble lopdf Document --
    let mut doc = Document::with_version("1.5");

    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let font1_id = doc.new_object_id();
    let font2_id = doc.new_object_id();
    let content_id = doc.new_object_id();

    // Helvetica-Bold for titles
    doc.objects.insert(
        font1_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica-Bold",
        }),
    );

    // Courier for body text (monospace for forensic data)
    doc.objects.insert(
        font2_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        }),
    );

    // Content stream
    let stream = Stream::new(dictionary! {}, stream_content.into_bytes());
    doc.objects.insert(content_id, Object::Stream(stream));

    // Page
    let resources = dictionary! {
        "Font" => dictionary! {
            "F1" => Object::Reference(font1_id),
            "F2" => Object::Reference(font2_id),
        },
    };

    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ],
            "Resources" => resources,
            "Contents" => Object::Reference(content_id),
        }),
    );

    // Pages
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => Object::Integer(1),
        }),
    );

    // Catalog
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));

    // -- Write PDF file --
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    doc.save(path)
        .with_context(|| format!("Failed to write PDF report to {}", path.display()))?;

    tracing::info!("PDF report saved to {}", path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Convenience: build ReportData from an ExportManifest
// ---------------------------------------------------------------------------

/// Build a ReportData from an export manifest JSON file.
///
/// This allows generating a report from a previous export by reading
/// the diamond-drill-manifest.json file.
pub fn report_data_from_manifest(manifest_path: &Path) -> Result<ReportData> {
    let data = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read manifest: {}", manifest_path.display()))?;

    let manifest: crate::export::ExportManifest = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse manifest: {}", manifest_path.display()))?;

    // Build file type counts from manifest entries
    let mut type_counts: std::collections::HashMap<String, (usize, u64)> =
        std::collections::HashMap::new();
    for entry in &manifest.entries {
        let ext = std::path::Path::new(&entry.source_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_uppercase();
        let counter = type_counts.entry(ext).or_insert((0, 0));
        counter.0 += 1;
        counter.1 += entry.size;
    }

    let mut file_type_counts: Vec<(String, usize, u64)> = type_counts
        .into_iter()
        .map(|(name, (count, bytes))| (name, count, bytes))
        .collect();
    file_type_counts.sort_by(|a, b| b.1.cmp(&a.1));

    // Compute root hash from all entry hashes
    let mut hasher = blake3::Hasher::new();
    for entry in &manifest.entries {
        hasher.update(entry.blake3_hash.as_bytes());
    }
    let root_hash = hex::encode(hasher.finalize().as_bytes());

    let operator = format!(
        "{}@{}",
        whoami::username(),
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    );

    let machine = format!(
        "{} ({} CPUs, {})",
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
        num_cpus::get(),
        std::env::consts::ARCH,
    );

    Ok(ReportData {
        title: "Recovery Report".to_string(),
        case_id: chrono::Utc::now().format("DD-%Y%m%d-%H%M").to_string(),
        source_path: manifest.source_root,
        dest_path: manifest.dest_root,
        timestamp: manifest.created_at,
        duration_secs: 0.0, // Not tracked in manifest
        files_recovered: manifest.total_files,
        files_failed: 0,
        total_bytes: manifest.total_bytes,
        bad_sectors: 0,
        file_type_counts,
        thumbnails: Vec::new(),
        errors: Vec::new(),
        operator,
        machine,
        root_hash,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> ReportData {
        ReportData {
            title: "Test Recovery".to_string(),
            case_id: "CASE-2024-0001".to_string(),
            source_path: "/dev/sda1".to_string(),
            dest_path: "/output/recovered".to_string(),
            timestamp: "2024-12-15 14:30:00 UTC".to_string(),
            duration_secs: 127.5,
            files_recovered: 1842,
            files_failed: 3,
            total_bytes: 4_831_229_952,
            bad_sectors: 7,
            file_type_counts: vec![
                ("JPEG".to_string(), 923, 2_100_000_000),
                ("PNG".to_string(), 412, 890_000_000),
                ("PDF".to_string(), 187, 540_000_000),
                ("DOCX".to_string(), 156, 320_000_000),
                ("MP4".to_string(), 89, 780_000_000),
                ("Other".to_string(), 75, 201_229_952),
            ],
            thumbnails: vec![
                ThumbnailEntry {
                    name: "photo_001.jpg".to_string(),
                    path: "/output/recovered/photo_001.jpg".to_string(),
                    thumb_path: None,
                    size: 2_400_000,
                    file_type: "JPEG".to_string(),
                },
                ThumbnailEntry {
                    name: "document.pdf".to_string(),
                    path: "/output/recovered/document.pdf".to_string(),
                    thumb_path: None,
                    size: 850_000,
                    file_type: "PDF".to_string(),
                },
            ],
            errors: vec![
                "Bad sector at offset 0x1A3F000: zero-filled 512 bytes".to_string(),
                "Corrupt header in file cluster 44291".to_string(),
            ],
            operator: "ryan@DRILL-RIG".to_string(),
            machine: "DRILL-RIG (16 CPUs, x86_64)".to_string(),
            root_hash: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
                .to_string(),
        }
    }

    #[test]
    fn test_generate_html_contains_key_elements() {
        let data = sample_data();
        let html = generate_html_report(&data);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Diamond Drill Recovery Report"));
        assert!(html.contains("CASE-2024-0001"));
        assert!(html.contains("1842")); // files recovered
        assert!(html.contains("JPEG"));
        assert!(html.contains("Chain of Custody"));
        assert!(html.contains("ryan@DRILL-RIG"));
        assert!(html.contains("a1b2c3d4e5f6"));
        assert!(html.contains("Bad sector at offset"));
        assert!(html.contains("photo_001.jpg"));
    }

    #[test]
    fn test_generate_html_no_thumbnails() {
        let mut data = sample_data();
        data.thumbnails.clear();
        let html = generate_html_report(&data);
        assert!(!html.contains(r#"class="gallery-grid"#));
    }

    #[test]
    fn test_generate_html_no_errors() {
        let mut data = sample_data();
        data.errors.clear();
        let html = generate_html_report(&data);
        assert!(!html.contains(r#"class="error-list"#));
    }

    #[test]
    fn test_generate_html_empty_file_types() {
        let mut data = sample_data();
        data.file_type_counts.clear();
        let html = generate_html_report(&data);
        assert!(!html.contains(r#"class="chart-container"#));
    }

    #[test]
    fn test_html_escape_special_chars() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a&b"), "a&amp;b");
        assert_eq!(html_escape(r#"x"y"#), "x&quot;y");
    }

    #[test]
    fn test_format_bytes() {
        assert!(format_bytes(0).contains('0'));
        let big = format_bytes(1_073_741_824);
        assert!(big.contains("GiB") || big.contains("GB"));
    }

    #[test]
    fn test_format_duration() {
        assert!(format_duration(0.5).contains("ms"));
        assert!(format_duration(45.0).contains("s"));
        assert!(format_duration(125.0).contains("m"));
        assert!(format_duration(7200.0).contains("h"));
    }

    #[test]
    fn test_save_html_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.html");
        let data = sample_data();

        let _ = save_html_report(&data, &path, false);
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Diamond Drill Recovery Report"));
    }

    #[test]
    fn test_generate_pdf_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.pdf");
        let data = sample_data();

        generate_pdf_report(&data, &path).expect("PDF generation should succeed");
        assert!(path.exists());

        // Verify it starts with PDF magic bytes
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
    }
}
