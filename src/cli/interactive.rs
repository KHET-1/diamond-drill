//! Interactive Mode - Full-featured TUI for power users
//!
//! Provides real-time filtering, preview, and selection capabilities.

use std::path::PathBuf;

use anyhow::Result;
use colored::Colorize;
use console::Term;
use dialoguer::{theme::ColorfulTheme, Confirm, FuzzySelect, Input};
use indicatif::{ProgressBar, ProgressStyle};

use crate::cli::InteractiveArgs;
use crate::core::DrillEngine;
use crate::export::ExportOptions;

/// Run interactive session
pub async fn run_interactive_session(args: &InteractiveArgs) -> Result<()> {
    let term = Term::stdout();
    term.clear_screen()?;

    print_interactive_banner();

    // Load or create session
    let mut session = InteractiveSession::new(args).await?;

    // Main loop
    loop {
        match session.state {
            SessionState::SelectSource => {
                session.select_source().await?;
            }
            SessionState::Indexing => {
                session.run_indexing().await?;
            }
            SessionState::Browse => {
                if !session.browse_files().await? {
                    break;
                }
            }
            SessionState::Search => {
                session.search_files().await?;
            }
            SessionState::Preview => {
                session.preview_selected().await?;
            }
            SessionState::Export => {
                session.export_files().await?;
            }
            SessionState::Exit => {
                break;
            }
        }
    }

    // Save session state if requested
    if session.save_state {
        session.persist_state().await?;
    }

    println!("\n{}\n", "Thanks for using Diamond Drill! 💎".bright_cyan());
    Ok(())
}

fn print_interactive_banner() {
    let banner = r#"
╔══════════════════════════════════════════════════════════════════════════════╗
║  💎 DIAMOND DRILL - Interactive Recovery Mode                                ║
║                                                                              ║
║  Commands: [/] Search  [f] Filter  [p] Preview  [x] Export  [q] Quit        ║
║            [Space] Select  [Enter] Confirm  [Tab] Next pane  [?] Help       ║
╚══════════════════════════════════════════════════════════════════════════════╝
"#;
    println!("{}", banner.bright_cyan());
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SessionState {
    SelectSource,
    Indexing,
    Browse,
    Search,
    Preview,
    Export,
    Exit,
}

struct InteractiveSession {
    _args: InteractiveArgs,
    state: SessionState,
    engine: Option<DrillEngine>,
    selected_files: Vec<String>,
    filter_pattern: String,
    current_directory: PathBuf,
    save_state: bool,
}

impl InteractiveSession {
    async fn new(args: &InteractiveArgs) -> Result<Self> {
        let initial_state = if args.source.is_some() {
            SessionState::Indexing
        } else {
            SessionState::SelectSource
        };

        Ok(Self {
            _args: args.clone(),
            state: initial_state,
            engine: None,
            selected_files: Vec::new(),
            filter_pattern: String::new(),
            current_directory: args.source.clone().unwrap_or_else(|| PathBuf::from(".")),
            save_state: false,
        })
    }

    async fn select_source(&mut self) -> Result<()> {
        println!(
            "\n{}\n",
            "Select a source to recover files from:".bright_yellow()
        );

        let source: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Source path")
            .interact_text()?;

        let path = PathBuf::from(&source);
        if !path.exists() {
            println!("{} Path does not exist: {}", "✗".bright_red(), source);
            return Ok(());
        }

        self.current_directory = path;
        self.state = SessionState::Indexing;
        Ok(())
    }

    async fn run_indexing(&mut self) -> Result<()> {
        println!(
            "\n{} Indexing: {}\n",
            "⚡".bright_yellow(),
            self.current_directory.display()
        );

        let engine = DrillEngine::new(self.current_directory.clone()).await?;

        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg} [{elapsed_precise}]")
                .unwrap(),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        pb.set_message("Scanning...");

        let args = crate::cli::IndexArgs {
            source: self.current_directory.clone(),
            resume: false,
            index_file: None,
            skip_hidden: false,
            depth: None,
            extensions: None,
            thumbnails: false,
            workers: None,
            checkpoint_interval: 1000,
            bad_sector_report: None,
            block_size: 4096,
        };

        engine.index_with_progress(&args).await?;

        let count = engine.file_count().await;
        pb.finish_with_message(format!(
            "{} Indexed {} files",
            "✓".bright_green(),
            count.to_string().bright_white()
        ));

        self.engine = Some(engine);
        self.state = SessionState::Browse;
        Ok(())
    }

    async fn browse_files(&mut self) -> Result<bool> {
        let engine = match &self.engine {
            Some(e) => e,
            None => {
                self.state = SessionState::SelectSource;
                return Ok(true);
            }
        };

        // Get files matching current filter
        let files = if self.filter_pattern.is_empty() {
            engine.get_all_files().await?
        } else {
            engine.search_fuzzy(&self.filter_pattern).await?
        };

        if files.is_empty() {
            println!("{}", "No files found.".yellow());
            self.state = SessionState::Search;
            return Ok(true);
        }

        // Show file browser
        println!(
            "\n{} {} files {}",
            "📁".bright_cyan(),
            files.len(),
            if self.filter_pattern.is_empty() {
                String::new()
            } else {
                format!("(filtered by '{}')", self.filter_pattern)
            }
        );

        // Build display items with type indicators
        let display_items: Vec<String> = files
            .iter()
            .take(50)
            .map(|f| {
                let icon = get_file_icon(f);
                let selected = if self.selected_files.contains(f) {
                    "[✓] "
                } else {
                    "[ ] "
                };
                format!("{}{} {}", selected, icon, f)
            })
            .collect();

        let options = [
            "──────────────────────────",
            "🔍 Search / Filter",
            "📋 Select All",
            "📋 Select None",
            "📤 Export Selected",
            "👁  Preview Selected",
            "🚪 Exit",
            "──────────────────────────",
        ];

        let all_items: Vec<&str> = options
            .iter()
            .copied()
            .chain(display_items.iter().map(|s| s.as_str()))
            .collect();

        let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Selected: {} | Filter: {}",
                self.selected_files.len(),
                if self.filter_pattern.is_empty() {
                    "<none>"
                } else {
                    &self.filter_pattern
                }
            ))
            .items(&all_items)
            .default(8) // First file
            .interact_opt()?;

        match selection {
            Some(1) => {
                // Search/Filter
                self.state = SessionState::Search;
            }
            Some(2) => {
                // Select All
                self.selected_files = files;
            }
            Some(3) => {
                // Select None
                self.selected_files.clear();
            }
            Some(4) => {
                // Export
                if self.selected_files.is_empty() {
                    println!("{}", "No files selected!".yellow());
                } else {
                    self.state = SessionState::Export;
                }
            }
            Some(5) => {
                // Preview
                self.state = SessionState::Preview;
            }
            Some(6) => {
                // Exit
                self.state = SessionState::Exit;
                return Ok(false);
            }
            Some(idx) if idx >= 8 => {
                // Toggle file selection
                let file_idx = idx - 8;
                if file_idx < files.len() {
                    let file = &files[file_idx];
                    if self.selected_files.contains(file) {
                        self.selected_files.retain(|f| f != file);
                    } else {
                        self.selected_files.push(file.clone());
                    }
                }
            }
            _ => {}
        }

        Ok(true)
    }

    async fn search_files(&mut self) -> Result<()> {
        println!("\n{}", "Search/Filter Files".bright_yellow().bold());
        println!("  Supports: glob (*.jpg), fuzzy (photo), extensions (.rs)\n");

        let pattern: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Pattern")
            .allow_empty(true)
            .with_initial_text(&self.filter_pattern)
            .interact_text()?;

        self.filter_pattern = pattern;
        self.state = SessionState::Browse;
        Ok(())
    }

    async fn preview_selected(&mut self) -> Result<()> {
        let engine = match &self.engine {
            Some(e) => e,
            None => {
                self.state = SessionState::Browse;
                return Ok(());
            }
        };

        if self.selected_files.is_empty() {
            println!("{}", "No files selected for preview.".yellow());
            self.state = SessionState::Browse;
            return Ok(());
        }

        println!(
            "\n{} Previewing {} files...\n",
            "👁".bright_cyan(),
            self.selected_files.len()
        );

        for file in &self.selected_files {
            let info = engine.get_file_info(file).await?;
            println!(
                "  {} {} ({}) - {}",
                get_file_icon(file),
                file.bright_white(),
                humansize::format_size(info.size, humansize::BINARY),
                info.modified
                    .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "Unknown".to_string())
            );
        }

        println!();
        self.state = SessionState::Browse;
        Ok(())
    }

    async fn export_files(&mut self) -> Result<()> {
        if self.selected_files.is_empty() {
            println!("{}", "No files selected!".yellow());
            self.state = SessionState::Browse;
            return Ok(());
        }

        let engine = match &self.engine {
            Some(e) => e,
            None => {
                self.state = SessionState::Browse;
                return Ok(());
            }
        };

        println!(
            "\n{} Export {} files\n",
            "📤".bright_yellow(),
            self.selected_files.len()
        );

        let dest: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Destination folder")
            .interact_text()?;

        let dest_path = PathBuf::from(&dest);

        let verify = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Verify file integrity with blake3 hash?")
            .default(true)
            .interact()?;

        let preserve = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Preserve directory structure?")
            .default(true)
            .interact()?;

        let options = ExportOptions {
            dest: dest_path,
            preserve_structure: preserve,
            verify_hash: verify,
            continue_on_error: true,
            create_manifest: true,
            ..Default::default()
        };

        let pb = ProgressBar::new(self.selected_files.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap(),
        );

        let result = engine
            .export_files_with_progress(&self.selected_files, &options, |p| {
                pb.set_position(p.completed as u64);
                pb.set_message(p.current_file.clone());
            })
            .await;

        pb.finish_with_message("Complete!");

        match result {
            Ok(stats) => {
                println!(
                    "\n{} Exported {} files ({} total)",
                    "✓".bright_green(),
                    stats.successful,
                    humansize::format_size(stats.total_bytes, humansize::BINARY)
                );
                if stats.failed > 0 {
                    println!("{} {} files failed", "⚠".yellow(), stats.failed);
                }
            }
            Err(e) => {
                println!("{} Export failed: {}", "✗".bright_red(), e);
            }
        }

        self.selected_files.clear();
        self.state = SessionState::Browse;
        Ok(())
    }

    async fn persist_state(&self) -> Result<()> {
        // Save session state to disk for resume
        // Implementation: serialize selected files, filter, etc.
        Ok(())
    }
}

/// Get emoji icon for file type
fn get_file_icon(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();

    match ext.as_str() {
        // Images
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tiff" | "raw" => "🖼 ",
        // Videos
        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => "🎬",
        // Audio
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "wma" => "🎵",
        // Documents
        "pdf" => "📕",
        "doc" | "docx" => "📘",
        "xls" | "xlsx" => "📗",
        "ppt" | "pptx" => "📙",
        "txt" | "md" | "rtf" => "📄",
        // Archives
        "zip" | "tar" | "gz" | "7z" | "rar" | "bz2" => "📦",
        // Code
        "rs" => "🦀",
        "py" => "🐍",
        "js" | "ts" => "📜",
        "html" | "css" => "🌐",
        "json" | "yaml" | "toml" => "⚙ ",
        // Executables
        "exe" | "dll" | "so" | "dylib" => "⚡",
        // Databases
        "db" | "sqlite" | "sql" => "🗃 ",
        // Others
        _ => "📄",
    }
}
