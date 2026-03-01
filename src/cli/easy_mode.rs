//! Easy Mode - Grandma-friendly simplified workflow
//!
//! Step-by-step guided recovery with minimal technical jargon.
//! Includes auto-detection for:
//! - Loop-mounted disk images
//! - ISO/IMG files
//! - Connected USB drives
//! - Network shares

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::Colorize;
use console::Term;
use dialoguer::{theme::ColorfulTheme, Confirm, FuzzySelect, Input, MultiSelect};
use indicatif::{ProgressBar, ProgressStyle};

use crate::core::DrillEngine;
use crate::export::ExportOptions;

// ============================================================================
// Detected Source Types
// ============================================================================

/// Detected source type for smart handling
#[derive(Debug, Clone)]
pub enum DetectedSource {
    /// Regular directory
    Directory(PathBuf),
    /// Disk image file (.img, .iso, .dmg, .raw, .dd)
    DiskImage(PathBuf),
    /// Loop-mounted device (/dev/loop*)
    LoopMount {
        device: String,
        mount_point: PathBuf,
    },
    /// USB/External drive
    ExternalDrive {
        label: String,
        path: PathBuf,
        size: u64,
    },
    /// Network share
    NetworkShare(PathBuf),
}

impl DetectedSource {
    /// Get display label for the source
    pub fn label(&self) -> String {
        match self {
            DetectedSource::Directory(p) => format!("📁 {}", p.display()),
            DetectedSource::DiskImage(p) => format!("💿 {} (disk image)", p.display()),
            DetectedSource::LoopMount {
                device,
                mount_point,
            } => {
                format!("🔁 {} mounted at {}", device, mount_point.display())
            }
            DetectedSource::ExternalDrive { label, size, .. } => {
                format!(
                    "💾 {} ({})",
                    label,
                    humansize::format_size(*size, humansize::BINARY)
                )
            }
            DetectedSource::NetworkShare(p) => format!("🌐 {} (network)", p.display()),
        }
    }

    /// Get the actual path to use
    pub fn path(&self) -> &Path {
        match self {
            DetectedSource::Directory(p) => p,
            DetectedSource::DiskImage(p) => p,
            DetectedSource::LoopMount { mount_point, .. } => mount_point,
            DetectedSource::ExternalDrive { path, .. } => path,
            DetectedSource::NetworkShare(p) => p,
        }
    }

    /// Check if this is a disk image that needs mounting
    pub fn needs_mount(&self) -> bool {
        matches!(self, DetectedSource::DiskImage(_))
    }
}

/// Recovery scenario — determines scan behavior
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecoveryScenario {
    /// Accidentally deleted files
    DeletedFiles,
    /// Corrupted drive or filesystem errors
    CorruptedDrive,
    /// Lost photos / camera recovery
    LostPhotos,
    /// Old backup drive — general browse
    BackupDrive,
    /// Full scan — everything on the device
    ScanEverything,
}

impl RecoveryScenario {
    /// Get IndexArgs overrides for this scenario
    pub fn scan_config(&self) -> (bool, Option<Vec<String>>, Option<usize>) {
        // Returns (skip_hidden, extensions_filter, max_depth)
        match self {
            RecoveryScenario::DeletedFiles => (false, None, None), // scan hidden too
            RecoveryScenario::CorruptedDrive => (false, None, None), // full scan
            RecoveryScenario::LostPhotos => (
                true,
                Some(vec![
                    "jpg".into(),
                    "jpeg".into(),
                    "png".into(),
                    "gif".into(),
                    "heic".into(),
                    "heif".into(),
                    "raw".into(),
                    "cr2".into(),
                    "nef".into(),
                    "arw".into(),
                    "dng".into(),
                    "mp4".into(),
                    "mov".into(),
                    "avi".into(),
                ]),
                None,
            ),
            RecoveryScenario::BackupDrive => (true, None, Some(10)),
            RecoveryScenario::ScanEverything => (false, None, None),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RecoveryScenario::DeletedFiles => "I accidentally deleted files",
            RecoveryScenario::CorruptedDrive => "My drive is corrupted / has errors",
            RecoveryScenario::LostPhotos => "I lost photos from a camera/phone",
            RecoveryScenario::BackupDrive => "I have an old backup drive to browse",
            RecoveryScenario::ScanEverything => "Scan everything on the device",
        }
    }
}

/// Run the easy mode interactive workflow
pub async fn run_easy_mode() -> Result<()> {
    // Loop instead of recursion to avoid stack overflow on repeated retries
    loop {
        let term = Term::stdout();
        term.clear_screen()?;

        print_banner();

        println!(
            "\n{}\n",
            "Welcome to Diamond Drill Easy Mode! 💎"
                .bright_cyan()
                .bold()
        );
        println!("I'll guide you through recovering your files step by step.\n");

        // Step 0: What happened?
        let scenario = step_what_happened()?;

        // Step 1: Select source
        let source = step_select_source()?;

        // Step 2: Index the source (with scenario-aware config)
        let engine = step_index_source(&source, scenario).await?;

        // Step 3: Find files
        let selected_files = step_find_files(&engine).await?;

        if selected_files.is_empty() {
            println!("\n{}", "No files selected. Exiting.".yellow());
            return Ok(());
        }

        // Step 4: Select destination
        let dest = step_select_destination()?;

        // Step 5: Export
        let export_result = step_export_files(&engine, &selected_files, &dest).await?;

        // Auto-generate HTML report if export succeeded and manifest was created
        if let Some(ref manifest_path) = export_result.manifest_path {
            let report_path = dest.join("diamond-drill-report.html");
            match crate::report::report_data_from_manifest(manifest_path) {
                Ok(report_data) => {
                    // Don't use save_html_report (which opens browser) — just write file
                    let html = crate::report::generate_html_report(&report_data);
                    if let Err(e) = std::fs::write(&report_path, html.as_bytes()) {
                        tracing::warn!("Could not write recovery report: {}", e);
                    } else {
                        println!(
                            "  {} Recovery report: {}",
                            "📊".bright_cyan(),
                            report_path.display().to_string().bright_white()
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("Could not generate report: {}", e);
                }
            }
        }

        println!(
            "\n{} {}",
            "✓".bright_green().bold(),
            "Recovery complete! Your files are safe.".bright_green()
        );

        // Step 6: Satisfaction check — returns true if user wants to run again
        if !step_satisfaction_check_should_retry().await? {
            return Ok(());
        }
    }
}

/// Step 0: Ask what happened to guide scanning
fn step_what_happened() -> Result<RecoveryScenario> {
    println!("{} What happened?", "First:".bright_yellow().bold());
    println!("  Tell me about your situation so I can help better.\n");

    let scenarios = [
        RecoveryScenario::ScanEverything,
        RecoveryScenario::DeletedFiles,
        RecoveryScenario::CorruptedDrive,
        RecoveryScenario::LostPhotos,
        RecoveryScenario::BackupDrive,
    ];

    let labels: Vec<&str> = scenarios.iter().map(|s| s.label()).collect();

    let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select your situation")
        .items(&labels)
        .default(0)
        .interact()?;

    let scenario = scenarios[selection];

    println!(
        "\n{} Got it! I'll optimize the scan for: {}\n",
        "✓".bright_green(),
        scenario.label().bright_white()
    );

    Ok(scenario)
}

fn print_banner() {
    let banner = r#"
    ╔═══════════════════════════════════════════════════════════╗
    ║                                                           ║
    ║     💎  D I A M O N D   D R I L L  💎                    ║
    ║                                                           ║
    ║          E A S Y   M O D E                                ║
    ║                                                           ║
    ╚═══════════════════════════════════════════════════════════╝
    "#;
    println!("{}", banner.bright_cyan());
}

fn step_select_source() -> Result<PathBuf> {
    loop {
        println!("{} Where are your files?", "Step 1:".bright_yellow().bold());
        println!("  This could be:");
        println!("  • A backup drive (like E:\\)");
        println!("  • A folder on your computer");
        println!("  • A disk image file (.img, .iso)\n");

        // Auto-detect available sources
        let detected = auto_detect_sources();
        if !detected.is_empty() {
            print_detected_sources(&detected);
        }

        let mut options = vec![
            "🔍 Auto-detected source (see above)".to_string(),
            "📁 Browse for a folder...".to_string(),
            "⌨️  Type a path manually".to_string(),
            "💾 Use a connected drive".to_string(),
            "💿 Open a disk image file".to_string(),
        ];

        // Disable auto-detect option if nothing was found
        if detected.is_empty() {
            options[0] = "🔍 (No sources auto-detected)".to_string();
        }

        let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
            .with_prompt("How would you like to select the source?")
            .items(&options)
            .default(if detected.is_empty() { 1 } else { 0 })
            .interact()?;

        let path: Option<PathBuf> = match selection {
            0 if !detected.is_empty() => {
                // Select from auto-detected sources
                let labels: Vec<String> = detected.iter().map(|s| s.label()).collect();
                let idx = FuzzySelect::with_theme(&ColorfulTheme::default())
                    .with_prompt("Select a detected source")
                    .items(&labels)
                    .interact()?;

                let source = &detected[idx];

                // Warn if disk image needs mounting
                if source.needs_mount() {
                    println!(
                        "\n{} {}",
                        "⚠".yellow(),
                        "Disk image detected! For best results, mount it first:".yellow()
                    );
                    #[cfg(target_os = "linux")]
                    println!(
                        "    sudo losetup -r /dev/loop0 {}",
                        source.path().display()
                    );
                    #[cfg(target_os = "linux")]
                    println!("    sudo mount -o ro /dev/loop0 /mnt/image");
                    #[cfg(target_os = "windows")]
                    println!("    Right-click the file > Mount (or use disk management)");
                    println!();

                    if !Confirm::with_theme(&ColorfulTheme::default())
                        .with_prompt(
                            "Continue with unmounted image? (slower, limited features)",
                        )
                        .default(true)
                        .interact()?
                    {
                        continue; // retry source selection
                    }
                }

                Some(source.path().to_path_buf())
            }
            0 => {
                // No auto-detected sources, retry
                println!(
                    "{}",
                    "No sources detected. Please select another option.".yellow()
                );
                continue;
            }
            1 => {
                // Browse for folder
                Some(
                    Input::with_theme(&ColorfulTheme::default())
                        .with_prompt("Enter the folder path")
                        .validate_with(|input: &String| {
                            let p = PathBuf::from(input);
                            if p.exists() {
                                Ok(())
                            } else {
                                Err("Path does not exist")
                            }
                        })
                        .interact_text()?
                        .into(),
                )
            }
            2 => {
                // Type path manually
                Some(
                    Input::with_theme(&ColorfulTheme::default())
                        .with_prompt("Enter the full path")
                        .validate_with(|input: &String| {
                            let p = PathBuf::from(input);
                            if p.exists() {
                                Ok(())
                            } else {
                                Err("Path does not exist")
                            }
                        })
                        .interact_text()?
                        .into(),
                )
            }
            3 => {
                // List available drives
                let drives = list_available_drives();
                if drives.is_empty() {
                    println!("{}", "No additional drives found.".yellow());
                    continue;
                }
                let drive_idx = FuzzySelect::with_theme(&ColorfulTheme::default())
                    .with_prompt("Select a drive")
                    .items(&drives)
                    .interact()?;
                Some(PathBuf::from(&drives[drive_idx]))
            }
            4 => {
                // Open disk image file
                let path_str: String = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Enter path to disk image (.img, .iso, .dmg)")
                    .validate_with(|input: &String| {
                        let p = PathBuf::from(input);
                        if !p.exists() {
                            return Err("File does not exist");
                        }
                        if !is_disk_image(&p) {
                            return Err("Not a recognized disk image format");
                        }
                        Ok(())
                    })
                    .interact_text()?;

                let p = PathBuf::from(&path_str);

                // Show disk image info
                if let Some(info) = get_disk_image_info(&p) {
                    println!("\n  {} {}", "💿".bright_cyan(), info);
                }

                println!(
                    "\n{} {}",
                    "💡".bright_yellow(),
                    "Tip: Mount the image for faster scanning:".bright_yellow()
                );
                #[cfg(target_os = "linux")]
                {
                    println!("    sudo losetup -r /dev/loop0 {}", p.display());
                    println!("    sudo mount -o ro /dev/loop0 /mnt/image");
                }
                #[cfg(target_os = "windows")]
                println!("    Right-click > Mount, or use Disk Management");
                println!();

                Some(p)
            }
            _ => {
                // Shouldn't happen, but loop around
                continue;
            }
        };

        if let Some(path) = path {
            println!(
                "\n{} Selected: {}\n",
                "✓".bright_green(),
                path.display().to_string().bright_white()
            );
            return Ok(path);
        }
    }
}

async fn step_index_source(
    source: &std::path::Path,
    scenario: RecoveryScenario,
) -> Result<DrillEngine> {
    println!(
        "{} Scanning your files...",
        "Step 2:".bright_yellow().bold()
    );
    println!("  This might take a moment for large drives.\n");

    let engine = DrillEngine::new(source.to_path_buf()).await?;

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .expect("valid progress bar template"),
    );
    pb.set_message("Scanning files...");
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // Apply scenario-aware configuration
    let (skip_hidden, extensions, depth) = scenario.scan_config();

    let args = crate::cli::IndexArgs {
        source: source.to_path_buf(),
        resume: false,
        index_file: None,
        skip_hidden,
        depth,
        extensions,
        thumbnails: false,
        workers: None,
        checkpoint_interval: 1000,
        bad_sector_report: None,
        block_size: 4096,
    };

    engine.index_with_progress(&args).await?;

    pb.finish_with_message(format!(
        "{} Found {} files",
        "✓".bright_green(),
        engine.file_count().await
    ));

    println!();
    Ok(engine)
}

async fn step_find_files(engine: &DrillEngine) -> Result<Vec<String>> {
    loop {
        println!(
            "{} What files do you want to recover?",
            "Step 3:".bright_yellow().bold()
        );

        let options = [
            "📁 Everything (scan all files)",
            "📷 Photos & Images",
            "🎬 Videos",
            "🎵 Music & Audio",
            "📄 Documents (PDF, Word, etc.)",
            "🔍 Search by name...",
        ];

        let selections = MultiSelect::with_theme(&ColorfulTheme::default())
            .with_prompt("Select file types (Space to toggle, Enter to confirm)")
            .items(&options)
            .interact()?;

        if selections.is_empty() {
            println!("{}", "No selection made.".yellow());
            return Ok(vec![]);
        }

        let mut files = Vec::new();

        for selection in selections {
            match selection {
                0 => files.extend(engine.get_all_files().await?),
                1 => files.extend(engine.get_files_by_type("image").await?),
                2 => files.extend(engine.get_files_by_type("video").await?),
                3 => files.extend(engine.get_files_by_type("audio").await?),
                4 => files.extend(engine.get_files_by_type("document").await?),
                5 => {
                    let pattern: String = Input::with_theme(&ColorfulTheme::default())
                        .with_prompt("Enter search term")
                        .interact_text()?;
                    files.extend(engine.search_fuzzy(&pattern).await?);
                }
                _ => {}
            }
        }

        // Deduplicate
        files.sort();
        files.dedup();

        println!(
            "\n{} Found {} files matching your criteria",
            "✓".bright_green(),
            files.len()
        );

        // Preview count by type
        let summary = engine.summarize_files(&files).await?;
        for (file_type, count) in summary {
            println!("  {} {} {}", "•".bright_cyan(), count, file_type);
        }

        // Confirm selection
        if Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Proceed with these files?")
            .default(true)
            .interact()?
        {
            return Ok(files);
        }
        // Otherwise loop back and let them re-select
    }
}

fn step_select_destination() -> Result<PathBuf> {
    loop {
        println!(
            "\n{} Where should I save the recovered files?",
            "Step 4:".bright_yellow().bold()
        );

        let dest: PathBuf = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter destination folder")
            .with_initial_text(
                dirs::document_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("Recovered Files")
                    .to_string_lossy()
                    .to_string(),
            )
            .interact_text()?
            .into();

        // Create if doesn't exist
        if !dest.exists() {
            if Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!("Folder doesn't exist. Create {}?", dest.display()))
                .default(true)
                .interact()?
            {
                std::fs::create_dir_all(&dest)
                    .context("Failed to create destination folder")?;
            } else {
                continue; // retry destination selection
            }
        }

        println!(
            "\n{} Saving to: {}\n",
            "✓".bright_green(),
            dest.display().to_string().bright_white()
        );
        return Ok(dest);
    }
}

async fn step_export_files(
    engine: &DrillEngine,
    files: &[String],
    dest: &std::path::Path,
) -> Result<crate::export::ExportResult> {
    println!(
        "{} Recovering your files...",
        "Step 5:".bright_yellow().bold()
    );

    let options = ExportOptions {
        dest: dest.to_path_buf(),
        preserve_structure: true,
        verify_hash: true,
        continue_on_error: true,
        create_manifest: true,
        ..Default::default()
    };

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )
            .expect("valid progress bar template")
            .progress_chars("█▓▒░"),
    );

    let result = engine
        .export_files_with_progress(files, &options, |progress| {
            pb.set_position(progress.completed as u64);
            pb.set_message(progress.current_file.clone());
        })
        .await;

    pb.finish_with_message("Done!");

    match result {
        Ok(stats) => {
            println!("\n{}", "═".repeat(50).bright_cyan());
            println!(
                "  {} {} files recovered successfully",
                "✓".bright_green().bold(),
                stats.successful
            );
            if stats.failed > 0 {
                println!(
                    "  {} {} files had errors (see log)",
                    "⚠".yellow(),
                    stats.failed
                );
            }
            println!(
                "  {} Total size: {}",
                "📊".bright_cyan(),
                humansize::format_size(stats.total_bytes, humansize::BINARY)
            );
            if let Some(ref manifest) = stats.manifest_path {
                println!(
                    "  {} Manifest saved: {}",
                    "📋".bright_cyan(),
                    manifest.display()
                );
            }
            println!("{}", "═".repeat(50).bright_cyan());
            Ok(stats)
        }
        Err(e) => {
            println!("\n{} Some errors occurred: {}", "⚠".yellow().bold(), e);
            Err(e)
        }
    }
}

/// Step 6: Post-recovery satisfaction check — returns true if user wants to retry
async fn step_satisfaction_check_should_retry() -> Result<bool> {
    println!("\n{} Did it work?", "Step 6:".bright_yellow().bold());

    let satisfied = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Did you find the files you were looking for?")
        .default(true)
        .interact()?;

    if satisfied {
        println!(
            "\n{} {}",
            "✓".bright_green().bold(),
            "Great! Your files are safe now.".bright_green()
        );
        println!();
        println!("  {} Next steps:", "💡".bright_yellow());
        println!("    • Keep a backup of the recovered files in a second location");
        println!(
            "    • Use {} to scan for duplicates",
            "diamond-drill dedup <path> --fuzzy".bright_cyan()
        );
        println!(
            "    • Use {} for an interactive browser",
            "diamond-drill tui <path>".bright_cyan()
        );
        println!(
            "\n  {} Thank you for using Diamond Drill!",
            "💎".bright_cyan()
        );
        println!(
            "  {}",
            "100% local. 100% private. 100% free.".bright_white()
        );
        println!();

        // Pause so the user actually sees the message
        Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Press Enter to exit")
            .allow_empty(true)
            .default(String::new())
            .show_default(false)
            .interact_text()?;
        Ok(false) // don't retry
    } else {
        println!(
            "\n{} {}",
            "💡".bright_yellow(),
            "Don't worry — here are some things to try:".bright_yellow()
        );
        println!("  1. Run again with \"Scan Everything\" to check for more files");
        println!("  2. Try a different source path (another partition or drive)");
        println!("  3. Use the search feature to look for files by name");
        println!("  4. If the drive is corrupted, try the Bad Sector mode:");
        println!(
            "     {}",
            "diamond-drill index <source> --bad-sector-report report.json".bright_cyan()
        );
        println!("  5. For duplicate detection:");
        println!(
            "     {}",
            "diamond-drill dedup <source> --fuzzy".bright_cyan()
        );

        let retry = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Would you like to run Easy Mode again?")
            .default(false)
            .interact()?;

        Ok(retry)
    }
}

/// List available drives (Windows-specific, stub for other platforms)
fn list_available_drives() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        let mut drives = Vec::new();
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            if PathBuf::from(&drive).exists() {
                drives.push(drive);
            }
        }
        drives
    }

    #[cfg(not(target_os = "windows"))]
    {
        // On Unix, list mounted volumes
        // /media/<user>/<drive> needs two-level traversal
        let mut drives = vec![String::from("/")];
        for base in &["/media", "/mnt", "/run/media"] {
            if let Ok(entries) = std::fs::read_dir(base) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Check if this is a mount point directly (e.g., /mnt/data)
                        // or a user dir containing mounts (e.g., /media/rathin/)
                        let mut has_subdirs = false;
                        if let Ok(sub_entries) = std::fs::read_dir(&path) {
                            for sub in sub_entries.flatten() {
                                if sub.path().is_dir() {
                                    has_subdirs = true;
                                    drives.push(sub.path().to_string_lossy().to_string());
                                }
                            }
                        }
                        // If no subdirs, it's likely a direct mount point
                        if !has_subdirs {
                            drives.push(path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        drives
    }
}

// ============================================================================
// Auto-Detection Functions
// ============================================================================

/// Auto-detect all available sources (drives, disk images, loop mounts)
pub fn auto_detect_sources() -> Vec<DetectedSource> {
    let mut sources = Vec::new();

    // Detect regular drives
    for drive in list_available_drives() {
        let path = PathBuf::from(&drive);
        if path.is_dir() {
            sources.push(DetectedSource::Directory(path));
        }
    }

    // Detect disk images in common locations
    sources.extend(detect_disk_images());

    // Detect loop mounts (Linux)
    #[cfg(target_os = "linux")]
    sources.extend(detect_loop_mounts());

    sources
}

/// Detect disk image files in common locations
fn detect_disk_images() -> Vec<DetectedSource> {
    let mut images = Vec::new();

    let image_extensions = [
        "img", "iso", "dmg", "raw", "dd", "bin", "vhd", "vhdx", "vmdk",
    ];

    // Check common directories
    let search_dirs = [
        dirs::home_dir(),
        dirs::download_dir(),
        dirs::document_dir(),
        Some(PathBuf::from(".")),
    ];

    for dir_opt in search_dirs.into_iter().flatten() {
        if let Ok(entries) = std::fs::read_dir(&dir_opt) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if image_extensions.contains(&ext.to_lowercase().as_str()) {
                        // Check file size to filter out tiny files
                        if let Ok(meta) = std::fs::metadata(&path) {
                            if meta.len() > 1_000_000 {
                                // > 1MB
                                images.push(DetectedSource::DiskImage(path));
                            }
                        }
                    }
                }
            }
        }
    }

    images
}

/// Detect loop-mounted devices (Linux only)
#[cfg(target_os = "linux")]
fn detect_loop_mounts() -> Vec<DetectedSource> {
    let mut mounts = Vec::new();

    // Read /proc/mounts to find loop devices
    if let Ok(content) = std::fs::read_to_string("/proc/mounts") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0].starts_with("/dev/loop") {
                let device = parts[0].to_string();
                let mount_point = PathBuf::from(parts[1]);
                if mount_point.exists() {
                    mounts.push(DetectedSource::LoopMount {
                        device,
                        mount_point,
                    });
                }
            }
        }
    }

    mounts
}

/// Check if a path is a disk image
pub fn is_disk_image(path: &std::path::Path) -> bool {
    let image_extensions = [
        "img", "iso", "dmg", "raw", "dd", "bin", "vhd", "vhdx", "vmdk",
    ];

    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| image_extensions.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Get disk image info
pub fn get_disk_image_info(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let size = humansize::format_size(meta.len(), humansize::BINARY);
    let ext = path.extension()?.to_str()?;

    let format = match ext.to_lowercase().as_str() {
        "iso" => "ISO 9660 CD/DVD image",
        "img" | "raw" | "dd" => "Raw disk image",
        "dmg" => "macOS disk image",
        "vhd" | "vhdx" => "Virtual Hard Disk",
        "vmdk" => "VMware disk",
        _ => "Disk image",
    };

    Some(format!(
        "{} - {} ({})",
        path.file_name()?.to_str()?,
        format,
        size
    ))
}

/// Print detected sources for user selection
pub fn print_detected_sources(sources: &[DetectedSource]) {
    if sources.is_empty() {
        println!("  {}", "No sources auto-detected.".yellow());
        return;
    }

    println!("  {} Detected sources:", "💎".bright_cyan());
    for (i, source) in sources.iter().enumerate() {
        println!("    {}. {}", i + 1, source.label());
    }
    println!();
}
