//! Integration tests for Diamond Drill Deduplication
//!
//! Verifies the full deduplication workflow using the DrillEngine.

use std::path::PathBuf;
use tempfile::tempdir;

use diamond_drill::cli::{DedupArgs, DedupKeepStrategy, DedupReportFormat};
use diamond_drill::core::DrillEngine;

/// Create a test environment with duplicate files
async fn create_dedup_test_structure(base: &PathBuf) -> std::io::Result<()> {
    std::fs::create_dir_all(base)?;

    // Create 3 identical files
    let content = b"duplicate content";
    std::fs::write(base.join("orig.txt"), content)?;
    std::fs::write(base.join("copy1.txt"), content)?;
    std::fs::write(base.join("copy2.txt"), content)?;

    // Create a unique file
    std::fs::write(base.join("unique.txt"), b"unique content")?;

    // Create a near-duplicate (rename/backup style) for fuzzy detection
    // Note: fuzzy detection needs same content for size check in current implementation,
    // or very similar size. The current implementation groups by name similarity first,
    // then checks size similarity.
    std::fs::write(base.join("report.pdf"), b"pdf content")?;
    std::fs::write(base.join("report_backup.pdf"), b"pdf content")?;

    Ok(())
}

#[tokio::test]
async fn test_engine_dedup_integration() {
    let source_dir = tempdir().unwrap();
    let source_path = source_dir.path().to_path_buf();

    // Setup
    create_dedup_test_structure(&source_path).await.unwrap();

    // Initialize engine
    let engine = DrillEngine::new(source_path.clone()).await.unwrap();

    // Index first (engine.run_dedup does this automatically if index is empty,
    // but explicit indexing is good for testing)
    let index_args = diamond_drill::cli::IndexArgs {
        source: source_path.clone(),
        resume: false,
        index_file: None,
        skip_hidden: false, // Temp dirs often start with '.', so we must NOT skip hidden for tests
        depth: None,
        extensions: None,
        thumbnails: false,
        workers: None,
        checkpoint_interval: 1000,
        bad_sector_report: None,
        block_size: 4096,
    };
    engine.index_with_progress(&index_args).await.unwrap();

    // 1. Test Exact Deduplication (Dry Run)
    let dedup_args = DedupArgs {
        source: source_path.clone(),
        keep: DedupKeepStrategy::Oldest, // consistent strategy
        fuzzy: false,
        threshold: 85,
        min_size: 1,
        purge: false, // Dry run
        report: DedupReportFormat::Json,
    };

    // We can't easily capture stdout here to verify report content without capturing implementation,
    // but we can verify it runs without error.
    // Ideally, we would unit test the logic (which is done in dedup/mod.rs),
    // this test ensures the engine wiring is correct.
    engine.run_dedup(&dedup_args).await.unwrap();

    // 2. Test Fuzzy Deduplication
    let fuzzy_args = DedupArgs {
        source: source_path.clone(),
        keep: DedupKeepStrategy::Cleanest,
        fuzzy: true,
        threshold: 80,
        min_size: 1,
        purge: false,
        report: DedupReportFormat::Human,
    };

    engine.run_dedup(&fuzzy_args).await.unwrap();

    // 3. Test Purge (Actual Deletion)
    // We expect 'copy1.txt' and 'copy2.txt' to be deleted if 'orig.txt' is oldest/kept,
    // or based on strategy.

    // First, verify all files exist
    assert!(source_path.join("orig.txt").exists());
    assert!(source_path.join("copy1.txt").exists());
    assert!(source_path.join("copy2.txt").exists());

    let purge_args = DedupArgs {
        source: source_path.clone(),
        keep: DedupKeepStrategy::Cleanest, // Should keep "orig.txt" (shortest/cleanest name)
        fuzzy: false,
        threshold: 85,
        min_size: 1,
        purge: true, // ACTUAL DELETE
        report: DedupReportFormat::Json,
    };

    engine.run_dedup(&purge_args).await.unwrap();

    // Verify deletions
    // Note: The implementation of 'Cleanest' prefers non-temp names.
    // 'orig.txt', 'copy1.txt', 'copy2.txt' - none look like temp names by the strict definition in dedup/mod.rs
    // (ends with ~, .bak, .tmp, etc.).
    // However, shortest name might be a tiebreaker or 'Newest' if score is same.
    // Let's rely on the fact that duplicates SHOULD be gone.

    let mut count = 0;
    if source_path.join("orig.txt").exists() {
        count += 1;
    }
    if source_path.join("copy1.txt").exists() {
        count += 1;
    }
    if source_path.join("copy2.txt").exists() {
        count += 1;
    }

    // Should have 1 remaining from the group of 3
    assert_eq!(
        count, 1,
        "Should preserve exactly one copy from the duplicate group"
    );
    assert!(
        source_path.join("unique.txt").exists(),
        "Unique file should stick around"
    );
}
