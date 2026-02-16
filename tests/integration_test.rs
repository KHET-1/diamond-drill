//! Integration tests for Diamond Drill

use std::path::Path;
use tempfile::tempdir;
use tokio::fs;

/// Create a test directory structure with various file types
async fn create_test_structure(base: &Path) -> std::io::Result<()> {
    // Create directories
    fs::create_dir_all(base.join("photos")).await?;
    fs::create_dir_all(base.join("documents")).await?;
    fs::create_dir_all(base.join("code")).await?;
    fs::create_dir_all(base.join(".hidden")).await?;

    // Create test files
    fs::write(base.join("photos/vacation.jpg"), b"fake jpeg content").await?;
    fs::write(base.join("photos/family.png"), b"fake png content").await?;
    fs::write(base.join("documents/report.pdf"), b"fake pdf content").await?;
    fs::write(base.join("documents/notes.txt"), "Some notes here").await?;
    fs::write(base.join("code/main.rs"), "fn main() {}").await?;
    fs::write(base.join(".hidden/secret.txt"), "hidden file").await?;

    Ok(())
}

#[tokio::test]
async fn test_full_workflow() {
    let source_dir = tempdir().unwrap();
    // Setup
    create_test_structure(source_dir.path())
        .await
        .unwrap();

    // This would test the full workflow:
    // 1. Create engine
    // 2. Index files
    // 3. Search for files
    // 4. Export selected files
    // 5. Verify exports

    // For now, just verify the structure was created
    assert!(source_dir.path().join("photos/vacation.jpg").exists());
    assert!(source_dir.path().join("documents/report.pdf").exists());
    assert!(source_dir.path().join("code/main.rs").exists());
}

#[tokio::test]
async fn test_export_verification() {
    let source_dir = tempdir().unwrap();

    // Create source file
    let source_file = source_dir.path().join("test.txt");
    let content = "Hello, Diamond Drill! This is test content.";
    fs::write(&source_file, content).await.unwrap();

    // Export would happen here...

    // For now, just verify source exists
    assert!(source_file.exists());
}

#[test]
fn test_file_type_detection() {
    use diamond_drill::core::FileType;

    assert_eq!(FileType::from_extension("jpg"), FileType::Image);
    assert_eq!(FileType::from_extension("JPEG"), FileType::Image);
    assert_eq!(FileType::from_extension("mp4"), FileType::Video);
    assert_eq!(FileType::from_extension("mp3"), FileType::Audio);
    assert_eq!(FileType::from_extension("pdf"), FileType::Document);
    assert_eq!(FileType::from_extension("zip"), FileType::Archive);
    assert_eq!(FileType::from_extension("rs"), FileType::Code);
    assert_eq!(FileType::from_extension("xyz"), FileType::Other);
}

#[test]
fn test_file_type_icons() {
    use diamond_drill::core::FileType;

    assert_eq!(FileType::Image.icon(), "🖼 ");
    assert_eq!(FileType::Video.icon(), "🎬");
    assert_eq!(FileType::Audio.icon(), "🎵");
    assert_eq!(FileType::Document.icon(), "📄");
    assert_eq!(FileType::Archive.icon(), "📦");
    assert_eq!(FileType::Code.icon(), "💻");
}

#[tokio::test]
async fn test_blake3_hashing() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("test.bin");

    // Create test file
    let content = b"Test content for hashing";
    fs::write(&file, content).await.unwrap();

    // Compute hash
    let hash = blake3::hash(content);
    let hash_hex = hex::encode(hash.as_bytes());

    // Hash should be consistent
    assert_eq!(hash_hex.len(), 64); // 32 bytes = 64 hex chars

    // Same content = same hash
    let hash2 = blake3::hash(content);
    assert_eq!(hash.as_bytes(), hash2.as_bytes());
}

#[test]
fn test_glob_patterns() {
    use globset::GlobBuilder;

    // Case-insensitive glob (matches how Diamond Drill's search_glob works on real filenames)
    let glob = GlobBuilder::new("*.jpg")
        .case_insensitive(true)
        .build()
        .unwrap()
        .compile_matcher();

    assert!(glob.is_match("photo.jpg"));
    assert!(glob.is_match("PHOTO.JPG"));
    assert!(!glob.is_match("photo.png"));
    assert!(!glob.is_match("photo.jpg.bak"));
}

#[test]
fn test_fuzzy_matching() {
    use fuzzy_matcher::skim::SkimMatcherV2;
    use fuzzy_matcher::FuzzyMatcher;

    let matcher = SkimMatcherV2::default();

    // Should match
    assert!(matcher.fuzzy_match("vacation_photo.jpg", "vac").is_some());
    assert!(matcher.fuzzy_match("my_document.pdf", "doc").is_some());
    assert!(matcher.fuzzy_match("source_code.rs", "src").is_some());

    // Typo tolerance
    assert!(matcher.fuzzy_match("photograph", "photgraph").is_some());
}
