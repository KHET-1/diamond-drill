//! App state - Central state management for the TUI

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use super::file_tree::FileTree;
use crate::badsector::SectorMap;
use crate::cli::TuiArgs;
use crate::dedup::{DedupOptions, DedupReport};

/// Current view/tab in the TUI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Files,
    Search,
    Export,
    Dedup,
    BadSectors,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[
            Tab::Files,
            Tab::Search,
            Tab::Export,
            Tab::Dedup,
            Tab::BadSectors,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tab::Files => " Files ",
            Tab::Search => " Search ",
            Tab::Export => " Export ",
            Tab::Dedup => " Dedup ",
            Tab::BadSectors => " BadSectors ",
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Tab::Files => 0,
            Tab::Search => 1,
            Tab::Export => 2,
            Tab::Dedup => 3,
            Tab::BadSectors => 4,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Tab::Files => Tab::Search,
            Tab::Search => Tab::Export,
            Tab::Export => Tab::Dedup,
            Tab::Dedup => Tab::BadSectors,
            Tab::BadSectors => Tab::Files,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Tab::Files => Tab::BadSectors,
            Tab::Search => Tab::Files,
            Tab::Export => Tab::Search,
            Tab::Dedup => Tab::Export,
            Tab::BadSectors => Tab::Dedup,
        }
    }
}

/// Application state machine phases
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    /// Initial state, no source loaded
    Init,
    /// Indexing in progress
    Indexing,
    /// Browsing files
    Browse,
    /// Typing in the search/filter bar
    SearchInput,
}

/// Main application state
pub struct App {
    /// Current app phase
    pub state: AppState,
    /// Current tab
    pub tab: Tab,
    /// Should the app quit
    pub should_quit: bool,
    /// Show help overlay
    pub show_help: bool,
    /// Source path
    pub source: Option<PathBuf>,
    /// File tree
    pub file_tree: FileTree,
    /// Total file count (before filtering)
    pub file_count: usize,
    /// Selected file paths for export
    pub selected_files: Vec<String>,
    /// Filter/search input text
    pub filter: String,
    /// Status bar message
    pub status_message: String,
    /// Index progress (0.0 - 1.0)
    pub index_progress: f64,
    /// Dedup report (populated on demand)
    pub dedup_report: Option<DedupReport>,
    /// Dedup scroll offset for rendering
    pub dedup_scroll: usize,
    /// Bad sector data (populated on demand)
    ///
    /// Requirements:
    /// - Backend running at http://localhost:8001
    /// - Ollama running at http://localhost:11434
    /// - pip install requests
    pub bad_sector_maps: Vec<SectorMap>,
    /// Bad sector scroll offset
    pub bad_sector_scroll: usize,
    /// Cached file entries for dedup operations
    pub cached_entries: Vec<crate::core::FileEntry>,
}

impl App {
    /// Create a new App state from CLI args
    pub async fn new(args: TuiArgs) -> Result<Self> {
        let (state, file_tree, file_count, source) = if let Some(path) = &args.source {
            if path.exists() {
                // In a real app, we'd start indexing here
                // For now, let's assume we browse whatever is there
                (AppState::Browse, FileTree::new(), 0, Some(path.clone()))
            } else {
                (AppState::Init, FileTree::new(), 0, None)
            }
        } else {
            (AppState::Init, FileTree::new(), 0, None)
        };

        Ok(Self {
            state,
            tab: Tab::Files,
            should_quit: false,
            show_help: false,
            source,
            file_tree,
            file_count,
            selected_files: Vec::new(),
            filter: String::new(),
            status_message: "Press '?' for help".to_string(),
            index_progress: 0.0,
            dedup_report: None,
            dedup_scroll: 0,
            bad_sector_maps: Vec::new(),
            bad_sector_scroll: 0,
            cached_entries: Vec::new(),
        })
    }

    /// Global key handler
    pub fn on_key(&mut self, key: KeyEvent) {
        if self.show_help {
            self.show_help = false;
            return;
        }

        match self.state {
            AppState::Browse => self.handle_browse_key(key),
            AppState::SearchInput => self.handle_search_key(key),
            _ => {
                if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                    self.should_quit = true;
                }
            }
        }
    }

    /// Key handler for main browse mode
    fn handle_browse_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,

            // Navigation
            KeyCode::Char('k') | KeyCode::Up => self.file_tree.select_prev(),
            KeyCode::Char('j') | KeyCode::Down => self.file_tree.select_next(),
            KeyCode::Char('g') => self.file_tree.select_first(),
            KeyCode::Char('G') => self.file_tree.select_last(),
            KeyCode::Home => self.file_tree.select_first(),
            KeyCode::End => self.file_tree.select_last(),
            KeyCode::PageUp => self.page_up(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::Char('h') | KeyCode::Left => self.file_tree.collapse(),
            KeyCode::Char('l') | KeyCode::Right => self.file_tree.expand(),

            // Selection
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_selection(),

            // Enter search mode
            KeyCode::Char('/') => {
                self.state = AppState::SearchInput;
                self.status_message = "Type to filter, Enter to confirm, Esc to cancel".to_string();
            }

            // Tab switching
            KeyCode::Tab => self.tab = self.tab.next(),
            KeyCode::BackTab => self.tab = self.tab.prev(),
            KeyCode::Char('1') => self.tab = Tab::Files,
            KeyCode::Char('2') => self.tab = Tab::Search,
            KeyCode::Char('3') => self.tab = Tab::Export,
            KeyCode::Char('4') => self.tab = Tab::Dedup,
            KeyCode::Char('5') => self.tab = Tab::BadSectors,

            // Select all / none / invert
            KeyCode::Char('a') => self.select_all(),
            KeyCode::Char('n') => self.select_none(),
            KeyCode::Char('i') => self.invert_selection(),

            // Document "Touching"
            KeyCode::Char('o') => self.open_selected(),
            KeyCode::Char('r') => self.reveal_selected(),

            // Dedup: 'd' to run analysis (or refresh)
            KeyCode::Char('d') => {
                if self.tab == Tab::Dedup {
                    self.run_dedup_analysis();
                }
            }

            // Bad sectors: 'b' to scan
            KeyCode::Char('b') => {
                if self.tab == Tab::BadSectors {
                    self.run_badsector_scan();
                }
            }

            // Scroll for dedup / bad sector tabs
            KeyCode::Char('[') => match self.tab {
                Tab::Dedup => self.dedup_scroll = self.dedup_scroll.saturating_sub(5),
                Tab::BadSectors => {
                    self.bad_sector_scroll = self.bad_sector_scroll.saturating_sub(5)
                }
                _ => {}
            },
            KeyCode::Char(']') => match self.tab {
                Tab::Dedup => self.dedup_scroll = self.dedup_scroll.saturating_add(5),
                Tab::BadSectors => {
                    self.bad_sector_scroll = self.bad_sector_scroll.saturating_add(5)
                }
                _ => {}
            },

            // Help overlay
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.show_help = true;
            }

            _ => {}
        }
    }

    /// Handle keys in search input mode
    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.state = AppState::Browse;
                self.file_tree.apply_filter(&self.filter);
                self.status_message = format!("Filter: '{}'", self.filter);
            }
            KeyCode::Esc => {
                self.state = AppState::Browse;
                self.filter.clear();
                self.file_tree.clear_filter();
                self.status_message = "Filter cleared".to_string();
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.file_tree.apply_filter(&self.filter);
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.file_tree.apply_filter(&self.filter);
            }
            _ => {}
        }
    }

    /// Move up one page
    fn page_up(&mut self) {
        for _ in 0..20 {
            self.file_tree.select_prev();
        }
    }

    /// Move down one page
    fn page_down(&mut self) {
        for _ in 0..20 {
            self.file_tree.select_next();
        }
    }

    /// Invert selection
    fn invert_selection(&mut self) {
        let visible: Vec<String> = self
            .file_tree
            .visible_nodes()
            .iter()
            .map(|n| n.path.clone())
            .collect();
        let mut new_selection = Vec::new();
        for path in visible {
            if !self.selected_files.contains(&path) {
                new_selection.push(path);
            }
        }
        self.selected_files = new_selection;
        self.status_message = format!("{} files selected (inverted)", self.selected_files.len());
    }

    /// Open current file in system default viewer
    fn open_selected(&mut self) {
        if let Some(path) = self.file_tree.selected_path() {
            let path_obj = std::path::Path::new(&path);
            if path_obj.exists() {
                if let Err(e) = opener::open(path_obj) {
                    self.status_message = format!("Failed to open: {}", e);
                } else {
                    self.status_message = format!(
                        "Opened: {}",
                        path_obj.file_name().unwrap_or_default().to_string_lossy()
                    );
                }
            } else {
                self.status_message = "File does not exist on disk".to_string();
            }
        }
    }

    /// Reveal current file in system explorer
    fn reveal_selected(&mut self) {
        if let Some(path) = self.file_tree.selected_path() {
            let path_obj = std::path::Path::new(&path);
            if path_obj.exists() {
                let to_reveal = if path_obj.is_dir() {
                    path_obj.to_path_buf()
                } else {
                    path_obj.parent().unwrap_or(path_obj).to_path_buf()
                };

                if let Err(e) = opener::open(to_reveal) {
                    self.status_message = format!("Failed to reveal: {}", e);
                } else {
                    self.status_message = "Revealed in explorer".to_string();
                }
            }
        }
    }

    /// Toggle selection of current file
    fn toggle_selection(&mut self) {
        if let Some(path) = self.file_tree.selected_path() {
            if let Some(pos) = self.selected_files.iter().position(|p| p == &path) {
                self.selected_files.remove(pos);
            } else {
                self.selected_files.push(path);
            }
            self.status_message = format!("{} files selected", self.selected_files.len());
        }
    }

    /// Select all visible files
    pub fn select_all(&mut self) {
        for node in self.file_tree.visible_nodes() {
            if !self.selected_files.contains(&node.path) {
                self.selected_files.push(node.path.clone());
            }
        }
        self.status_message = format!("All {} files selected", self.selected_files.len());
    }

    /// Clear selection
    pub fn select_none(&mut self) {
        self.selected_files.clear();
        self.status_message = "Selection cleared".to_string();
    }

    /// Run dedup analysis on cached entries
    pub fn run_dedup_analysis(&mut self) {
        if self.cached_entries.is_empty() {
            self.status_message = "No indexed files — index a source first".to_string();
            return;
        }

        self.status_message = "Running dedup analysis...".to_string();

        let options = DedupOptions {
            strategy: crate::dedup::KeepStrategy::Newest,
            fuzzy: true,
            fuzzy_threshold: 80,
            min_size: 1,
        };

        match crate::dedup::analyze(&self.cached_entries, &options) {
            Ok(report) => {
                self.status_message = format!(
                    "Dedup: {} groups, {} duplicates, {} wasted",
                    report.duplicate_groups,
                    report.total_duplicates,
                    humansize::format_size(report.wasted_bytes, humansize::BINARY),
                );
                self.dedup_report = Some(report);
                self.dedup_scroll = 0;
            }
            Err(e) => {
                self.status_message = format!("Dedup error: {}", e);
            }
        }
    }

    /// Run bad sector scan on a sample of cached files
    pub fn run_badsector_scan(&mut self) {
        if self.cached_entries.is_empty() {
            self.status_message = "No indexed files — index a source first".to_string();
            return;
        }

        self.status_message = "Scanning for bad sectors...".to_string();

        let reader = crate::badsector::SectorReader::new();
        let mut maps = Vec::new();
        let mut scanned = 0usize;
        let mut bad_files = 0usize;

        // Scan first 100 files (or all, whichever is smaller)
        let limit = self.cached_entries.len().min(100);
        for entry in &self.cached_entries[..limit] {
            match reader.read_with_sector_tracking(&entry.path) {
                Ok(map) => {
                    scanned += 1;
                    if map.has_bad_sectors() {
                        bad_files += 1;
                        maps.push(map);
                    }
                }
                Err(_) => {
                    // Permission denied or file moved — skip silently
                }
            }
        }

        self.bad_sector_maps = maps;
        self.bad_sector_scroll = 0;
        self.status_message = format!(
            "Bad sectors: scanned {} files, {} with errors",
            scanned, bad_files,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn make_test_args(source: Option<PathBuf>) -> TuiArgs {
        TuiArgs { source }
    }

    #[tokio::test]
    async fn test_app_state_transitions() {
        let mut app = App::new(make_test_args(None)).await.unwrap();
        assert_eq!(app.state, AppState::Init);
        assert_eq!(app.tab, Tab::Files);

        app.tab = app.tab.next();
        assert_eq!(app.tab, Tab::Search);

        app.tab = app.tab.next();
        assert_eq!(app.tab, Tab::Export);

        app.tab = app.tab.next();
        assert_eq!(app.tab, Tab::Dedup);

        app.tab = app.tab.next();
        assert_eq!(app.tab, Tab::BadSectors);

        app.tab = app.tab.next();
        assert_eq!(app.tab, Tab::Files);

        app.tab = app.tab.prev();
        assert_eq!(app.tab, Tab::BadSectors);
    }

    #[tokio::test]
    async fn test_keybinding_quit() {
        let mut app = App::new(make_test_args(None)).await.unwrap();
        assert!(!app.should_quit);

        app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_file_selection_via_tree() {
        let mut app = App::new(make_test_args(Some(PathBuf::from("."))))
            .await
            .unwrap();

        // Populate file tree
        let paths = vec!["a.txt".to_string(), "b.txt".to_string()];
        app.file_tree = super::super::file_tree::FileTree::from_paths(&paths);
        app.file_count = paths.len();

        // Toggle selection
        app.toggle_selection();
        assert_eq!(app.selected_files.len(), 1);

        // Move down, toggle again
        app.file_tree.select_next();
        app.toggle_selection();
        assert_eq!(app.selected_files.len(), 2);

        // Select none
        app.select_none();
        assert_eq!(app.selected_files.len(), 0);

        // Select all
        app.select_all();
        assert_eq!(app.selected_files.len(), 2);
    }

    #[tokio::test]
    async fn test_search_input_mode() {
        let mut app = App::new(make_test_args(Some(PathBuf::from("."))))
            .await
            .unwrap();
        app.state = AppState::Browse;

        let paths = vec![
            "photo.jpg".to_string(),
            "document.pdf".to_string(),
            "photo_2.jpg".to_string(),
        ];
        app.file_tree = super::super::file_tree::FileTree::from_paths(&paths);
        app.file_count = paths.len();

        // Enter search mode
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(app.state, AppState::SearchInput);

        // Type "photo"
        for c in "photo".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.filter, "photo");
        assert_eq!(app.file_tree.visible_count(), 2); // photo.jpg and photo_2.jpg

        // Confirm
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.state, AppState::Browse);
    }
}
