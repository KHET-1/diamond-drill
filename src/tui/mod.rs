//! TUI Module - Terminal User Interface powered by ratatui
//!
//! Full-featured terminal UI with file tree, vim keybindings,
//! search, export, dedup, and bad sector visualization.

mod app;
pub mod file_tree;
mod ui;

pub use app::{App, AppState};

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::time::Duration;

use crate::cli::TuiArgs;
use crate::core::DrillEngine;

/// Run the TUI application
pub async fn run_tui(args: TuiArgs) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(args.clone()).await?;

    // If source provided, index it
    if let Some(ref source) = args.source {
        app.status_message = format!("Indexing {}...", source.display());
        app.state = AppState::Indexing;

        // Index synchronously before entering the event loop
        let engine = DrillEngine::new(source.clone()).await?;
        let index_args = crate::cli::IndexArgs {
            source: source.clone(),
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
        engine.index_with_progress(&index_args).await?;

        // Populate file tree from indexed files
        let files = engine.get_all_files().await?;
        let paths: Vec<String> = files.iter().map(|p| p.to_string()).collect();
        app.file_tree = file_tree::FileTree::from_paths(&paths);
        app.file_count = paths.len();

        app.state = AppState::Browse;
        app.status_message = format!("Indexed {} files from {}", app.file_count, source.display());
    }

    // Run main loop
    let result = run_event_loop(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("TUI error: {}", e);
    }

    Ok(())
}

/// Main TUI event loop
fn run_event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Poll for events with timeout
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                app.on_key(key);
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
