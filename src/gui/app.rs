//! Diamond Drill GUI — Modern dark-themed file recovery interface
//!
//! 5-view layout: Source → Browse → Carve → Export → Stats
//! Built on Iced 0.12 with the Elm architecture.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, progress_bar, row, scrollable,
    text, text_input, vertical_space, Column, Row,
};
use iced::{executor, Application, Command, Element, Length, Settings, Theme};
use parking_lot::RwLock;

use crate::carve::{CarveOptions, CarveResult, CarvedFile, Carver};
use crate::cli::GuiArgs;
use crate::core::{DrillEngine, FileEntry, FileType};
use crate::export::{ExportOptions, Exporter};

pub fn run_gui(args: GuiArgs) -> anyhow::Result<()> {
    let (width, height) = parse_size(&args.size);

    DiamondDrillApp::run(Settings {
        window: iced::window::Settings {
            size: iced::Size::new(width as f32, height as f32),
            ..Default::default()
        },
        default_font: iced::Font::DEFAULT,
        ..Default::default()
    })?;

    Ok(())
}

fn parse_size(size: &str) -> (u32, u32) {
    let parts: Vec<&str> = size.split('x').collect();
    if parts.len() == 2 {
        (
            parts[0].parse().unwrap_or(1280),
            parts[1].parse().unwrap_or(800),
        )
    } else {
        (1280, 800)
    }
}

// ── State ────────────────────────────────────────────────────────────────

struct DiamondDrillApp {
    view: AppView,
    source_input: String,
    dest_input: String,
    carve_source_input: String,
    carve_output_input: String,
    filter_input: String,
    engine: Option<Arc<RwLock<DrillEngine>>>,
    files: Vec<FileEntry>,
    filtered_indices: Vec<usize>,
    selected: Vec<usize>,
    carved_files: Vec<CarvedFile>,
    carve_result: Option<CarveResult>,
    type_filter: Option<FileType>,
    status: String,
    loading: bool,
    progress: f32,
    progress_label: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppView {
    Source,
    Browse,
    Carve,
    Export,
    Stats,
}

impl AppView {
    fn label(&self) -> &'static str {
        match self {
            AppView::Source => "Source",
            AppView::Browse => "Browse",
            AppView::Carve => "Carve",
            AppView::Export => "Export",
            AppView::Stats => "Stats",
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            AppView::Source => "📁",
            AppView::Browse => "🔍",
            AppView::Carve => "💎",
            AppView::Export => "📤",
            AppView::Stats => "📊",
        }
    }
}

const ALL_VIEWS: [AppView; 5] = [
    AppView::Source,
    AppView::Browse,
    AppView::Carve,
    AppView::Export,
    AppView::Stats,
];

// ── Messages ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    SetView(AppView),
    SourceInputChanged(String),
    DestInputChanged(String),
    CarveSourceChanged(String),
    CarveOutputChanged(String),
    FilterInputChanged(String),

    BrowseSource,
    BrowseDest,
    BrowseCarveSource,
    BrowseCarveOutput,

    StartIndex,
    IndexComplete(Result<Vec<FileEntry>, String>),

    ToggleSelect(usize),
    SelectAll,
    SelectNone,
    SetTypeFilter(Option<FileType>),

    StartExport,
    ExportComplete(Result<usize, String>),

    StartCarve,
    CarveComplete(Result<(Vec<CarvedFile>, CarveResult), String>),

    DismissError,
}

// ── Application ──────────────────────────────────────────────────────────

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
                carve_source_input: String::new(),
                carve_output_input: String::new(),
                filter_input: String::new(),
                engine: None,
                files: Vec::new(),
                filtered_indices: Vec::new(),
                selected: Vec::new(),
                carved_files: Vec::new(),
                carve_result: None,
                type_filter: None,
                status: "Ready — select a source to begin".to_string(),
                loading: false,
                progress: 0.0,
                progress_label: String::new(),
                error: None,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        format!(
            "Diamond Drill — {} | {}",
            self.view.label(),
            self.status
        )
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            // ── Navigation ───────────────────────────────
            Message::SetView(view) => {
                self.view = view;
                self.error = None;
            }

            // ── Text inputs ──────────────────────────────
            Message::SourceInputChanged(v) => self.source_input = v,
            Message::DestInputChanged(v) => self.dest_input = v,
            Message::CarveSourceChanged(v) => self.carve_source_input = v,
            Message::CarveOutputChanged(v) => self.carve_output_input = v,

            Message::FilterInputChanged(v) => {
                self.filter_input = v.clone();
                self.rebuild_filter(&v);
            }

            // ── File picker dialogs ──────────────────────
            Message::BrowseSource => {
                #[cfg(feature = "gui")]
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select source folder or image")
                    .pick_folder()
                {
                    self.source_input = path.to_string_lossy().to_string();
                }
            }
            Message::BrowseDest => {
                #[cfg(feature = "gui")]
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select destination folder")
                    .pick_folder()
                {
                    self.dest_input = path.to_string_lossy().to_string();
                }
            }
            Message::BrowseCarveSource => {
                #[cfg(feature = "gui")]
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select raw disk image")
                    .add_filter("Disk images", &["img", "dd", "raw", "iso", "bin"])
                    .add_filter("All files", &["*"])
                    .pick_file()
                {
                    self.carve_source_input = path.to_string_lossy().to_string();
                }
            }
            Message::BrowseCarveOutput => {
                #[cfg(feature = "gui")]
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Select output folder for carved files")
                    .pick_folder()
                {
                    self.carve_output_input = path.to_string_lossy().to_string();
                }
            }

            // ── Indexing ─────────────────────────────────
            Message::StartIndex => {
                if self.source_input.is_empty() {
                    self.error = Some("Enter a source path first".into());
                    return Command::none();
                }
                self.loading = true;
                self.progress = 0.0;
                self.progress_label = "Indexing files...".into();
                self.error = None;
                self.status = "Indexing...".into();
                let source = self.source_input.clone();
                return Command::perform(
                    async move { index_source(source).await },
                    Message::IndexComplete,
                );
            }
            Message::IndexComplete(result) => {
                self.loading = false;
                self.progress = 1.0;
                match result {
                    Ok(files) => {
                        let count = files.len();
                        self.filtered_indices = (0..count).collect();
                        self.files = files;
                        self.status = format!("Indexed {} files", count);
                        self.view = AppView::Browse;
                    }
                    Err(e) => self.error = Some(e),
                }
            }

            // ── Selection ────────────────────────────────
            Message::ToggleSelect(idx) => {
                if let Some(pos) = self.selected.iter().position(|&i| i == idx) {
                    self.selected.remove(pos);
                } else {
                    self.selected.push(idx);
                }
                self.status = format!("{} files selected", self.selected.len());
            }
            Message::SelectAll => {
                self.selected = self.filtered_indices.clone();
                self.status = format!("{} files selected", self.selected.len());
            }
            Message::SelectNone => {
                self.selected.clear();
                self.status = "Selection cleared".into();
            }
            Message::SetTypeFilter(ft) => {
                self.type_filter = ft;
                self.rebuild_filter(&self.filter_input.clone());
            }

            // ── Export ───────────────────────────────────
            Message::StartExport => {
                if self.selected.is_empty() {
                    self.error = Some("No files selected".into());
                    return Command::none();
                }
                if self.dest_input.is_empty() {
                    self.error = Some("Enter a destination path".into());
                    return Command::none();
                }
                self.loading = true;
                self.progress = 0.0;
                self.progress_label = "Exporting...".into();
                self.error = None;

                let entries: Vec<FileEntry> = self
                    .selected
                    .iter()
                    .filter_map(|&i| self.files.get(i).cloned())
                    .collect();
                let dest = self.dest_input.clone();
                return Command::perform(
                    async move { run_export(entries, dest).await },
                    Message::ExportComplete,
                );
            }
            Message::ExportComplete(result) => {
                self.loading = false;
                self.progress = 1.0;
                match result {
                    Ok(count) => {
                        self.status = format!("Exported {} files", count);
                        self.view = AppView::Stats;
                    }
                    Err(e) => self.error = Some(e),
                }
            }

            // ── Carving ──────────────────────────────────
            Message::StartCarve => {
                if self.carve_source_input.is_empty() {
                    self.error = Some("Enter a disk image path".into());
                    return Command::none();
                }
                if self.carve_output_input.is_empty() {
                    self.error = Some("Enter an output folder".into());
                    return Command::none();
                }
                self.loading = true;
                self.progress = 0.0;
                self.progress_label = "Carving...".into();
                self.error = None;
                self.status = "Scanning image for file signatures...".into();

                let source = self.carve_source_input.clone();
                let output = self.carve_output_input.clone();
                return Command::perform(
                    async move { run_carve(source, output).await },
                    Message::CarveComplete,
                );
            }
            Message::CarveComplete(result) => {
                self.loading = false;
                self.progress = 1.0;
                match result {
                    Ok((carved, stats)) => {
                        self.status = format!(
                            "Carved {} files ({})",
                            stats.files_extracted,
                            humansize::format_size(stats.total_bytes_extracted, humansize::BINARY),
                        );
                        self.carved_files = carved;
                        self.carve_result = Some(stats);
                        self.view = AppView::Stats;
                    }
                    Err(e) => self.error = Some(e),
                }
            }

            Message::DismissError => self.error = None,
        }

        Command::none()
    }

    fn view(&self) -> Element<Message> {
        let sidebar = self.view_sidebar();
        let content = match self.view {
            AppView::Source => self.view_source(),
            AppView::Browse => self.view_browse(),
            AppView::Carve => self.view_carve(),
            AppView::Export => self.view_export(),
            AppView::Stats => self.view_stats(),
        };

        let main_area = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(24);

        let layout = row![sidebar, main_area];

        let mut page = column![layout];

        if self.loading {
            page = page.push(self.view_progress_bar());
        }

        if let Some(ref err) = self.error {
            page = page.push(self.view_error_banner(err));
        }

        page = page.push(self.view_status_bar());

        container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

// ── View builders ────────────────────────────────────────────────────────

impl DiamondDrillApp {
    fn view_sidebar(&self) -> Element<Message> {
        let title = column![
            text("💎").size(32),
            text("Diamond").size(14),
            text("Drill").size(14),
        ]
        .align_items(iced::Alignment::Center)
        .spacing(2);

        let mut nav = Column::new().spacing(4).padding(8).width(Length::Fixed(72.0));
        nav = nav.push(title);
        nav = nav.push(vertical_space().height(16));

        for &view in &ALL_VIEWS {
            let is_active = self.view == view;
            let label = column![
                text(view.icon()).size(20),
                text(view.label()).size(11),
            ]
            .align_items(iced::Alignment::Center)
            .spacing(2);

            let btn = button(container(label).center_x().width(Length::Fill))
                .on_press(Message::SetView(view))
                .width(Length::Fill)
                .padding(8);

            nav = nav.push(btn);
        }

        nav = nav.push(vertical_space());

        let file_count = text(format!("{}", self.files.len())).size(11);
        let sel_count = text(format!("sel: {}", self.selected.len())).size(11);
        nav = nav.push(file_count);
        nav = nav.push(sel_count);

        container(nav)
            .height(Length::Fill)
            .into()
    }

    // ── Source View ──────────────────────────────────────────────────

    fn view_source(&self) -> Element<Message> {
        let heading = text("Select Source").size(28);
        let subtitle = text("Choose a folder, mounted drive, or disk image to scan for recoverable files.");

        let path_row = row![
            text_input("e.g. /mnt/recovery or E:\\Backup", &self.source_input)
                .on_input(Message::SourceInputChanged)
                .padding(12)
                .size(16),
            button(text("Browse")).on_press(Message::BrowseSource).padding(12),
        ]
        .spacing(8);

        let scan_btn = button(
            row![text("🚀"), text("  Start Scan")].align_items(iced::Alignment::Center),
        )
        .on_press(Message::StartIndex)
        .padding(14);

        let hint = text("Read-only — your source data is never modified.").size(13);

        column![
            heading,
            vertical_space().height(8),
            subtitle,
            vertical_space().height(24),
            text("Source path:").size(14),
            path_row,
            vertical_space().height(20),
            scan_btn,
            vertical_space().height(12),
            hint,
        ]
        .spacing(4)
        .into()
    }

    // ── Browse View ─────────────────────────────────────────────────

    fn view_browse(&self) -> Element<Message> {
        let heading = row![
            text("Browse Files").size(28),
            horizontal_space(),
            text(format!(
                "{} shown / {} total / {} selected",
                self.filtered_indices.len(),
                self.files.len(),
                self.selected.len(),
            ))
            .size(13),
        ]
        .align_items(iced::Alignment::Center);

        let toolbar = row![
            text_input("Filter by name...", &self.filter_input)
                .on_input(Message::FilterInputChanged)
                .padding(10)
                .width(Length::FillPortion(3)),
            self.view_type_filter_buttons(),
            button(text("All")).on_press(Message::SelectAll).padding(8),
            button(text("None")).on_press(Message::SelectNone).padding(8),
        ]
        .spacing(6)
        .align_items(iced::Alignment::Center);

        let file_list: Element<Message> = if self.files.is_empty() {
            container(
                column![
                    text("No files loaded").size(18),
                    vertical_space().height(8),
                    text("Go to Source tab and scan a folder or drive.").size(14),
                ]
                .align_items(iced::Alignment::Center),
            )
            .center_x()
            .center_y()
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            let items: Vec<Element<Message>> = self
                .filtered_indices
                .iter()
                .take(500)
                .filter_map(|&idx| {
                    let entry = self.files.get(idx)?;
                    let is_sel = self.selected.contains(&idx);
                    let check = if is_sel { "☑" } else { "☐" };
                    let icon = entry.file_type.icon();
                    let name = entry.name();
                    let size_str = humansize::format_size(entry.size, humansize::BINARY);
                    let ext = if entry.extension.is_empty() {
                        String::new()
                    } else {
                        format!(".{}", entry.extension)
                    };

                    let file_row = row![
                        text(check).size(16).width(Length::Fixed(24.0)),
                        text(icon).size(16).width(Length::Fixed(28.0)),
                        text(&name).size(14).width(Length::FillPortion(5)),
                        text(ext).size(12).width(Length::Fixed(60.0)),
                        text(size_str).size(12).width(Length::Fixed(80.0)),
                    ]
                    .spacing(6)
                    .align_items(iced::Alignment::Center);

                    Some(
                        button(file_row)
                            .on_press(Message::ToggleSelect(idx))
                            .width(Length::Fill)
                            .padding(4)
                            .into(),
                    )
                })
                .collect();

            if items.is_empty() {
                text("No files match the current filter.").into()
            } else {
                scrollable(Column::with_children(items).spacing(1))
                    .height(Length::Fill)
                    .into()
            }
        };

        column![heading, toolbar, horizontal_rule(1), file_list]
            .spacing(10)
            .height(Length::Fill)
            .into()
    }

    fn view_type_filter_buttons(&self) -> Element<Message> {
        let types = [
            ("All", None),
            ("🖼 ", Some(FileType::Image)),
            ("🎬", Some(FileType::Video)),
            ("🎵", Some(FileType::Audio)),
            ("📄", Some(FileType::Document)),
            ("📦", Some(FileType::Archive)),
        ];

        let mut r = Row::new().spacing(4);
        for (label, ft) in types {
            let btn = button(text(label).size(13))
                .on_press(Message::SetTypeFilter(ft))
                .padding(6);
            r = r.push(btn);
        }
        r.into()
    }

    // ── Carve View ──────────────────────────────────────────────────

    fn view_carve(&self) -> Element<Message> {
        let heading = text("Carve Raw Disk Image").size(28);
        let subtitle = text(
            "Scan a raw disk image (dd, img, iso) for file signatures and extract recovered files.",
        );

        let source_row = row![
            text_input("Path to disk image...", &self.carve_source_input)
                .on_input(Message::CarveSourceChanged)
                .padding(12)
                .size(16),
            button(text("Browse")).on_press(Message::BrowseCarveSource).padding(12),
        ]
        .spacing(8);

        let output_row = row![
            text_input("Output folder for carved files...", &self.carve_output_input)
                .on_input(Message::CarveOutputChanged)
                .padding(12)
                .size(16),
            button(text("Browse")).on_press(Message::BrowseCarveOutput).padding(12),
        ]
        .spacing(8);

        let carve_btn = button(
            row![text("💎"), text("  Start Carving")].align_items(iced::Alignment::Center),
        )
        .on_press(Message::StartCarve)
        .padding(14);

        let features = column![
            text("Features:").size(14),
            text("  • 71 file format signatures (images, video, audio, docs, archives)").size(13),
            text("  • Parallel mmap scanning (uses all CPU cores)").size(13),
            text("  • Smart size detection (PNG chunks, RIFF headers, ZIP EOCD)").size(13),
            text("  • Blake3 hash verification on every extracted file").size(13),
            text("  • Sector-aligned scanning for true disk images").size(13),
        ]
        .spacing(3);

        column![
            heading,
            vertical_space().height(8),
            subtitle,
            vertical_space().height(20),
            text("Disk image:").size(14),
            source_row,
            vertical_space().height(12),
            text("Output folder:").size(14),
            output_row,
            vertical_space().height(20),
            carve_btn,
            vertical_space().height(20),
            horizontal_rule(1),
            vertical_space().height(12),
            features,
        ]
        .spacing(4)
        .into()
    }

    // ── Export View ──────────────────────────────────────────────────

    fn view_export(&self) -> Element<Message> {
        let heading = text("Export Files").size(28);

        let selected_count = self.selected.len();
        let total_size: u64 = self
            .selected
            .iter()
            .filter_map(|&i| self.files.get(i))
            .map(|e| e.size)
            .sum();

        let summary = if selected_count == 0 {
            text("No files selected. Go to Browse and select files first.")
        } else {
            text(format!(
                "{} files selected ({})",
                selected_count,
                humansize::format_size(total_size, humansize::BINARY),
            ))
        };

        let dest_row = row![
            text_input("Destination folder...", &self.dest_input)
                .on_input(Message::DestInputChanged)
                .padding(12)
                .size(16),
            button(text("Browse")).on_press(Message::BrowseDest).padding(12),
        ]
        .spacing(8);

        let export_btn = button(
            row![text("📤"), text("  Export with Verification")]
                .align_items(iced::Alignment::Center),
        )
        .on_press(Message::StartExport)
        .padding(14);

        let notes = column![
            text("Export options:").size(14),
            text("  • Blake3 hash verification on every file").size(13),
            text("  • JSON manifest with file hashes generated").size(13),
            text("  • Source data is never modified (read-only)").size(13),
        ]
        .spacing(3);

        column![
            heading,
            vertical_space().height(12),
            summary,
            vertical_space().height(20),
            text("Destination:").size(14),
            dest_row,
            vertical_space().height(20),
            export_btn,
            vertical_space().height(20),
            horizontal_rule(1),
            vertical_space().height(12),
            notes,
        ]
        .spacing(4)
        .into()
    }

    // ── Stats View ──────────────────────────────────────────────────

    fn view_stats(&self) -> Element<Message> {
        let heading = text("Recovery Summary").size(28);

        let mut stats_col = Column::new().spacing(8);

        // Index stats
        if !self.files.is_empty() {
            let mut type_counts: HashMap<FileType, (usize, u64)> = HashMap::new();
            for entry in &self.files {
                let e = type_counts.entry(entry.file_type).or_insert((0, 0));
                e.0 += 1;
                e.1 += entry.size;
            }

            stats_col = stats_col.push(text("Index Summary").size(20));
            stats_col = stats_col.push(text(format!("Total files: {}", self.files.len())).size(14));

            let total_bytes: u64 = self.files.iter().map(|e| e.size).sum();
            stats_col = stats_col.push(
                text(format!(
                    "Total size: {}",
                    humansize::format_size(total_bytes, humansize::BINARY)
                ))
                .size(14),
            );

            stats_col = stats_col.push(vertical_space().height(8));
            stats_col = stats_col.push(text("By type:").size(14));

            let mut sorted: Vec<_> = type_counts.iter().collect();
            sorted.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));

            for (ft, (count, bytes)) in sorted {
                stats_col = stats_col.push(
                    text(format!(
                        "  {} {:?}: {} files ({})",
                        ft.icon(),
                        ft,
                        count,
                        humansize::format_size(*bytes, humansize::BINARY),
                    ))
                    .size(13),
                );
            }

            stats_col = stats_col.push(vertical_space().height(16));
            stats_col = stats_col.push(horizontal_rule(1));
            stats_col = stats_col.push(vertical_space().height(16));
        }

        // Carve stats
        if let Some(ref cr) = self.carve_result {
            stats_col = stats_col.push(text("Carve Results").size(20));
            stats_col = stats_col.push(
                text(format!("Image size: {}", humansize::format_size(cr.image_size, humansize::BINARY))).size(14),
            );
            stats_col = stats_col.push(text(format!("Files found: {}", cr.files_found)).size(14));
            stats_col = stats_col.push(text(format!("Files extracted: {}", cr.files_extracted)).size(14));
            if cr.files_verified > 0 {
                stats_col = stats_col.push(text(format!("Content verified: {}", cr.files_verified)).size(14));
            }
            if cr.files_failed > 0 {
                stats_col = stats_col.push(text(format!("Failed: {}", cr.files_failed)).size(14));
            }
            stats_col = stats_col.push(
                text(format!(
                    "Extracted: {}",
                    humansize::format_size(cr.total_bytes_extracted, humansize::BINARY)
                ))
                .size(14),
            );
            if cr.duration_ms > 0 {
                let speed = cr.image_size * 1000 / cr.duration_ms.max(1);
                stats_col = stats_col.push(
                    text(format!(
                        "Duration: {:.1}s ({}/s)",
                        cr.duration_ms as f64 / 1000.0,
                        humansize::format_size(speed, humansize::BINARY),
                    ))
                    .size(14),
                );
            }

            if !cr.by_type.is_empty() {
                stats_col = stats_col.push(vertical_space().height(8));
                stats_col = stats_col.push(text("Carved by type:").size(14));
                let mut types: Vec<_> = cr.by_type.iter().collect();
                types.sort_by(|a, b| b.1.cmp(a.1));
                for (ext, count) in types {
                    stats_col = stats_col.push(text(format!("  .{}: {}", ext, count)).size(13));
                }
            }
        }

        if self.files.is_empty() && self.carve_result.is_none() {
            stats_col = stats_col.push(
                text("No data yet. Run a scan or carve operation first.").size(16),
            );
        }

        column![heading, vertical_space().height(12), scrollable(stats_col).height(Length::Fill)]
            .spacing(4)
            .into()
    }

    // ── Shared widgets ──────────────────────────────────────────────

    fn view_progress_bar(&self) -> Element<Message> {
        let bar = progress_bar(0.0..=1.0, self.progress).height(4);
        let label = text(&self.progress_label).size(12);
        container(column![bar, label].spacing(2))
            .padding([4, 24])
            .width(Length::Fill)
            .into()
    }

    fn view_error_banner(&self, err: &str) -> Element<Message> {
        let msg = row![
            text(format!("❌ {}", err)).size(14),
            horizontal_space(),
            button(text("✕").size(12))
                .on_press(Message::DismissError)
                .padding(4),
        ]
        .align_items(iced::Alignment::Center);

        container(msg)
            .padding([8, 24])
            .width(Length::Fill)
            .into()
    }

    fn view_status_bar(&self) -> Element<Message> {
        let status = text(format!("✓ {}", self.status)).size(12);
        let counts = text(format!(
            "{} files | {} selected | {} carved",
            self.files.len(),
            self.selected.len(),
            self.carved_files.len(),
        ))
        .size(12);

        container(row![status, horizontal_space(), counts].align_items(iced::Alignment::Center))
            .padding([6, 24])
            .width(Length::Fill)
            .into()
    }

    // ── Logic helpers ───────────────────────────────────────────────

    fn rebuild_filter(&mut self, query: &str) {
        if query.is_empty() && self.type_filter.is_none() {
            self.filtered_indices = (0..self.files.len()).collect();
            return;
        }
        let lower = query.to_lowercase();
        self.filtered_indices = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if let Some(ft) = self.type_filter {
                    if e.file_type != ft {
                        return false;
                    }
                }
                if !lower.is_empty() && !e.name().to_lowercase().contains(&lower) {
                    return false;
                }
                true
            })
            .map(|(i, _)| i)
            .collect();
    }
}

// ── Async operations (run off the GUI thread) ────────────────────────────

async fn index_source(source: String) -> Result<Vec<FileEntry>, String> {
    let path = PathBuf::from(&source);
    if !path.exists() {
        return Err(format!("Path does not exist: {}", source));
    }

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

    let files = engine.get_all_files().await.map_err(|e| e.to_string())?;
    let entries: Vec<FileEntry> = files
        .iter()
        .filter_map(|p| {
            let pb = PathBuf::from(p);
            let meta = std::fs::metadata(&pb).ok()?;
            Some(FileEntry::new(pb, &meta))
        })
        .collect();

    Ok(entries)
}

async fn run_export(entries: Vec<FileEntry>, dest: String) -> Result<usize, String> {
    let options = ExportOptions {
        dest: PathBuf::from(&dest),
        preserve_structure: true,
        verify_hash: true,
        continue_on_error: true,
        create_manifest: true,
        dry_run: false,
    };

    let exporter = Exporter::new(options);
    let result = exporter
        .export_batch(&entries, |_| {})
        .await
        .map_err(|e| e.to_string())?;

    Ok(result.successful)
}

async fn run_carve(
    source: String,
    output: String,
) -> Result<(Vec<CarvedFile>, CarveResult), String> {
    let opts = CarveOptions {
        source: PathBuf::from(&source),
        output_dir: PathBuf::from(&output),
        sector_aligned: true,
        min_size: 512,
        file_types: None,
        workers: num_cpus::get(),
        dry_run: false,
        verify: true,
    };

    let carver = Carver::new(opts);
    carver.carve().await.map_err(|e| e.to_string())
}
