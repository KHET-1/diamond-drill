//! Diamond Drill - Ultra-fast offline disk image recovery tool
//!
//! Build a CLI-first, optionally GUI-enabled tool that indexes, previews,
//! searches, selects and exports files from disk images/clones with extreme
//! speed and safety.

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use diamond_drill::cli::{self, Cli, Commands};
use diamond_drill::core::DrillEngine;
#[cfg(feature = "gui")]
use diamond_drill::gui;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer().with_target(false).compact())
        .with(EnvFilter::from_default_env().add_directive("diamond_drill=info".parse()?))
        .init();

    let cli = Cli::parse();

    // Handle grandma mode - simplified interactive workflow
    if cli.easy {
        return cli::easy_mode::run_easy_mode().await;
    }

    match cli.command {
        Some(Commands::Index(args)) => {
            let engine = DrillEngine::new(args.source.clone()).await?;
            engine.index_with_progress(&args).await?;
        }
        Some(Commands::Search(args)) => {
            let engine = DrillEngine::load_or_create(&args.source).await?;
            engine.search_interactive(&args).await?;
        }
        Some(Commands::Preview(args)) => {
            let engine = DrillEngine::load_or_create(&args.source).await?;
            engine.preview_files(&args).await?;
        }
        Some(Commands::Export(args)) => {
            let engine = DrillEngine::load_or_create(&args.source).await?;
            engine.export_selected(&args).await?;
        }
        Some(Commands::Carve(args)) => {
            run_carve(args).await?;
        }
        Some(Commands::Interactive(args)) => {
            cli::interactive::run_interactive_session(&args).await?;
        }
        Some(Commands::Dedup(args)) => {
            let engine = DrillEngine::load_or_create(&args.source).await?;
            engine.run_dedup(&args).await?;
        }
        Some(Commands::Verify(args)) => {
            use diamond_drill::proof;

            println!("Diamond Drill Proof Verification");
            println!("Loading manifest: {}\n", args.manifest.display());

            let manifest = proof::load_manifest(&args.manifest)?;
            println!(
                "Manifest: {} files, {} total bytes, root_hash={}",
                manifest.total_files,
                humansize::format_size(manifest.total_bytes, humansize::BINARY),
                &manifest.root_hash[..16]
            );
            println!("Operator: {}\n", manifest.chain_of_custody.operator);

            let result = proof::verify_manifest(&manifest)?;

            match args.report {
                cli::VerifyReportFormat::Human => {
                    print!("{}", proof::format_verify_result(&result));
                }
                cli::VerifyReportFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
            }

            if !result.is_clean() {
                std::process::exit(1);
            }
        }
        Some(Commands::Swarm(args)) => {
            use diamond_drill::swarm;

            println!("Diamond Drill Swarm Pipeline");
            println!("Source: {}\n", args.source.display());

            let mut config = swarm::SwarmConfig::new(args.source.clone());
            config.heal.max_retries = args.max_retries;
            config.skip_hidden = args.skip_hidden;
            config.chunk_size = args.chunk_size;
            config.chunk_overlap = args.chunk_overlap;

            if let Some(ref exts) = args.extensions {
                config.extensions = Some(exts.clone());
            }
            if let Some(ref output) = args.output {
                config.output = Some(output.clone());
            }
            if args.silent_heal {
                config.heal.silent_heal = true;
            }
            if let Some(ref log) = args.heal_log {
                config.heal.log_path = Some(log.clone());
            }
            if args.gpu_fallback {
                config.heal.enable_gpu_fallback = true;
            }

            let result = swarm::run_swarm_with_config(config)?;
            println!(
                "Swarm complete: {} files, {} errors",
                result.files_scanned, result.errors_encountered
            );
        }
        Some(Commands::Tui(args)) => {
            diamond_drill::tui::run_tui(args).await?;
        }
        #[cfg(feature = "gui")]
        Some(Commands::Gui(args)) => {
            gui::run_gui(args)?;
        }
        None => {
            // Default: run interactive mode
            cli::interactive::run_interactive_session(&cli::InteractiveArgs::default()).await?;
        }
    }

    Ok(())
}

async fn run_carve(args: cli::CarveArgs) -> Result<()> {
    use colored::Colorize;
    use diamond_drill::carve::{CarveOptions, CarveProgress, Carver};
    use indicatif::{ProgressBar, ProgressStyle};

    let min_size = parse_size_str(&args.min_size).unwrap_or(512);

    let file_types = args.file_type.map(|filters| {
        filters
            .into_iter()
            .filter_map(|ft| match ft {
                cli::FileTypeFilter::Image => Some(diamond_drill::core::FileType::Image),
                cli::FileTypeFilter::Video => Some(diamond_drill::core::FileType::Video),
                cli::FileTypeFilter::Audio => Some(diamond_drill::core::FileType::Audio),
                cli::FileTypeFilter::Document => Some(diamond_drill::core::FileType::Document),
                cli::FileTypeFilter::Archive => Some(diamond_drill::core::FileType::Archive),
                cli::FileTypeFilter::Code => Some(diamond_drill::core::FileType::Code),
                cli::FileTypeFilter::All => None,
            })
            .collect()
    });

    let image_size = std::fs::metadata(&args.source).map(|m| m.len()).unwrap_or(0);

    let opts = CarveOptions {
        source: args.source.clone(),
        output_dir: args.output.clone(),
        sector_aligned: args.sector_aligned,
        min_size,
        file_types,
        workers: args.workers.unwrap_or_else(num_cpus::get),
        dry_run: args.dry_run,
        verify: !args.no_verify,
    };

    let json_output = matches!(args.output_format, Some(cli::OutputFormat::Json));

    if !json_output {
        println!(
            "\n{} Carving files from: {}",
            "💎".bright_cyan(),
            args.source.display().to_string().bright_white()
        );
        println!(
            "   Output: {}  |  Mode: {}  |  Image: {}",
            args.output.display().to_string().bright_white(),
            if args.dry_run { "dry run" } else { "extract" },
            humansize::format_size(image_size, humansize::BINARY),
        );
    }

    let pb = if !json_output {
        let pb = ProgressBar::new(image_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) {msg}",
                )
                .unwrap()
                .progress_chars("█▓▒░"),
        );
        pb.set_message("Scanning...");
        Some(pb)
    } else {
        None
    };

    let carver = Carver::new(opts);
    let (carved, result) = carver
        .carve_with_progress(|progress| {
            match progress {
                CarveProgress::ScanComplete { headers_found } => {
                    if let Some(ref pb) = pb {
                        pb.finish_with_message(format!("Scan done: {} headers", headers_found));
                    }
                }
                CarveProgress::Extracting { current, total, ref extension } => {
                    if let Some(ref pb) = pb {
                        if current == 1 {
                            pb.reset();
                            pb.set_length(total as u64);
                            pb.set_style(
                                ProgressStyle::default_bar()
                                    .template(
                                        "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}",
                                    )
                                    .unwrap()
                                    .progress_chars("█▓▒░"),
                            );
                        }
                        pb.set_position(current as u64);
                        pb.set_message(format!("Extracting .{}", extension));
                    }
                }
                CarveProgress::Done => {
                    if let Some(ref pb) = pb {
                        pb.finish_and_clear();
                    }
                }
                _ => {}
            }
        })
        .await?;

    if json_output {
        let output = serde_json::json!({
            "files_found": result.files_found,
            "files_extracted": result.files_extracted,
            "files_verified": result.files_verified,
            "files_failed": result.files_failed,
            "total_bytes_extracted": result.total_bytes_extracted,
            "image_size": result.image_size,
            "duration_ms": result.duration_ms,
            "by_type": result.by_type,
            "files": carved,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("\n{}", "═".repeat(60).bright_cyan());
    println!(
        "  {} {} files found, {} extracted",
        "✓".bright_green().bold(),
        result.files_found,
        result.files_extracted,
    );
    if result.files_verified > 0 {
        println!("  {} {} verified by content type", "✓".bright_green(), result.files_verified);
    }
    if result.files_failed > 0 {
        println!("  {} {} failed", "⚠".yellow(), result.files_failed);
    }
    println!(
        "  {} Total extracted: {}",
        "📊",
        humansize::format_size(result.total_bytes_extracted, humansize::BINARY)
    );
    if result.duration_ms > 0 {
        let speed = result.image_size * 1000 / result.duration_ms.max(1);
        println!(
            "  {} {:.1}s | {}/s",
            "⏱ ",
            result.duration_ms as f64 / 1000.0,
            humansize::format_size(speed, humansize::BINARY),
        );
    }
    if !result.by_type.is_empty() {
        println!("\n  By type:");
        let mut types: Vec<_> = result.by_type.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1));
        for (ext, count) in types {
            println!("    {} .{}: {}", "•".bright_cyan(), ext, count);
        }
    }
    println!("{}", "═".repeat(60).bright_cyan());
    Ok(())
}

fn parse_size_str(s: &str) -> Option<u64> {
    let s = s.trim().to_uppercase();
    let (num, unit) = if s.ends_with("GB") {
        (&s[..s.len() - 2], 1024u64 * 1024 * 1024)
    } else if s.ends_with("MB") {
        (&s[..s.len() - 2], 1024u64 * 1024)
    } else if s.ends_with("KB") {
        (&s[..s.len() - 2], 1024u64)
    } else if s.ends_with('B') {
        (&s[..s.len() - 1], 1u64)
    } else {
        (s.as_str(), 1u64)
    };
    num.trim().parse::<u64>().ok().map(|n| n * unit)
}
