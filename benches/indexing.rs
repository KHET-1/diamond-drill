//! Benchmarks for Diamond Drill performance
//!
//! Run: cargo bench
//! Run specific: cargo bench -- blake3
//! Compare: cargo bench -- --save-baseline v1 && cargo bench -- --baseline v1

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::path::Path;
use tempfile::tempdir;

// ============================================================================
// File Type Detection
// ============================================================================

fn benchmark_file_type_detection(c: &mut Criterion) {
    let extensions = vec![
        "jpg", "png", "gif", "mp4", "mp3", "pdf", "txt", "rs", "py", "zip", "docx", "xlsx", "mov",
        "flac", "tar", "7z", "webp", "bmp", "svg", "wasm",
    ];

    c.bench_function("file_type_from_extension_20", |b| {
        b.iter(|| {
            for ext in &extensions {
                let _ = black_box(diamond_drill::core::FileType::from_extension(ext));
            }
        })
    });
}

// ============================================================================
// Blake3 Hashing (throughput-oriented)
// ============================================================================

fn benchmark_blake3_hashing(c: &mut Criterion) {
    let mut group = c.benchmark_group("blake3_hashing");

    for size in [1_024, 10_240, 102_400, 1_024_000, 10_240_000].iter() {
        let data = vec![0xABu8; *size];

        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let hash = blake3::hash(black_box(&data));
                black_box(hash)
            })
        });
    }

    group.finish();
}

// ============================================================================
// Fuzzy Matching
// ============================================================================

fn benchmark_fuzzy_matching(c: &mut Criterion) {
    use fuzzy_matcher::skim::SkimMatcherV2;
    use fuzzy_matcher::FuzzyMatcher;

    let matcher = SkimMatcherV2::default();
    let patterns = vec!["vac", "doc", "photo", "src", "test"];
    let filenames: Vec<String> = (0..1000)
        .map(|i| format!("some_long_filename_with_number_{}.ext", i))
        .collect();

    c.bench_function("fuzzy_match_1000_files_x5_patterns", |b| {
        b.iter(|| {
            for pattern in &patterns {
                for filename in &filenames {
                    let _ = black_box(matcher.fuzzy_match(filename, pattern));
                }
            }
        })
    });
}

// ============================================================================
// Scan Throughput — measures file discovery speed
// ============================================================================

fn create_bench_tree(dir: &Path, count: usize) {
    // Create a mix of file types across subdirectories
    for i in 0..count {
        let subdir = dir.join(format!("dir_{}", i % 10));
        std::fs::create_dir_all(&subdir).unwrap();

        let ext = match i % 8 {
            0 => "jpg",
            1 => "pdf",
            2 => "txt",
            3 => "rs",
            4 => "png",
            5 => "mp3",
            6 => "docx",
            _ => "bin",
        };
        let path = subdir.join(format!("file_{}.{}", i, ext));
        // Small files — we're benchmarking discovery, not I/O
        std::fs::write(&path, format!("bench-content-{}", i)).unwrap();
    }
}

fn benchmark_scan_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan_throughput");
    group.sample_size(10); // Fewer samples for I/O-bound benchmarks

    for &file_count in &[100, 500, 1000] {
        let dir = tempdir().unwrap();
        create_bench_tree(dir.path(), file_count);

        group.throughput(Throughput::Elements(file_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            &file_count,
            |b, _| {
                b.iter(|| {
                    let mut count = 0u64;
                    for entry in walkdir::WalkDir::new(dir.path())
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file())
                    {
                        let _ = black_box(entry.path());
                        count += 1;
                    }
                    black_box(count)
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Dedup Hashing — measures hash throughput for duplicate detection
// ============================================================================

fn benchmark_dedup_hashing(c: &mut Criterion) {
    let mut group = c.benchmark_group("dedup_hashing");
    group.sample_size(10);

    let dir = tempdir().unwrap();

    // Create files of varying sizes for realistic dedup benchmarking
    let sizes: Vec<(usize, &str)> = vec![
        (1_024, "1KB"),
        (10_240, "10KB"),
        (102_400, "100KB"),
        (1_024_000, "1MB"),
    ];

    for (size, label) in &sizes {
        let file_path = dir.path().join(format!("bench_file_{}.dat", label));
        let data: Vec<u8> = (0..*size).map(|i| (i % 256) as u8).collect();
        std::fs::write(&file_path, &data).unwrap();

        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("hash_file", label),
            &file_path,
            |b, path| {
                b.iter(|| {
                    let hash = diamond_drill::dedup::hash_file(black_box(path)).unwrap();
                    black_box(hash)
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// FileEntry creation throughput
// ============================================================================

fn benchmark_file_entry_creation(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let test_file = dir.path().join("test.jpg");
    std::fs::write(&test_file, "fake jpg content for benchmark").unwrap();

    c.bench_function("file_entry_from_path", |b| {
        b.iter(|| {
            let metadata = std::fs::metadata(black_box(&test_file)).unwrap();
            let _ = black_box((
                test_file.file_name().unwrap().to_string_lossy().to_string(),
                metadata.len(),
                diamond_drill::core::FileType::from_extension("jpg"),
            ));
        })
    });
}

// ============================================================================
// Groups
// ============================================================================

criterion_group!(
    benches,
    benchmark_file_type_detection,
    benchmark_blake3_hashing,
    benchmark_fuzzy_matching,
    benchmark_scan_throughput,
    benchmark_dedup_hashing,
    benchmark_file_entry_creation,
);

criterion_main!(benches);
