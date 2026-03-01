//! TUI rendering with ratatui — btop-inspired dense visual layout
//!
//! Draws the terminal UI: header, tabs, file tree, details, stats, status bar.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Padding, Paragraph, Tabs},
    Frame,
};

use super::app::{App, AppState, Tab};
use crate::core::FileType;

// ── Color palette ───────────────────────────────────────────────────
const C_BRAND: Color = Color::Cyan;
const C_ACCENT: Color = Color::LightCyan;
const C_OK: Color = Color::Green;
const C_WARN: Color = Color::Yellow;
const C_ERR: Color = Color::Red;
const C_TEXT: Color = Color::White;
const C_DIM: Color = Color::DarkGray;
const C_BG_SELECT: Color = Color::Rgb(30, 60, 90);
const C_BORDER: Color = Color::Rgb(60, 60, 80);
const C_BORDER_ACTIVE: Color = Color::Cyan;

// ── File type colors ────────────────────────────────────────────────
fn ft_color(ft: &FileType) -> Color {
    match ft {
        FileType::Image => Color::Magenta,
        FileType::Video => Color::Cyan,
        FileType::Audio => Color::Yellow,
        FileType::Document => Color::Green,
        FileType::Archive => Color::Blue,
        FileType::Code => Color::Red,
        FileType::Executable => Color::LightRed,
        FileType::Database => Color::LightBlue,
        FileType::Other => Color::White,
    }
}

fn ft_icon(ft: &FileType) -> &'static str {
    match ft {
        FileType::Image => "IMG",
        FileType::Video => "VID",
        FileType::Audio => "AUD",
        FileType::Document => "DOC",
        FileType::Archive => "ARC",
        FileType::Code => "COD",
        FileType::Executable => "EXE",
        FileType::Database => " DB",
        FileType::Other => "---",
    }
}

fn ft_label(ft: &FileType) -> &'static str {
    match ft {
        FileType::Image => "Image",
        FileType::Video => "Video",
        FileType::Audio => "Audio",
        FileType::Document => "Doc",
        FileType::Archive => "Archive",
        FileType::Code => "Code",
        FileType::Executable => "Exe",
        FileType::Database => "DB",
        FileType::Other => "Other",
    }
}

/// Format bytes as human-readable, compact
fn fmt_size(bytes: u64) -> String {
    humansize::format_size(bytes, humansize::BINARY)
}

// ═══════════════════════════════════════════════════════════════════
//  MAIN DRAW
// ═══════════════════════════════════════════════════════════════════

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Main layout: header + tabs + content + status
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header bar
            Constraint::Length(3), // Tab bar
            Constraint::Min(8),   // Content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    draw_tabs(frame, chunks[1], app);

    match app.state {
        AppState::Indexing => draw_indexing(frame, chunks[2], app),
        AppState::Init => draw_init(frame, chunks[2]),
        _ => draw_content(frame, chunks[2], app),
    }

    draw_status_bar(frame, chunks[3], app);

    if app.show_help {
        draw_help_overlay(frame, area);
    }
}

// ═══════════════════════════════════════════════════════════════════
//  HEADER BAR — single dense line with branding + stats
// ═══════════════════════════════════════════════════════════════════

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(" \u{25c6} ", Style::default().fg(C_BRAND).add_modifier(Modifier::BOLD)),
        Span::styled(
            "DIAMOND DRILL",
            Style::default().fg(C_BRAND).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" v0.1.0 ", Style::default().fg(C_DIM)),
        Span::styled("\u{2502} ", Style::default().fg(C_BORDER)),
    ];

    if app.file_count > 0 {
        spans.extend_from_slice(&[
            Span::styled(
                format!("{}", app.file_count),
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" files ", Style::default().fg(C_DIM)),
            Span::styled("\u{2502} ", Style::default().fg(C_BORDER)),
            Span::styled(
                fmt_size(app.total_size),
                Style::default().fg(C_ACCENT),
            ),
            Span::styled(" total ", Style::default().fg(C_DIM)),
            Span::styled("\u{2502} ", Style::default().fg(C_BORDER)),
        ]);

        if !app.selected_files.is_empty() {
            spans.extend_from_slice(&[
                Span::styled(
                    format!("{}", app.selected_files.len()),
                    Style::default().fg(C_WARN).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" selected ", Style::default().fg(C_DIM)),
                Span::styled(
                    format!("({})", fmt_size(app.selected_size)),
                    Style::default().fg(C_WARN),
                ),
                Span::styled(" \u{2502} ", Style::default().fg(C_BORDER)),
            ]);
        }

        spans.push(Span::styled(
            " READ-ONLY ",
            Style::default()
                .fg(Color::Black)
                .bg(C_OK)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            &app.source_label,
            Style::default().fg(C_DIM),
        ));
    }

    let header = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));
    frame.render_widget(header, area);
}

// ═══════════════════════════════════════════════════════════════════
//  TAB BAR
// ═══════════════════════════════════════════════════════════════════

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Tab::all()
        .iter()
        .map(|t| {
            if *t == app.tab {
                Line::from(vec![
                    Span::styled(
                        "\u{25b8} ",
                        Style::default().fg(C_BRAND).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        t.label().trim(),
                        Style::default().fg(C_BRAND).add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(Span::styled(
                    format!("  {} ", t.label().trim()),
                    Style::default().fg(C_DIM),
                ))
            }
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_BORDER))
                .title(Span::styled(
                    " Tabs [1-6] ",
                    Style::default().fg(C_DIM),
                )),
        )
        .select(app.tab.index())
        .style(Style::default().fg(C_TEXT))
        .highlight_style(
            Style::default()
                .fg(C_BRAND)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled(" \u{2502} ", Style::default().fg(C_BORDER)));

    frame.render_widget(tabs, area);
}

// ═══════════════════════════════════════════════════════════════════
//  INIT SCREEN
// ═══════════════════════════════════════════════════════════════════

fn draw_init(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "       \u{25c6}  DIAMOND DRILL  \u{25c6}",
            Style::default()
                .fg(C_BRAND)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "     Ultra-fast Disk Recovery",
            Style::default().fg(C_DIM),
        )),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "  No source loaded. Run with:",
            Style::default().fg(C_TEXT),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "    diamond-drill tui /path/to/disk-or-folder",
            Style::default().fg(C_ACCENT),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Press 'q' to quit.",
            Style::default().fg(C_DIM),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER))
        .title(Span::styled(
            " Welcome ",
            Style::default().fg(C_BRAND),
        ));

    frame.render_widget(Paragraph::new(text).block(block), area);
}

// ═══════════════════════════════════════════════════════════════════
//  INDEXING PROGRESS
// ═══════════════════════════════════════════════════════════════════

fn draw_indexing(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(1),
        ])
        .split(area);

    // Progress gauge
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_BORDER))
                .title(Span::styled(
                    " Indexing ",
                    Style::default().fg(C_BRAND),
                )),
        )
        .gauge_style(
            Style::default()
                .fg(C_BRAND)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(app.index_progress)
        .label(Span::styled(
            format!("{:.0}%", app.index_progress * 100.0),
            Style::default()
                .fg(C_TEXT)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(gauge, chunks[0]);

    // Status message
    let status_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(C_DIM)),
            Span::styled(
                &app.status_message,
                Style::default().fg(C_ACCENT),
            ),
        ]),
    ];
    let msg = Paragraph::new(status_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(C_BORDER))
            .title(Span::styled(" Status ", Style::default().fg(C_DIM))),
    );
    frame.render_widget(msg, chunks[1]);

    // ASCII art
    let art = vec![
        Line::from(""),
        Line::from(Span::styled(
            "    Scanning filesystem...",
            Style::default().fg(C_DIM),
        )),
        Line::from(Span::styled(
            "    Building index for fast search and recovery",
            Style::default().fg(C_DIM),
        )),
    ];
    frame.render_widget(Paragraph::new(art), chunks[2]);
}

// ═══════════════════════════════════════════════════════════════════
//  CONTENT ROUTER
// ═══════════════════════════════════════════════════════════════════

fn draw_content(frame: &mut Frame, area: Rect, app: &App) {
    match app.tab {
        Tab::Files => draw_files_tab(frame, area, app),
        Tab::Search => draw_search_tab(frame, area, app),
        Tab::Export => draw_export_tab(frame, area, app),
        Tab::Carve => draw_carve_tab(frame, area, app),
        Tab::Dedup => draw_dedup_tab(frame, area, app),
        Tab::BadSectors => draw_badsector_tab(frame, area, app),
    }
}

// ═══════════════════════════════════════════════════════════════════
//  FILES TAB — split into file tree (left) + details/stats (right)
// ═══════════════════════════════════════════════════════════════════

fn draw_files_tab(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_file_list(frame, chunks[0], app);
    draw_right_panel(frame, chunks[1], app);
}

/// Draw the file list with aligned columns
fn draw_file_list(frame: &mut Frame, area: Rect, app: &App) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let (nodes, relative_selected) = app.file_tree.visible_window(inner_height);

    let items: Vec<ListItem> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let is_selected = app.selected_files.contains(&node.path);
            let is_cursor = i == relative_selected;

            let check = if is_selected {
                Span::styled("\u{25cf} ", Style::default().fg(C_OK).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("\u{25cb} ", Style::default().fg(C_DIM))
            };

            let icon_color = ft_color(&node.file_type);
            let icon = Span::styled(
                format!("{} ", ft_icon(&node.file_type)),
                Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
            );

            // Find file size from cached entries
            let size_str = app
                .cached_entries
                .iter()
                .find(|e| e.path.to_string_lossy().as_ref() == node.path.as_str())
                .map(|e| fmt_size(e.size))
                .unwrap_or_default();

            // Truncate name to fit
            let max_name = (area.width as usize).saturating_sub(22);
            let name = if node.name.len() > max_name {
                format!("{}\u{2026}", &node.name[..max_name.saturating_sub(1)])
            } else {
                node.name.clone()
            };

            let name_style = if is_cursor {
                Style::default()
                    .fg(C_TEXT)
                    .bg(C_BG_SELECT)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(C_OK)
            } else {
                Style::default().fg(icon_color)
            };

            let size_style = if is_cursor {
                Style::default().fg(C_DIM).bg(C_BG_SELECT)
            } else {
                Style::default().fg(C_DIM)
            };

            // Pad name for alignment
            let padded_name = format!("{:<width$}", name, width = max_name);

            ListItem::new(Line::from(vec![
                check,
                icon,
                Span::styled(padded_name, name_style),
                Span::styled(format!(" {:>8}", size_str), size_style),
            ]))
        })
        .collect();

    let title = format!(
        " Files ({}/{}) ",
        app.file_tree.visible_count(),
        app.file_count
    );

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(C_BORDER_ACTIVE))
            .title(Span::styled(title, Style::default().fg(C_BRAND))),
    );

    frame.render_widget(list, area);
}

/// Draw the right panel: details + distribution + summary
fn draw_right_panel(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),  // File details
            Constraint::Min(8),     // Distribution
            Constraint::Length(4),  // Selection summary
        ])
        .split(area);

    draw_file_details(frame, chunks[0], app);
    draw_distribution(frame, chunks[1], app);
    draw_summary_panel(frame, chunks[2], app);
}

/// Draw file details panel
fn draw_file_details(frame: &mut Frame, area: Rect, app: &App) {
    let text = if let Some(path) = app.file_tree.selected_path() {
        let entry = app
            .cached_entries
            .iter()
            .find(|e| e.path.to_string_lossy().as_ref() == path.as_str());

        let name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&path);

        let mut lines = vec![
            Line::from(Span::styled(
                format!("  {}", name),
                Style::default()
                    .fg(C_TEXT)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("  {}", "\u{2500}".repeat(name.len().min(40))),
                Style::default().fg(C_BORDER),
            )),
        ];

        if let Some(e) = entry {
            lines.push(Line::from(vec![
                Span::styled("  Type   ", Style::default().fg(C_DIM)),
                Span::styled(
                    ft_label(&e.file_type),
                    Style::default().fg(ft_color(&e.file_type)).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" (.{})", e.extension),
                    Style::default().fg(C_DIM),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Size   ", Style::default().fg(C_DIM)),
                Span::styled(
                    fmt_size(e.size),
                    Style::default().fg(C_ACCENT),
                ),
                Span::styled(
                    format!(" ({} bytes)", e.size),
                    Style::default().fg(C_DIM),
                ),
            ]));
            if let Some(ref modified) = e.modified {
                lines.push(Line::from(vec![
                    Span::styled("  Date   ", Style::default().fg(C_DIM)),
                    Span::styled(
                        modified.format("%Y-%m-%d %H:%M").to_string(),
                        Style::default().fg(C_TEXT),
                    ),
                ]));
            }
            if let Some(ref hash) = e.hash {
                let short = if hash.len() > 16 { &hash[..16] } else { hash };
                lines.push(Line::from(vec![
                    Span::styled("  Hash   ", Style::default().fg(C_DIM)),
                    Span::styled(short, Style::default().fg(C_DIM)),
                    Span::styled("\u{2026}", Style::default().fg(C_DIM)),
                ]));
            }
            if e.has_bad_sectors {
                lines.push(Line::from(Span::styled(
                    "  \u{26a0} BAD SECTORS DETECTED",
                    Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
                )));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled("  Path   ", Style::default().fg(C_DIM)),
                Span::styled(path.clone(), Style::default().fg(C_TEXT)),
            ]));
        }

        lines
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No file selected",
                Style::default().fg(C_DIM),
            )),
        ]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER))
        .title(Span::styled(" Details ", Style::default().fg(C_BRAND)));

    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// Draw file type distribution bars
fn draw_distribution(frame: &mut Frame, area: Rect, app: &App) {
    let inner_width = area.width.saturating_sub(4) as usize;
    let bar_width = inner_width.saturating_sub(22); // label(7) + bar + count(6) + pct(5) + padding

    // Sort types by count descending
    let mut type_list: Vec<(FileType, usize, u64)> = app
        .type_counts
        .iter()
        .map(|(&ft, &count)| {
            let size = app.type_sizes.get(&ft).copied().unwrap_or(0);
            (ft, count, size)
        })
        .collect();
    type_list.sort_by(|a, b| b.1.cmp(&a.1));

    let max_count = type_list.first().map(|t| t.1).unwrap_or(1).max(1);
    let inner_height = area.height.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();

    for (ft, count, _size) in type_list.iter().take(inner_height) {
        let pct = if app.file_count > 0 {
            (*count as f64 / app.file_count as f64 * 100.0) as u32
        } else {
            0
        };

        let filled = (*count as f64 / max_count as f64 * bar_width as f64) as usize;
        let empty = bar_width.saturating_sub(filled);

        let bar_filled = "\u{2588}".repeat(filled);
        let bar_empty = "\u{2591}".repeat(empty);

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<7}", ft_label(ft)),
                Style::default().fg(ft_color(ft)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(bar_filled, Style::default().fg(ft_color(ft))),
            Span::styled(bar_empty, Style::default().fg(Color::Rgb(40, 40, 50))),
            Span::styled(
                format!(" {:>5}", count),
                Style::default().fg(C_TEXT),
            ),
            Span::styled(
                format!(" {:>3}%", pct),
                Style::default().fg(C_DIM),
            ),
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No files indexed yet",
            Style::default().fg(C_DIM),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER))
        .title(Span::styled(
            " Distribution ",
            Style::default().fg(C_BRAND),
        ));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Draw selection/total summary panel
fn draw_summary_panel(frame: &mut Frame, area: Rect, app: &App) {
    let lines = vec![
        Line::from(vec![
            Span::styled("  Total    ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", app.file_count),
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" files  ", Style::default().fg(C_DIM)),
            Span::styled(
                fmt_size(app.total_size),
                Style::default().fg(C_ACCENT),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Select   ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", app.selected_files.len()),
                Style::default()
                    .fg(if app.selected_files.is_empty() {
                        C_DIM
                    } else {
                        C_WARN
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" files  ", Style::default().fg(C_DIM)),
            Span::styled(
                fmt_size(app.selected_size),
                Style::default().fg(if app.selected_files.is_empty() {
                    C_DIM
                } else {
                    C_WARN
                }),
            ),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER))
        .title(Span::styled(" Summary ", Style::default().fg(C_DIM)));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ═══════════════════════════════════════════════════════════════════
//  SEARCH TAB
// ═══════════════════════════════════════════════════════════════════

fn draw_search_tab(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    // Search input
    let (search_text, search_style) = if app.state == AppState::SearchInput {
        (
            format!(" \u{25b8} /{}_ ", app.filter),
            Style::default().fg(C_WARN).add_modifier(Modifier::BOLD),
        )
    } else if app.filter.is_empty() {
        (
            " Press '/' to search...".to_string(),
            Style::default().fg(C_DIM),
        )
    } else {
        (
            format!(" Filter: {} ", app.filter),
            Style::default().fg(C_ACCENT),
        )
    };

    let border_color = if app.state == AppState::SearchInput {
        C_WARN
    } else {
        C_BORDER
    };

    let search = Paragraph::new(search_text).style(search_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(" Search ", Style::default().fg(C_BRAND))),
    );
    frame.render_widget(search, chunks[0]);

    // Results list
    draw_file_list(frame, chunks[1], app);
}

// ═══════════════════════════════════════════════════════════════════
//  EXPORT TAB
// ═══════════════════════════════════════════════════════════════════

fn draw_export_tab(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(3)])
        .split(area);

    // Export summary
    let summary = vec![
        Line::from(vec![
            Span::styled("  Selected: ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{} files", app.selected_files.len()),
                Style::default()
                    .fg(if app.selected_files.is_empty() {
                        C_DIM
                    } else {
                        C_OK
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(C_BORDER)),
            Span::styled(
                fmt_size(app.selected_size),
                Style::default().fg(C_ACCENT),
            ),
        ]),
        Line::from(""),
        if app.selected_files.is_empty() {
            Line::from(Span::styled(
                "  Go to Files tab, press Space to select files for export.",
                Style::default().fg(C_DIM),
            ))
        } else {
            Line::from(Span::styled(
                "  Use CLI to export:  diamond-drill export <source> <dest>",
                Style::default().fg(C_ACCENT),
            ))
        },
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER))
        .title(Span::styled(" Export ", Style::default().fg(C_BRAND)));

    frame.render_widget(Paragraph::new(summary).block(block), chunks[0]);

    // Selected files list
    if !app.selected_files.is_empty() {
        let items: Vec<ListItem> = app
            .selected_files
            .iter()
            .map(|path| {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
                let size = app
                    .cached_entries
                    .iter()
                    .find(|e| e.path.to_string_lossy().as_ref() == path.as_str())
                    .map(|e| fmt_size(e.size))
                    .unwrap_or_default();

                ListItem::new(Line::from(vec![
                    Span::styled("  \u{25cf} ", Style::default().fg(C_OK)),
                    Span::styled(name, Style::default().fg(C_TEXT)),
                    Span::styled(format!("  {}", size), Style::default().fg(C_DIM)),
                ]))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_BORDER))
                .title(Span::styled(
                    " Selected Files ",
                    Style::default().fg(C_OK),
                )),
        );
        frame.render_widget(list, chunks[1]);
    } else {
        frame.render_widget(
            Paragraph::new("").block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(C_BORDER)),
            ),
            chunks[1],
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
//  CARVE TAB
// ═══════════════════════════════════════════════════════════════════

fn draw_carve_tab(frame: &mut Frame, area: Rect, app: &App) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  File Carving Engine",
            Style::default()
                .fg(C_BRAND)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            Style::default().fg(C_BORDER),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Supported: ", Style::default().fg(C_DIM)),
            Span::styled(
                "60+ file signatures",
                Style::default().fg(C_ACCENT),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Modes:     ", Style::default().fg(C_DIM)),
            Span::styled(
                "sector-aligned, raw scan, dry-run",
                Style::default().fg(C_TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Verify:    ", Style::default().fg(C_DIM)),
            Span::styled(
                "BLAKE3 content validation",
                Style::default().fg(C_OK),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  To carve, use CLI:",
            Style::default().fg(C_DIM),
        )),
        Line::from(Span::styled(
            format!("    diamond-drill carve {} <output-dir>",
                app.source.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "<source>".into())),
            Style::default().fg(C_ACCENT),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER))
        .title(Span::styled(" Carve ", Style::default().fg(C_BRAND)));

    frame.render_widget(Paragraph::new(text).block(block), area);
}

// ═══════════════════════════════════════════════════════════════════
//  DEDUP TAB
// ═══════════════════════════════════════════════════════════════════

fn draw_dedup_tab(frame: &mut Frame, area: Rect, app: &App) {
    let report = match &app.dedup_report {
        Some(r) => r,
        None => {
            let text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Duplicate Detection",
                    Style::default()
                        .fg(C_BRAND)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press 'd' to scan indexed files for duplicates.",
                    Style::default().fg(C_TEXT),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Uses BLAKE3 hashing + fuzzy name matching.",
                    Style::default().fg(C_DIM),
                )),
            ];
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(C_BORDER))
                .title(Span::styled(" Dedup ", Style::default().fg(C_BRAND)));
            frame.render_widget(Paragraph::new(text).block(block), area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(3)])
        .split(area);

    // Summary with colored stats
    let summary = vec![
        Line::from(vec![
            Span::styled("  Scanned  ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", report.scanned_files),
                Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  \u{2502}  ", Style::default().fg(C_BORDER)),
            Span::styled("Unique  ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", report.unique_files),
                Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Groups   ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", report.duplicate_groups),
                Style::default().fg(C_WARN).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  \u{2502}  ", Style::default().fg(C_BORDER)),
            Span::styled("Dupes   ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", report.total_duplicates),
                Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  \u{2502}  ", Style::default().fg(C_BORDER)),
            Span::styled("Wasted  ", Style::default().fg(C_DIM)),
            Span::styled(
                fmt_size(report.wasted_bytes),
                Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Method   ", Style::default().fg(C_DIM)),
            Span::styled(&report.strategy, Style::default().fg(C_ACCENT)),
            Span::styled("  \u{2502}  ", Style::default().fg(C_BORDER)),
            Span::styled(
                format!("Threshold {}%", report.fuzzy_threshold),
                Style::default().fg(C_DIM),
            ),
        ]),
    ];

    let summary_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_WARN))
        .title(Span::styled(
            " Dedup Summary ",
            Style::default().fg(C_WARN).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(Paragraph::new(summary).block(summary_block), chunks[0]);

    // Group list with visual distinction
    let inner_height = chunks[1].height.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (i, group) in report.groups.iter().enumerate() {
        let kind = if group.similarity == 100 {
            Span::styled(
                " EXACT ",
                Style::default()
                    .fg(Color::Black)
                    .bg(C_ERR)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                format!(" ~{}% ", group.similarity),
                Style::default()
                    .fg(Color::Black)
                    .bg(C_WARN)
                    .add_modifier(Modifier::BOLD),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  #{:<3} ", i + 1),
                Style::default().fg(C_DIM),
            ),
            kind,
            Span::styled("  ", Style::default()),
            Span::styled(
                fmt_size(group.wasted_bytes),
                Style::default().fg(C_ERR),
            ),
            Span::styled(" wasted", Style::default().fg(C_DIM)),
        ]));

        let master_name = group
            .master
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| group.master.display().to_string());
        lines.push(Line::from(vec![
            Span::styled(
                "    KEEP ",
                Style::default().fg(C_OK).add_modifier(Modifier::BOLD),
            ),
            Span::styled(master_name, Style::default().fg(C_TEXT)),
        ]));

        for dup in group.duplicates.iter().take(5) {
            let dup_name = dup
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| dup.display().to_string());
            lines.push(Line::from(vec![
                Span::styled("    DEL  ", Style::default().fg(C_ERR)),
                Span::styled(dup_name, Style::default().fg(C_DIM)),
            ]));
        }
        if group.duplicates.len() > 5 {
            lines.push(Line::from(Span::styled(
                format!("    \u{2026} +{} more", group.duplicates.len() - 5),
                Style::default().fg(C_DIM),
            )));
        }
        lines.push(Line::from(""));
    }

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(app.dedup_scroll)
        .take(inner_height)
        .collect();

    let groups_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_WARN))
        .title(Span::styled(
            " Groups [\u{2191}/\u{2193} scroll] [d refresh] ",
            Style::default().fg(C_DIM),
        ));
    frame.render_widget(Paragraph::new(visible_lines).block(groups_block), chunks[1]);
}

// ═══════════════════════════════════════════════════════════════════
//  BAD SECTORS TAB — heatmap visualization
// ═══════════════════════════════════════════════════════════════════

fn draw_badsector_tab(frame: &mut Frame, area: Rect, app: &App) {
    if app.bad_sector_maps.is_empty() {
        let text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Bad Sector Scanner",
                Style::default()
                    .fg(C_BRAND)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press 'b' to scan indexed files for bad sectors.",
                Style::default().fg(C_TEXT),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Reads blocks with retry + exponential backoff.",
                Style::default().fg(C_DIM),
            )),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(C_BORDER))
            .title(Span::styled(" Bad Sectors ", Style::default().fg(C_BRAND)));
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(3)])
        .split(area);

    // Summary
    let total_bad: u64 = app.bad_sector_maps.iter().map(|m| m.bad_bytes).sum();
    let total_bad_blocks: u64 = app
        .bad_sector_maps
        .iter()
        .map(|m| m.bad_blocks.len() as u64)
        .sum();

    let summary = vec![
        Line::from(vec![
            Span::styled("  Files with errors: ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", app.bad_sector_maps.len()),
                Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Bad blocks: ", Style::default().fg(C_DIM)),
            Span::styled(
                format!("{}", total_bad_blocks),
                Style::default().fg(C_ERR),
            ),
            Span::styled("  \u{2502}  ", Style::default().fg(C_BORDER)),
            Span::styled("Unreadable: ", Style::default().fg(C_DIM)),
            Span::styled(
                fmt_size(total_bad),
                Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let summary_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_ERR))
        .title(Span::styled(
            " Bad Sector Summary ",
            Style::default().fg(C_ERR).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(Paragraph::new(summary).block(summary_block), chunks[0]);

    // Heatmaps with colored block visualization
    let inner_height = chunks[1].height.saturating_sub(2) as usize;
    let heatmap_width = chunks[1].width.saturating_sub(6) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for map in &app.bad_sector_maps {
        let filename = map
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| map.path.display().to_string());

        let pct = map.readable_percent();
        let health_color = if pct > 95.0 {
            C_OK
        } else if pct > 50.0 {
            C_WARN
        } else {
            C_ERR
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", filename), Style::default().fg(C_TEXT)),
            Span::styled(
                format!("{:.1}%", pct),
                Style::default().fg(health_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ok", Style::default().fg(C_DIM)),
        ]));

        // Visual heatmap bar using block chars
        let heatmap = map.heatmap();
        let bar_width = heatmap_width.min(60);
        let bar = heatmap.summary_bar(bar_width);
        // Color the heatmap: good blocks green, bad blocks red
        let mut bar_spans = vec![Span::styled("  \u{2595}", Style::default().fg(C_DIM))];
        for ch in bar.chars() {
            let color = match ch {
                '\u{2588}' => C_ERR,   // full block = bad
                '\u{2593}' => C_WARN,  // dark shade = partial
                _ => C_OK,             // good
            };
            bar_spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
        }
        bar_spans.push(Span::styled("\u{258f}", Style::default().fg(C_DIM)));
        lines.push(Line::from(bar_spans));

        for block in map.bad_blocks.iter().take(3) {
            lines.push(Line::from(Span::styled(
                format!(
                    "    0x{:08X}  {} bytes  {} retries",
                    block.offset, block.length, block.retry_count
                ),
                Style::default().fg(C_DIM),
            )));
        }
        if map.bad_blocks.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("    \u{2026} +{} more", map.bad_blocks.len() - 3),
                Style::default().fg(C_DIM),
            )));
        }
        lines.push(Line::from(""));
    }

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(app.bad_sector_scroll)
        .take(inner_height)
        .collect();

    let heatmap_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_ERR))
        .title(Span::styled(
            " Heatmaps [\u{2191}/\u{2193} scroll] [b rescan] ",
            Style::default().fg(C_DIM),
        ));
    frame.render_widget(
        Paragraph::new(visible_lines).block(heatmap_block),
        chunks[1],
    );
}

// ═══════════════════════════════════════════════════════════════════
//  STATUS BAR — dense single line
// ═══════════════════════════════════════════════════════════════════

fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let left_spans = vec![
        Span::styled(" ", Style::default()),
        Span::styled(&app.status_message, Style::default().fg(C_TEXT)),
    ];

    let right_text = " ?:Help  j/k:Nav  Space:Sel  /:Find  Tab:Switch  q:Quit ";

    // Calculate right-align padding
    let left_len = app.status_message.len() + 1;
    let right_len = right_text.len();
    let padding = (area.width as usize)
        .saturating_sub(left_len)
        .saturating_sub(right_len);

    let mut spans = left_spans;
    spans.push(Span::styled(
        " ".repeat(padding),
        Style::default(),
    ));
    spans.push(Span::styled(right_text, Style::default().fg(C_DIM)));

    let bar = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));
    frame.render_widget(bar, area);
}

// ═══════════════════════════════════════════════════════════════════
//  HELP OVERLAY
// ═══════════════════════════════════════════════════════════════════

fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    let popup_width = 58.min(area.width.saturating_sub(4));
    let popup_height = 26.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  KEYBOARD SHORTCUTS",
            Style::default()
                .fg(C_BRAND)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled("  Navigation", Style::default().fg(C_WARN))),
        Line::from(vec![
            Span::styled("    j/\u{2193}  k/\u{2191}  ", Style::default().fg(C_ACCENT)),
            Span::styled("Move down / up", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    g  G       ", Style::default().fg(C_ACCENT)),
            Span::styled("Jump to first / last", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    PgUp PgDn  ", Style::default().fg(C_ACCENT)),
            Span::styled("Page up / down", Style::default().fg(C_TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Selection", Style::default().fg(C_WARN))),
        Line::from(vec![
            Span::styled("    Space      ", Style::default().fg(C_ACCENT)),
            Span::styled("Toggle selection", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    a  n  i    ", Style::default().fg(C_ACCENT)),
            Span::styled("All / None / Invert", Style::default().fg(C_TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Actions", Style::default().fg(C_WARN))),
        Line::from(vec![
            Span::styled("    o          ", Style::default().fg(C_ACCENT)),
            Span::styled("Open in viewer", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    r          ", Style::default().fg(C_ACCENT)),
            Span::styled("Reveal in explorer", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    d          ", Style::default().fg(C_ACCENT)),
            Span::styled("Run dedup analysis (Dedup tab)", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    b          ", Style::default().fg(C_ACCENT)),
            Span::styled("Scan bad sectors (BadSector tab)", Style::default().fg(C_TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Tabs & Search", Style::default().fg(C_WARN))),
        Line::from(vec![
            Span::styled("    Tab  1-6   ", Style::default().fg(C_ACCENT)),
            Span::styled("Switch tabs", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    /          ", Style::default().fg(C_ACCENT)),
            Span::styled("Filter / search files", Style::default().fg(C_TEXT)),
        ]),
        Line::from(vec![
            Span::styled("    ?  q       ", Style::default().fg(C_ACCENT)),
            Span::styled("Help / Quit", Style::default().fg(C_TEXT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press any key to close",
            Style::default().fg(C_DIM),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BRAND))
        .title(Span::styled(
            " Help ",
            Style::default().fg(C_BRAND).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center)
        .padding(Padding::horizontal(1));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, popup_area);
}
