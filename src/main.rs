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
