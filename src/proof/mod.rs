//! Proof Manifest module - Cryptographic chain-of-custody for exports
//!
//! Generates Blake3-based proof manifests with Merkle-like root hashes,
//! chain-of-custody metadata, and offline verification capability.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Current proof manifest format version
pub const PROOF_VERSION: u32 = 1;

/// Tool identification string
pub const TOOL_NAME: &str = "Diamond Drill";

/// A cryptographic proof manifest for an export operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofManifest {
    /// Format version
    pub version: u32,
    /// Tool that generated this manifest
    pub tool: String,
    /// Tool version
    pub tool_version: String,
    /// When the manifest was created
    pub created_at: DateTime<Utc>,
    /// Source root path
    pub source_root: String,
    /// Destination root path
    pub dest_root: String,
    /// Merkle-like root hash (Blake3 of sorted entry hashes)
    pub root_hash: String,
    /// Total files in manifest
    pub total_files: usize,
    /// Total bytes across all files
    pub total_bytes: u64,
    /// Individual file entries
    pub entries: Vec<ProofEntry>,
    /// Chain of custody metadata
    pub chain_of_custody: ChainOfCustody,
}

/// A single file entry in the proof manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofEntry {
    /// Original source path
    pub source_path: String,
    /// Destination path after export
    pub dest_path: String,
    /// File size in bytes
    pub size: u64,
    /// Blake3 hash of the file contents
    pub blake3_hash: String,
    /// When this file was exported
    pub exported_at: DateTime<Utc>,
    /// Notes about bad sectors (if any)
    pub bad_sector_notes: Option<String>,
    /// Whether the hash was verified after copy
    pub verified: bool,
}

/// Chain of custody metadata for legal/forensic provenance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainOfCustody {
    /// Operator identification (hostname + username)
    pub operator: String,
    /// Machine identification
    pub machine: String,
    /// Operating system
    pub os: String,
    /// When the export operation started
    pub started_at: DateTime<Utc>,
    /// When the export operation completed
    pub completed_at: Option<DateTime<Utc>>,
    /// Export options summary
    pub options_used: BTreeMap<String, String>,
}

impl ChainOfCustody {
    /// Create chain of custody from current environment
    pub fn from_environment() -> Self {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let username = whoami::username();
        let operator = format!("{}@{}", username, hostname);

        let machine = format!(
            "{} ({} CPUs, {})",
            hostname,
            num_cpus::get(),
            std::env::consts::ARCH
        );

        let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

        Self {
            operator,
            machine,
            os,
            started_at: Utc::now(),
            completed_at: None,
            options_used: BTreeMap::new(),
        }
    }
}

/// Result of verifying a proof manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    /// Total files in manifest
    pub total: usize,
    /// Files that verified successfully
    pub verified: usize,
    /// Files that failed verification
    pub failed: usize,
    /// Files that are missing from disk
    pub missing: usize,
    /// Details of tampered files
    pub tampered: Vec<TamperInfo>,
    /// Whether the root hash matches
    pub root_hash_valid: bool,
    /// Expected root hash
    pub expected_root_hash: String,
    /// Computed root hash
    pub computed_root_hash: String,
}

impl VerifyResult {
    /// Check if verification passed completely
    pub fn is_clean(&self) -> bool {
        self.failed == 0 && self.missing == 0 && self.root_hash_valid
    }
}

/// Information about a tampered or missing file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TamperInfo {
    /// File path
    pub path: String,
    /// Expected hash from manifest
    pub expected_hash: String,
    /// Actual hash (empty if file missing)
    pub actual_hash: String,
    /// Type of issue
    pub issue: TamperType,
}

/// Type of tampering detected
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TamperType {
    /// File hash doesn't match
    HashMismatch,
    /// File is missing from disk
    Missing,
    /// File size changed
    SizeChanged,
}

/// Compute a Merkle-like root hash from proof entries.
///
/// Algorithm: sort entries by source_path, concatenate their blake3 hashes
/// in order, then Blake3 the concatenation.
pub fn compute_root_hash(entries: &[ProofEntry]) -> String {
    if entries.is_empty() {
        return blake3::hash(b"empty").to_hex().to_string();
    }

    // Sort entries by source path for deterministic ordering
    let mut sorted: Vec<&ProofEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.source_path.cmp(&b.source_path));

    // Concatenate all hashes
    let mut hasher = blake3::Hasher::new();
    for entry in &sorted {
        hasher.update(entry.blake3_hash.as_bytes());
    }

    hasher.finalize().to_hex().to_string()
}

/// Build a ProofManifest from export data
pub fn build_manifest(
    source_root: &Path,
    dest_root: &Path,
    entries: Vec<ProofEntry>,
    custody: ChainOfCustody,
) -> ProofManifest {
    let total_bytes: u64 = entries.iter().map(|e| e.size).sum();
    let root_hash = compute_root_hash(&entries);

    ProofManifest {
        version: PROOF_VERSION,
        tool: TOOL_NAME.to_string(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: Utc::now(),
        source_root: source_root.to_string_lossy().to_string(),
        dest_root: dest_root.to_string_lossy().to_string(),
        root_hash,
        total_files: entries.len(),
        total_bytes,
        entries,
        chain_of_custody: custody,
    }
}

/// Save a proof manifest to disk
pub fn save_manifest(manifest: &ProofManifest, path: &Path) -> Result<()> {
    let json =
        serde_json::to_string_pretty(manifest).context("Failed to serialize proof manifest")?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, json)
        .with_context(|| format!("Failed to write manifest to {}", path.display()))?;

    Ok(())
}

/// Load a proof manifest from disk
pub fn load_manifest(path: &Path) -> Result<ProofManifest> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read manifest from {}", path.display()))?;

    let manifest: ProofManifest = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse manifest from {}", path.display()))?;

    Ok(manifest)
}

/// Verify a proof manifest against files on disk.
///
/// Re-hashes every dest file and compares against manifest entries.
/// Also recomputes and verifies the root hash.
pub fn verify_manifest(manifest: &ProofManifest) -> Result<VerifyResult> {
    let mut verified = 0usize;
    let mut failed = 0usize;
    let mut missing = 0usize;
    let mut tampered = Vec::new();

    for entry in &manifest.entries {
        let path = Path::new(&entry.dest_path);

        if !path.exists() {
            missing += 1;
            tampered.push(TamperInfo {
                path: entry.dest_path.clone(),
                expected_hash: entry.blake3_hash.clone(),
                actual_hash: String::new(),
                issue: TamperType::Missing,
            });
            continue;
        }

        // Check file size
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() != entry.size {
                failed += 1;
                tampered.push(TamperInfo {
                    path: entry.dest_path.clone(),
                    expected_hash: entry.blake3_hash.clone(),
                    actual_hash: format!("size:{}", metadata.len()),
                    issue: TamperType::SizeChanged,
                });
                continue;
            }
        }

        // Compute blake3 hash
        match compute_file_hash_sync(path) {
            Ok(hash) => {
                if hash == entry.blake3_hash {
                    verified += 1;
                } else {
                    failed += 1;
                    tampered.push(TamperInfo {
                        path: entry.dest_path.clone(),
                        expected_hash: entry.blake3_hash.clone(),
                        actual_hash: hash,
                        issue: TamperType::HashMismatch,
                    });
                }
            }
            Err(_) => {
                failed += 1;
                tampered.push(TamperInfo {
                    path: entry.dest_path.clone(),
                    expected_hash: entry.blake3_hash.clone(),
                    actual_hash: String::new(),
                    issue: TamperType::Missing,
                });
            }
        }
    }

    // Verify root hash
    let computed_root = compute_root_hash(&manifest.entries);
    let root_hash_valid = computed_root == manifest.root_hash;

    Ok(VerifyResult {
        total: manifest.entries.len(),
        verified,
        failed,
        missing,
        tampered,
        root_hash_valid,
        expected_root_hash: manifest.root_hash.clone(),
        computed_root_hash: computed_root,
    })
}

/// Compute blake3 hash of a file synchronously
fn compute_file_hash_sync(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file =
        std::fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;

    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Format a VerifyResult for human display
pub fn format_verify_result(result: &VerifyResult) -> String {
    let mut out = String::new();

    out.push_str("\n  Diamond Drill Proof Verification\n");
    out.push_str("  ==========================================\n\n");

    if result.is_clean() {
        out.push_str("  VERIFICATION PASSED\n\n");
    } else {
        out.push_str("  VERIFICATION FAILED\n\n");
    }

    out.push_str(&format!("  Total files:    {}\n", result.total));
    out.push_str(&format!("  Verified:       {}\n", result.verified));
    out.push_str(&format!("  Failed:         {}\n", result.failed));
    out.push_str(&format!("  Missing:        {}\n", result.missing));
    out.push_str(&format!(
        "  Root hash:      {}\n",
        if result.root_hash_valid {
            "VALID"
        } else {
            "INVALID"
        }
    ));

    if !result.tampered.is_empty() {
        out.push_str("\n  Tampered Files:\n");
        for info in &result.tampered {
            let issue_str = match &info.issue {
                TamperType::HashMismatch => "HASH MISMATCH",
                TamperType::Missing => "MISSING",
                TamperType::SizeChanged => "SIZE CHANGED",
            };
            out.push_str(&format!("    [{}] {}\n", issue_str, info.path));
            if !info.expected_hash.is_empty() {
                out.push_str(&format!("      Expected: {}\n", info.expected_hash));
            }
            if !info.actual_hash.is_empty() {
                out.push_str(&format!("      Actual:   {}\n", info.actual_hash));
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_compute_root_hash_deterministic() {
        let entries = vec![
            ProofEntry {
                source_path: "/a.txt".to_string(),
                dest_path: "/out/a.txt".to_string(),
                size: 100,
                blake3_hash: "abc123".to_string(),
                exported_at: Utc::now(),
                bad_sector_notes: None,
                verified: true,
            },
            ProofEntry {
                source_path: "/b.txt".to_string(),
                dest_path: "/out/b.txt".to_string(),
                size: 200,
                blake3_hash: "def456".to_string(),
                exported_at: Utc::now(),
                bad_sector_notes: None,
                verified: true,
            },
        ];

        let hash1 = compute_root_hash(&entries);
        let hash2 = compute_root_hash(&entries);
        assert_eq!(hash1, hash2, "Root hash must be deterministic");
        assert!(!hash1.is_empty());
    }

    #[test]
    fn test_compute_root_hash_order_independent() {
        let entry_a = ProofEntry {
            source_path: "/a.txt".to_string(),
            dest_path: "/out/a.txt".to_string(),
            size: 100,
            blake3_hash: "abc123".to_string(),
            exported_at: Utc::now(),
            bad_sector_notes: None,
            verified: true,
        };
        let entry_b = ProofEntry {
            source_path: "/b.txt".to_string(),
            dest_path: "/out/b.txt".to_string(),
            size: 200,
            blake3_hash: "def456".to_string(),
            exported_at: Utc::now(),
            bad_sector_notes: None,
            verified: true,
        };

        // Different input order, same result
        let hash_ab = compute_root_hash(&[entry_a.clone(), entry_b.clone()]);
        let hash_ba = compute_root_hash(&[entry_b, entry_a]);
        assert_eq!(hash_ab, hash_ba, "Root hash must be order-independent");
    }

    #[test]
    fn test_compute_root_hash_empty() {
        let hash = compute_root_hash(&[]);
        assert!(
            !hash.is_empty(),
            "Empty entries should produce a valid hash"
        );
    }

    #[test]
    fn test_tamper_detection() {
        let dir = tempdir().unwrap();

        // Create a file
        let file_path = dir.path().join("evidence.txt");
        std::fs::write(&file_path, "original content").unwrap();

        // Compute its hash
        let hash = compute_file_hash_sync(&file_path).unwrap();

        // Build manifest with correct hash
        let entries = vec![ProofEntry {
            source_path: "/source/evidence.txt".to_string(),
            dest_path: file_path.to_string_lossy().to_string(),
            size: 16, // "original content" is 16 bytes
            blake3_hash: hash.clone(),
            exported_at: Utc::now(),
            bad_sector_notes: None,
            verified: true,
        }];

        let custody = ChainOfCustody {
            operator: "test".to_string(),
            machine: "test-machine".to_string(),
            os: "test-os".to_string(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            options_used: BTreeMap::new(),
        };

        let manifest = build_manifest(Path::new("/source"), dir.path(), entries, custody);

        // Verify — should pass
        let result = verify_manifest(&manifest).unwrap();
        assert!(result.is_clean());
        assert_eq!(result.verified, 1);
        assert_eq!(result.failed, 0);
        assert!(result.root_hash_valid);

        // Now tamper with the file
        std::fs::write(&file_path, "TAMPERED content!").unwrap();

        // Verify again — should fail
        let result2 = verify_manifest(&manifest).unwrap();
        assert!(!result2.is_clean());
        assert_eq!(result2.tampered.len(), 1);
    }

    #[test]
    fn test_missing_file_detection() {
        let entries = vec![ProofEntry {
            source_path: "/source/gone.txt".to_string(),
            dest_path: "/nonexistent/path/gone.txt".to_string(),
            size: 10,
            blake3_hash: "fakehash".to_string(),
            exported_at: Utc::now(),
            bad_sector_notes: None,
            verified: true,
        }];

        let custody = ChainOfCustody {
            operator: "test".to_string(),
            machine: "test".to_string(),
            os: "test".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            options_used: BTreeMap::new(),
        };

        let manifest = build_manifest(
            Path::new("/source"),
            Path::new("/nonexistent"),
            entries,
            custody,
        );

        let result = verify_manifest(&manifest).unwrap();
        assert!(!result.is_clean());
        assert_eq!(result.missing, 1);
    }

    #[test]
    fn test_manifest_save_load_roundtrip() {
        let dir = tempdir().unwrap();
        let manifest_path = dir.path().join("proof.json");

        let entries = vec![ProofEntry {
            source_path: "/a.txt".to_string(),
            dest_path: "/out/a.txt".to_string(),
            size: 42,
            blake3_hash: "testhash".to_string(),
            exported_at: Utc::now(),
            bad_sector_notes: Some("2 bad blocks zero-filled".to_string()),
            verified: true,
        }];

        let custody = ChainOfCustody {
            operator: "edge@test".to_string(),
            machine: "test-rig".to_string(),
            os: "windows amd64".to_string(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            options_used: BTreeMap::from([
                ("verify_hash".to_string(), "true".to_string()),
                ("preserve_structure".to_string(), "true".to_string()),
            ]),
        };

        let manifest = build_manifest(Path::new("/source"), Path::new("/dest"), entries, custody);

        // Save
        save_manifest(&manifest, &manifest_path).unwrap();

        // Load
        let loaded = load_manifest(&manifest_path).unwrap();

        assert_eq!(loaded.version, PROOF_VERSION);
        assert_eq!(loaded.root_hash, manifest.root_hash);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].blake3_hash, "testhash");
        assert_eq!(loaded.chain_of_custody.operator, "edge@test");
        assert_eq!(loaded.chain_of_custody.options_used.len(), 2);
    }

    #[test]
    fn test_format_verify_result() {
        let result = VerifyResult {
            total: 10,
            verified: 8,
            failed: 1,
            missing: 1,
            tampered: vec![TamperInfo {
                path: "/out/bad.txt".to_string(),
                expected_hash: "expected".to_string(),
                actual_hash: "actual".to_string(),
                issue: TamperType::HashMismatch,
            }],
            root_hash_valid: false,
            expected_root_hash: "expected_root".to_string(),
            computed_root_hash: "computed_root".to_string(),
        };

        let text = format_verify_result(&result);
        assert!(text.contains("VERIFICATION FAILED"));
        assert!(text.contains("Verified:       8"));
        assert!(text.contains("Failed:         1"));
        assert!(text.contains("HASH MISMATCH"));
        assert!(text.contains("/out/bad.txt"));
    }
}
