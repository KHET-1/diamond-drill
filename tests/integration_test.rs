//! Integration tests for Diamond Drill
//!
//! Tests the full DrillEngine API: index → search → preview → export → carve

use std::path::Path;
use tempfile::tempdir;
use tokio::fs;

use diamond_drill::carve::{CarveOptions, Carver};
use diamond_drill::core::{DrillEngine, FileType};
use diamond_drill::export::{ExportOptions, Exporter};

// ═══════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════

async fn create_test_structure(base: &Path) -> std::io::Result<()> {
    fs::create_dir_all(base.join("photos")).await?;
    fs::create_dir_all(base.join("documents")).await?;
    fs::create_dir_all(base.join("code")).await?;
    fs::create_dir_all(base.join(".hidden")).await?;

    fs::write(base.join("photos/vacation.jpg"), b"fake jpeg content here").await?;
    fs::write(base.join("photos/family.png"), b"fake png content here!").await?;
    fs::write(base.join("documents/report.pdf"), b"fake pdf content here").await?;
    fs::write(base.join("documents/notes.txt"), "Some important notes here").await?;
    fs::write(base.join("code/main.rs"), "fn main() { println!(\"hello\"); }").await?;
    fs::write(base.join(".hidden/secret.txt"), "hidden file content").await?;

    Ok(())
}

fn make_index_args(source: std::path::PathBuf) -> diamond_drill::cli::IndexArgs {
    diamond_drill::cli::IndexArgs {
        source,
        resume: false,
        index_file: None,
        skip_hidden: true,
        depth: None,
        extensions: None,
        thumbnails: false,
        workers: Some(2),
        checkpoint_interval: 0,
        bad_sector_report: None,
        block_size: 4096,
    }
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: Index + file_count
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_index_and_count() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let count = engine.file_count().await;
    assert_eq!(count, 5, "5 visible files (hidden skipped)");
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: get_all_files + get_files_by_type
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_get_files_by_type() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let all = engine.get_all_files().await.unwrap();
    assert_eq!(all.len(), 5);

    let images = engine.get_files_by_type("image").await.unwrap();
    assert_eq!(images.len(), 2, "vacation.jpg + family.png");

    let docs = engine.get_files_by_type("document").await.unwrap();
    assert_eq!(docs.len(), 2, "report.pdf + notes.txt");

    let code = engine.get_files_by_type("code").await.unwrap();
    assert_eq!(code.len(), 1, "main.rs");

    let unknown = engine.get_files_by_type("nosuchtype").await.unwrap();
    assert!(unknown.is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: search_fuzzy
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_search_fuzzy() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let results = engine.search_fuzzy("vacation").await.unwrap();
    assert!(!results.is_empty(), "should find vacation.jpg");
    assert!(
        results[0].contains("vacation"),
        "top result should contain 'vacation'"
    );

    let results = engine.search_fuzzy("main").await.unwrap();
    assert!(!results.is_empty(), "should find main.rs");

    let results = engine.search_fuzzy("xyznonexistent").await.unwrap();
    assert!(results.is_empty(), "garbage query should return empty");
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: search_glob
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_search_glob() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let results = engine.search_glob("*.jpg").await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("vacation.jpg"));

    let results = engine.search_glob("*.rs").await.unwrap();
    assert_eq!(results.len(), 1);

    let results = engine.search_glob("*.xyz").await.unwrap();
    assert!(results.is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: search_regex
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_search_regex() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let results = engine.search_regex(r"\.pdf$").await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("report.pdf"));

    let results = engine.search_regex(r"\.(jpg|png)$").await.unwrap();
    assert_eq!(results.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: search_exact
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_search_exact() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let results = engine.search_exact("notes").await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("notes.txt"));

    let results = engine.search_exact("NOTES").await.unwrap();
    assert_eq!(results.len(), 1, "search_exact should be case-insensitive");
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: get_file_info
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_get_file_info() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let all = engine.get_all_files().await.unwrap();
    let jpg_path = all.iter().find(|p| p.contains("vacation.jpg")).unwrap();

    let info = engine.get_file_info(jpg_path).await.unwrap();
    assert_eq!(info.file_type, FileType::Image);
    assert_eq!(info.extension, "jpg");
    assert!(info.size > 0);

    let err = engine.get_file_info("/no/such/file").await;
    assert!(err.is_err(), "nonexistent file should error");
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: summarize_files
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_summarize_files() {
    let dir = tempdir().unwrap();
    create_test_structure(dir.path()).await.unwrap();

    let engine = DrillEngine::new(dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(dir.path().to_path_buf()))
        .await
        .unwrap();

    let all = engine.get_all_files().await.unwrap();
    let summary = engine.summarize_files(&all).await.unwrap();

    assert!(!summary.is_empty());
    let total: usize = summary.iter().map(|(_, c)| c).sum();
    assert_eq!(total, 5);
}

// ═══════════════════════════════════════════════════════════════════
// Export: full pipeline with verification
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_export_full_pipeline() {
    let source_dir = tempdir().unwrap();
    let dest_dir = tempdir().unwrap();
    create_test_structure(source_dir.path()).await.unwrap();

    let engine = DrillEngine::new(source_dir.path().to_path_buf()).await.unwrap();
    engine
        .index_with_progress(&make_index_args(source_dir.path().to_path_buf()))
        .await
        .unwrap();

    let all_files = engine.get_all_files().await.unwrap();
    let options = ExportOptions {
        dest: dest_dir.path().to_path_buf(),
        preserve_structure: false,
        verify_hash: true,
        continue_on_error: false,
        create_manifest: true,
        dry_run: false,
    };

    let result = engine
        .export_files_with_progress(&all_files, &options, |_| {})
        .await
        .unwrap();

    assert_eq!(result.successful, 5);
    assert_eq!(result.failed, 0);
    assert!(result.manifest_path.is_some());

    let manifest_path = result.manifest_path.unwrap();
    let manifest_json = fs::read_to_string(&manifest_path).await.unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_json).unwrap();
    assert_eq!(manifest["total_files"], 5);

    let entries = manifest["entries"].as_array().unwrap();
    for entry in entries {
        assert!(!entry["blake3_hash"].as_str().unwrap().is_empty());
        assert!(entry["verified"].as_bool().unwrap());
    }
}

// ═══════════════════════════════════════════════════════════════════
// Export: dry run writes nothing
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_export_dry_run() {
    let source_dir = tempdir().unwrap();
    let dest_dir = tempdir().unwrap();

    let source_file = source_dir.path().join("test.txt");
    fs::write(&source_file, "dry run test content").await.unwrap();

    let entry = diamond_drill::core::FileEntry::new(
        source_file,
        &std::fs::metadata(source_dir.path().join("test.txt")).unwrap(),
    );

    let options = ExportOptions {
        dest: dest_dir.path().to_path_buf(),
        preserve_structure: false,
        verify_hash: false,
        continue_on_error: false,
        create_manifest: false,
        dry_run: true,
    };

    let exporter = Exporter::new(options);
    let result = exporter.export_batch(&[entry], |_| {}).await.unwrap();

    assert_eq!(result.successful, 1);
    assert!(result.manifest_path.is_none());

    let entries: Vec<_> = std::fs::read_dir(dest_dir.path()).unwrap().collect();
    assert!(entries.is_empty(), "dry run should not write files");
}

// ═══════════════════════════════════════════════════════════════════
// Carve: full dry-run on synthetic image
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_carve_jpeg_in_synthetic_image() {
    let dir = tempdir().unwrap();
    let img_path = dir.path().join("test.img");

    let mut img = vec![0u8; 8192];
    img[0] = 0xFF; img[1] = 0xD8; img[2] = 0xFF; img[3] = 0xE0;
    img[3000] = 0xFF; img[3001] = 0xD9;

    std::fs::write(&img_path, &img).unwrap();

    let opts = CarveOptions {
        source: img_path,
        output_dir: dir.path().join("out"),
        sector_aligned: false,
        min_size: 100,
        file_types: None,
        workers: 1,
        dry_run: true,
        verify: false,
    };

    let carver = Carver::new(opts);
    let (carved, result) = carver.carve().await.unwrap();

    assert_eq!(result.files_found, 1);
    assert_eq!(carved[0].extension, "jpg");
    assert!(carved[0].size >= 3000);
}

// ═══════════════════════════════════════════════════════════════════
// Carve: MP4 at sector boundary (ftyp offset-sig)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_carve_mp4_sector_aligned() {
    let dir = tempdir().unwrap();
    let img_path = dir.path().join("mp4.img");

    let mut img = vec![0u8; 16384];
    // MP4 at sector 0: ftyp box
    img[0..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x1C]); // box size 28
    img[4..8].copy_from_slice(b"ftyp");
    img[8..12].copy_from_slice(b"isom");
    // Another header at 8192 to bound it
    img[8192] = 0xFF; img[8193] = 0xD8; img[8194] = 0xFF;

    std::fs::write(&img_path, &img).unwrap();

    let opts = CarveOptions {
        source: img_path,
        output_dir: dir.path().join("out"),
        sector_aligned: true,
        min_size: 100,
        file_types: None,
        workers: 1,
        dry_run: true,
        verify: false,
    };

    let carver = Carver::new(opts);
    let (carved, _result) = carver.carve().await.unwrap();

    let mp4 = carved.iter().find(|c| c.extension == "mp4" || c.extension == "mov");
    assert!(mp4.is_some(), "Should find MP4 at sector-aligned offset 0");
}

// ═══════════════════════════════════════════════════════════════════
// DrillEngine: load_or_create (nonexistent index falls through)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_engine_load_or_create_fallback() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "aaa").await.unwrap();

    let engine = DrillEngine::load_or_create(&dir.path().to_path_buf()).await.unwrap();
    let count = engine.file_count().await;
    assert_eq!(count, 0, "no index loaded, engine should be empty");
}

// ═══════════════════════════════════════════════════════════════════
// Static tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_file_type_detection() {
    assert_eq!(FileType::from_extension("jpg"), FileType::Image);
    assert_eq!(FileType::from_extension("JPEG"), FileType::Image);
    assert_eq!(FileType::from_extension("mp4"), FileType::Video);
    assert_eq!(FileType::from_extension("mp3"), FileType::Audio);
    assert_eq!(FileType::from_extension("pdf"), FileType::Document);
    assert_eq!(FileType::from_extension("zip"), FileType::Archive);
    assert_eq!(FileType::from_extension("rs"), FileType::Code);
    assert_eq!(FileType::from_extension("exe"), FileType::Executable);
    assert_eq!(FileType::from_extension("sqlite"), FileType::Database);
    assert_eq!(FileType::from_extension("xyz"), FileType::Other);
}

#[test]
fn test_file_type_icons() {
    assert_eq!(FileType::Image.icon(), "🖼 ");
    assert_eq!(FileType::Video.icon(), "🎬");
    assert_eq!(FileType::Audio.icon(), "🎵");
    assert_eq!(FileType::Document.icon(), "📄");
    assert_eq!(FileType::Archive.icon(), "📦");
    assert_eq!(FileType::Code.icon(), "💻");
}

#[test]
fn test_glob_patterns() {
    use globset::GlobBuilder;

    let glob = GlobBuilder::new("*.jpg")
        .case_insensitive(true)
        .build()
        .unwrap()
        .compile_matcher();

    assert!(glob.is_match("photo.jpg"));
    assert!(glob.is_match("PHOTO.JPG"));
    assert!(!glob.is_match("photo.png"));
}

#[test]
fn test_fuzzy_matching() {
    use fuzzy_matcher::skim::SkimMatcherV2;
    use fuzzy_matcher::FuzzyMatcher;

    let matcher = SkimMatcherV2::default();
    assert!(matcher.fuzzy_match("vacation_photo.jpg", "vac").is_some());
    assert!(matcher.fuzzy_match("photograph", "photgraph").is_some());
}
