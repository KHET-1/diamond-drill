//! GUI Application using iced
//!
//! Modern, responsive file recovery interface.

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{
    button, column, container, horizontal_space, pick_list, row, scrollable, text, text_input,
    vertical_space, Column, Container, Row,
};
use iced::{executor, Application, Command, Element, Length, Settings, Theme};
use parking_lot::RwLock;

use crate::cli::GuiArgs;
use crate::core::{DrillEngine, FileEntry, FileType};

/// Run the GUI application
pub fn run_gui(args: GuiArgs) -> anyhow::Result<()> {
    let (width, height) = parse_size(&args.size);

    DiamondDrillApp::run(Settings {
        window: iced::window::Settings {
            size: iced::Size::new(width as f32, height as f32),
            ..Default::default()
        },
        ..Default::default()
    })?;

    Ok(())
}

/// Parse window size from string (e.g., "1280x800")
fn parse_size(size: &str) -> (u32, u32) {
    let parts: Vec<&str> = size.split('x').collect();
    if parts.len() == 2 {
        let width = parts[0].parse().unwrap_or(1280);
        let height = parts[1].parse().unwrap_or(800);
        (width, height)
    } else {
        (1280, 800)
    }
}

/// Main application state
struct DiamondDrillApp {
    /// Current view
    view: AppView,
    /// Source path input
    source_input: String,
    /// Destination path input
    dest_input: String,
    /// Search filter
    filter_input: String,
    /// Engine instance
    engine: Option<Arc<RwLock<DrillEngine>>>,
    /// File list
    files: Vec<FileEntry>,
    /// Selected files
    selected: Vec<usize>,
    /// Status message
    status: String,
    /// Is loading
    loading: bool,
    /// Error message
    error: Option<String>,
    /// File type filter
    type_filter: Option<FileType>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppView {
    Source,
    Browse,
    Export,
}

#[derive(Debug, Clone)]
enum Message {
    // Navigation
    SetView(AppView),

    // Input
    SourceInputChanged(String),
    DestInputChanged(String),
    FilterInputChanged(String),

    // Actions
    SelectSource,
    StartIndex,
    IndexComplete(Result<Vec<FileEntry>, String>),
    ToggleSelect(usize),
    SelectAll,
    SelectNone,
    StartExport,
    ExportComplete(Result<usize, String>),

    // Filters
    SetTypeFilter(Option<FileType>),
    ApplyFilter,
}

impl Application for DiamondDrillApp {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        (
            Self {
                view: AppView::Source,
                source_input: String::new(),
                dest_input: String::new(),
                filter_input: String::new(),
                engine: None,
                files: Vec::new(),
                selected: Vec::new(),
                status: "Welcome to Diamond Drill! 💎".to_string(),
                loading: false,
                error: None,
                type_filter: None,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        String::from("💎 Diamond Drill - File Recovery")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::SetView(view) => {
                self.view = view;
            }
            Message::SourceInputChanged(value) => {
                self.source_input = value;
            }
            Message::DestInputChanged(value) => {
                self.dest_input = value;
            }
            Message::FilterInputChanged(value) => {
                self.filter_input = value;
            }
            Message::SelectSource => {
                // In a full implementation, this would open a file picker
                self.status = "Select a source path...".to_string();
            }
            Message::StartIndex => {
                if self.source_input.is_empty() {
                    self.error = Some("Please enter a source path".to_string());
                    return Command::none();
                }

                self.loading = true;
                self.status = "Indexing...".to_string();
                self.error = None;

                // Start indexing in background
                let source = self.source_input.clone();
                return Command::perform(
                    async move { index_source(source).await },
                    Message::IndexComplete,
                );
            }
            Message::IndexComplete(result) => {
                self.loading = false;
                match result {
                    Ok(files) => {
                        self.status = format!("Indexed {} files", files.len());
                        self.files = files;
                        self.view = AppView::Browse;
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
            }
            Message::ToggleSelect(idx) => {
                if self.selected.contains(&idx) {
                    self.selected.retain(|&i| i != idx);
                } else {
                    self.selected.push(idx);
                }
                self.status = format!("{} files selected", self.selected.len());
            }
            Message::SelectAll => {
                self.selected = (0..self.files.len()).collect();
                self.status = format!("{} files selected", self.selected.len());
            }
            Message::SelectNone => {
                self.selected.clear();
                self.status = "No files selected".to_string();
            }
            Message::StartExport => {
                if self.selected.is_empty() {
                    self.error = Some("No files selected".to_string());
                    return Command::none();
                }
                if self.dest_input.is_empty() {
                    self.error = Some("Please enter a destination path".to_string());
                    return Command::none();
                }

                self.loading = true;
                self.status = "Exporting...".to_string();
                self.view = AppView::Export;
            }
            Message::ExportComplete(result) => {
                self.loading = false;
                match result {
                    Ok(count) => {
                        self.status = format!("Exported {} files successfully!", count);
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
            }
            Message::SetTypeFilter(filter) => {
                self.type_filter = filter;
            }
            Message::ApplyFilter => {
                // Filter would be applied to files list
                self.status = format!(
                    "Filter: {}",
                    self.type_filter
                        .map(|t| format!("{:?}", t))
                        .unwrap_or_else(|| "All".to_string())
                );
            }
        }

        Command::none()
    }

    fn view(&self) -> Element<Message> {
        let content = match self.view {
            AppView::Source => self.view_source(),
            AppView::Browse => self.view_browse(),
            AppView::Export => self.view_export(),
        };

        // Main layout with header and content
        let header = self.view_header();

        let main = column![
            header,
            vertical_space().height(10),
            content,
            vertical_space().height(10),
            self.view_status(),
        ]
        .padding(20)
        .spacing(10);

        container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

impl DiamondDrillApp {
    fn view_header(&self) -> Element<Message> {
        let title = text("💎 Diamond Drill").size(28);

        let nav = row![
            button("📁 Source")
                .on_press(Message::SetView(AppView::Source))
                .padding(10),
            button("🔍 Browse")
                .on_press(Message::SetView(AppView::Browse))
                .padding(10),
            button("📤 Export")
                .on_press(Message::SetView(AppView::Export))
                .padding(10),
        ]
        .spacing(10);

        row![title, horizontal_space(), nav]
            .align_items(iced::Alignment::Center)
            .into()
    }

    fn view_source(&self) -> Element<Message> {
        let source_input = text_input("Enter source path (e.g., E:\\Backup)", &self.source_input)
            .on_input(Message::SourceInputChanged)
            .padding(10)
            .size(16);

        let browse_btn = button("Browse...")
            .on_press(Message::SelectSource)
            .padding(10);

        let start_btn = button("🚀 Start Indexing")
            .on_press(Message::StartIndex)
            .padding(15);

        column![
            text("Select Source").size(24),
            vertical_space().height(20),
            text("Enter the path to your backup drive, disk image, or folder:"),
            vertical_space().height(10),
            row![source_input, browse_btn].spacing(10),
            vertical_space().height(20),
            start_btn,
        ]
        .spacing(5)
        .into()
    }

    fn view_browse(&self) -> Element<Message> {
        let filter_input = text_input("Filter files...", &self.filter_input)
            .on_input(Message::FilterInputChanged)
            .padding(10);

        let select_btns = row![
            button("Select All").on_press(Message::SelectAll).padding(8),
            button("Select None")
                .on_press(Message::SelectNone)
                .padding(8),
        ]
        .spacing(10);

        let file_list: Element<Message> = if self.files.is_empty() {
            text("No files indexed yet. Select a source first.").into()
        } else {
            let items: Vec<Element<Message>> = self
                .files
                .iter()
                .enumerate()
                .take(100) // Limit displayed items
                .map(|(idx, entry)| {
                    let selected = self.selected.contains(&idx);
                    let icon = entry.file_type.icon();
                    let name = entry.name();
                    let size = humansize::format_size(entry.size, humansize::BINARY);

                    let checkbox = if selected { "☑" } else { "☐" };

                    button(text(format!("{} {} {} ({})", checkbox, icon, name, size)))
                        .on_press(Message::ToggleSelect(idx))
                        .width(Length::Fill)
                        .padding(5)
                        .into()
                })
                .collect();

            scrollable(Column::with_children(items).spacing(2))
                .height(Length::Fill)
                .into()
        };

        column![
            text("Browse Files").size(24),
            row![filter_input, select_btns].spacing(10),
            vertical_space().height(10),
            text(format!(
                "{} files | {} selected",
                self.files.len(),
                self.selected.len()
            )),
            file_list,
        ]
        .spacing(10)
        .into()
    }

    fn view_export(&self) -> Element<Message> {
        let dest_input = text_input("Enter destination path", &self.dest_input)
            .on_input(Message::DestInputChanged)
            .padding(10);

        let export_btn = button("📤 Export Selected Files")
            .on_press(Message::StartExport)
            .padding(15);

        let summary = if self.selected.is_empty() {
            text("No files selected")
        } else {
            let total_size: u64 = self
                .selected
                .iter()
                .filter_map(|&idx| self.files.get(idx))
                .map(|e| e.size)
                .sum();

            text(format!(
                "{} files selected ({})",
                self.selected.len(),
                humansize::format_size(total_size, humansize::BINARY)
            ))
        };

        column![
            text("Export Files").size(24),
            vertical_space().height(20),
            summary,
            vertical_space().height(10),
            text("Destination folder:"),
            dest_input,
            vertical_space().height(20),
            export_btn,
        ]
        .spacing(5)
        .into()
    }

    fn view_status(&self) -> Element<Message> {
        let status_text = if self.loading {
            text(format!("⏳ {}", self.status))
        } else if let Some(ref error) = self.error {
            text(format!("❌ {}", error))
        } else {
            text(format!("✓ {}", self.status))
        };

        container(status_text)
            .padding(10)
            .width(Length::Fill)
            .into()
    }
}

/// Index a source path (runs in background)
async fn index_source(source: String) -> Result<Vec<FileEntry>, String> {
    let path = PathBuf::from(&source);

    if !path.exists() {
        return Err(format!("Path does not exist: {}", source));
    }

    // Create engine and index
    let engine = DrillEngine::new(path.clone())
        .await
        .map_err(|e| e.to_string())?;

    let args = crate::cli::IndexArgs {
        source: path,
        resume: false,
        index_file: None,
        skip_hidden: true,
        depth: None,
        extensions: None,
        thumbnails: false,
        workers: None,
        checkpoint_interval: 1000,
        bad_sector_report: None,
        block_size: 4096,
    };

    engine
        .index_with_progress(&args)
        .await
        .map_err(|e| e.to_string())?;

    // Get all files
    let files = engine.get_all_files().await.map_err(|e| e.to_string())?;

    // Convert to entries (simplified)
    let entries: Vec<FileEntry> = files
        .iter()
        .filter_map(|path| {
            let p = PathBuf::from(path);
            let meta = std::fs::metadata(&p).ok()?;
            Some(FileEntry::new(p, &meta))
        })
        .collect();

    Ok(entries)
}
