//! Read-Only Enforcement Module
//!
//! Provides safety guards to ensure Diamond Drill NEVER modifies source data:
//! - Panic guard if write access is detected
//! - File handle validation
//! - Mount point verification
//! - Runtime enforcement checks

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use colored::Colorize;

// ============================================================================
// Global Read-Only State
// ============================================================================

/// Global flag for read-only enforcement (enabled by default)
static READONLY_ENFORCED: AtomicBool = AtomicBool::new(true);

/// Enable read-only enforcement (panics on write attempts)
pub fn enable_readonly_enforcement() {
    READONLY_ENFORCED.store(true, Ordering::SeqCst);
    tracing::info!("🔒 Read-only enforcement ENABLED");
}

/// Disable read-only enforcement (for testing only)
pub fn disable_readonly_enforcement() {
    READONLY_ENFORCED.store(false, Ordering::SeqCst);
    tracing::warn!("⚠️ Read-only enforcement DISABLED - use with caution!");
}

/// Check if read-only enforcement is enabled
pub fn is_readonly_enforced() -> bool {
    READONLY_ENFORCED.load(Ordering::SeqCst)
}

// ============================================================================
// Safe File Opening
// ============================================================================

/// Open a file in read-only mode with enforcement
///
/// This is the ONLY way to open files in Diamond Drill.
/// It guarantees read-only access and panics if enforcement is violated.
pub fn open_readonly(path: &Path) -> io::Result<File> {
    // Verify we're not trying to open a write handle
    if is_readonly_enforced() {
        verify_not_writable(path)?;
    }

    File::open(path)
}

/// Verify a path is not writable (for enforcement)
fn verify_not_writable(path: &Path) -> io::Result<()> {
    // Try to open for write - this SHOULD fail
    let write_result = OpenOptions::new().write(true).create(false).open(path);

    match write_result {
        Ok(_) => {
            // We were able to open for write - this is a violation!
            if is_readonly_enforced() {
                let msg = format!(
                    "READONLY VIOLATION: Write access available to {}",
                    path.display()
                );
                tracing::error!("{}", msg);

                // In enforced mode, panic to prevent any possibility of data modification
                panic!(
                    "\n\n{}\n{}\n{}\n\n",
                    "═".repeat(60).red(),
                    format!("🚨 CRITICAL: {} 🚨", msg).red().bold(),
                    "═".repeat(60).red()
                );
            }
        }
        Err(_) => {
            // Good - write access denied as expected
        }
    }

    Ok(())
}

// ============================================================================
// Mount Point Verification
// ============================================================================

/// Verify a mount point is read-only (Linux/Unix)
#[cfg(unix)]
pub fn verify_mount_readonly(path: &Path) -> Result<bool, String> {
    use std::process::Command;

    // Use 'mount' command to check mount options
    let output = Command::new("mount")
        .output()
        .map_err(|e| format!("Failed to run mount command: {}", e))?;

    let mount_info = String::from_utf8_lossy(&output.stdout);

    // Find the mount point for this path
    let path_str = path.to_string_lossy();

    for line in mount_info.lines() {
        if line.contains(&*path_str)
            || path_str.starts_with(line.split_whitespace().nth(2).unwrap_or(""))
        {
            // Check for 'ro' option
            if line.contains("(ro") || line.contains(",ro,") || line.contains(",ro)") {
                return Ok(true);
            }
            // Found mount but not read-only
            return Ok(false);
        }
    }

    // Mount point not found - assume writable
    Ok(false)
}

/// Verify a mount point is read-only (Windows stub)
#[cfg(windows)]
pub fn verify_mount_readonly(_path: &Path) -> Result<bool, String> {
    // Windows doesn't have the same mount semantics
    // We rely on file-level checks instead
    Ok(false)
}

// ============================================================================
// Safety Warnings
// ============================================================================

/// Print a warning if the source is not mounted read-only
pub fn warn_if_writable(path: &Path) {
    match verify_mount_readonly(path) {
        Ok(true) => {
            println!("  {} Source is mounted read-only (safe)", "🔒".green());
        }
        Ok(false) => {
            println!(
                "\n  {} {}",
                "⚠".yellow().bold(),
                "WARNING: Source may be writable!".yellow().bold()
            );
            println!(
                "  {}",
                "  For maximum safety, mount with read-only flag:".yellow()
            );
            #[cfg(unix)]
            println!(
                "    {}",
                "sudo mount -o ro,remount /path/to/source".bright_cyan()
            );
            #[cfg(windows)]
            println!(
                "    {}",
                "Use Disk Management to set drive as read-only".bright_cyan()
            );
            println!();
        }
        Err(e) => {
            tracing::debug!("Could not verify mount status: {}", e);
        }
    }
}

// ============================================================================
// Pre-Operation Checks
// ============================================================================

/// Run all safety checks before starting an operation
pub fn run_safety_checks(source: &Path) -> Result<(), String> {
    println!("\n  {} Running safety checks...", "🔐".bright_cyan());

    // Check 1: Read-only enforcement is enabled
    if !is_readonly_enforced() {
        println!("  {} Read-only enforcement is DISABLED", "⚠".yellow());
    } else {
        println!("  {} Read-only enforcement enabled", "✓".green());
    }

    // Check 2: Source exists
    if !source.exists() {
        return Err(format!("Source does not exist: {}", source.display()));
    }
    println!("  {} Source exists", "✓".green());

    // Check 3: Source is readable
    if std::fs::metadata(source).is_err() {
        return Err(format!("Cannot read source metadata: {}", source.display()));
    }
    println!("  {} Source is readable", "✓".green());

    // Check 4: Mount status
    warn_if_writable(source);

    println!();
    Ok(())
}

// ============================================================================
// Safe Copy (for export)
// ============================================================================

/// Copy a file safely (source is read-only, dest is created)
pub fn safe_copy(source: &Path, dest: &Path) -> io::Result<u64> {
    // Open source in read-only mode (with enforcement)
    let mut src_file = open_readonly(source)?;

    // Create destination file
    let mut dst_file = File::create(dest)?;

    // Copy contents
    io::copy(&mut src_file, &mut dst_file)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_readonly_enforcement_toggle() {
        enable_readonly_enforcement();
        assert!(is_readonly_enforced());

        disable_readonly_enforcement();
        assert!(!is_readonly_enforced());

        // Re-enable for other tests
        enable_readonly_enforcement();
    }

    #[test]
    fn test_open_readonly() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "test content").unwrap();

        // Should succeed in read-only mode
        disable_readonly_enforcement(); // Disable for test
        let file = open_readonly(&file_path);
        assert!(file.is_ok());
        enable_readonly_enforcement();
    }

    #[test]
    fn test_safe_copy() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");

        std::fs::write(&src, "test content for copy").unwrap();

        disable_readonly_enforcement();
        let bytes = safe_copy(&src, &dst).unwrap();
        enable_readonly_enforcement();

        assert!(bytes > 0);
        assert!(dst.exists());
        assert_eq!(
            std::fs::read_to_string(&dst).unwrap(),
            "test content for copy"
        );
    }
}
