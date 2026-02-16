//! TUI rendering with ratatui
//!
//! Draws the terminal UI: file tree, details, tabs, status bar.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Padding, Paragraph, Tabs},
    Frame,
};

use super::app::{App, AppState, Tab};
use crate::core::FileType;

/// Main draw function — renders the entire TUI
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Main layout: top tabs, center content, bottom status
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tab bar
            Constraint::Min(5),    // Content
            Constraint::Length(3), // Status bar
        ])
        .split(area);

    draw_tabs(frame, chunks[0], app);

    match app.state {
        AppState::Indexing => draw_indexing(frame, chunks[1], app),
        AppState::Init => draw_init(frame, chunks[1]),
        _ => draw_content(frame, chunks[1], app),
    }

    draw_status_bar(frame, chunks[2], app);

    // Help overlay (drawn last, on top)
    if app.show_help {
        draw_help_overlay(frame, area);
    }
}

/// Draw tab bar
fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Tab::all()
        .iter()
        .map(|t| {
            let style = if *t == app.tab {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(Span::styled(t.label(), style))
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Diamond Drill TUI "),
        )
        .select(app.tab.index())
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}

/// Draw init screen
fn draw_init(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Diamond Drill TUI",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  No source loaded. Use:"),
        Line::from("    diamond-drill tui <source-path>"),
        Line::from(""),
        Line::from("  Or press 'q' to quit."),
    ];

    let block = Block::default().borders(Borders::ALL).title(" Welcome ");

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

/// Draw indexing progress
fn draw_indexing(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Indexing... "),
        )
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(app.index_progress)
        .label(format!("{:.0}%", app.index_progress * 100.0));

    frame.render_widget(gauge, chunks[0]);

    let msg = Paragraph::new(app.status_message.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Status "));
    frame.render_widget(msg, chunks[1]);
}

/// Draw main content based on active tab
fn draw_content(frame: &mut Frame, area: Rect, app: &App) {
    match app.tab {
        Tab::Files => draw_files_tab(frame, area, app),
        Tab::Search => draw_search_tab(frame, area, app),
        Tab::Export => draw_export_tab(frame, area, app),
        Tab::Dedup => draw_dedup_tab(frame, area, app),
        Tab::BadSectors => draw_badsector_tab(frame, area, app),
    }
}

/// Draw files tab — split into file tree (left) and details (right)
fn draw_files_tab(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left: File tree
    draw_file_list(frame, chunks[0], app);

    // Right: File details
    draw_file_details(frame, chunks[1], app);
}

/// Draw the file list panel
fn draw_file_list(frame: &mut Frame, area: Rect, app: &App) {
    let inner_height = area.height.saturating_sub(2) as usize; // borders
    let (nodes, relative_selected) = app.file_tree.visible_window(inner_height);

    let items: Vec<ListItem> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let selected_marker = if app.selected_files.contains(&node.path) {
                "[x] "
            } else {
                "[ ] "
            };

            let icon = file_type_icon(&node.file_type);
            let style = if i == relative_selected {
                Style::default()
                    .fg(file_type_color(&node.file_type))
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default().fg(file_type_color(&node.file_type))
            };

            ListItem::new(Line::from(Span::styled(
                format!("{}{} {}", selected_marker, icon, node.name),
                style,
            )))
        })
        .collect();

    let title = format!(
        " Files ({}/{}) ",
        app.file_tree.visible_count(),
        app.file_count
    );

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));

    frame.render_widget(list, area);
}

/// Draw file details panel
fn draw_file_details(frame: &mut Frame, area: Rect, app: &App) {
    let detail_text = if let Some(path) = app.file_tree.selected_path() {
        vec![
            Line::from(Span::styled(
                "  Selected File",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!("  Path: {}", path)),
            Line::from(""),
            Line::from(format!("  Selected: {} files", app.selected_files.len())),
        ]
    } else {
        vec![Line::from(""), Line::from("  No file selected")]
    };

    let block = Block::default().borders(Borders::ALL).title(" Details ");

    let paragraph = Paragraph::new(detail_text).block(block);
    frame.render_widget(paragraph, area);
}

/// Draw search tab
fn draw_search_tab(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    // Search input
    let search_style = if app.state == AppState::SearchInput {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };

    let search_text = if app.state == AppState::SearchInput {
        format!("/{}_", app.filter)
    } else if app.filter.is_empty() {
        "Press '/' to search...".to_string()
    } else {
        format!("Filter: {}", app.filter)
    };

    let search = Paragraph::new(search_text)
        .style(search_style)
        .block(Block::default().borders(Borders::ALL).title(" Search "));

    frame.render_widget(search, chunks[0]);

    // Results
    draw_file_list(frame, chunks[1], app);
}

/// Draw export tab
fn draw_export_tab(frame: &mut Frame, area: Rect, app: &App) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Export",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("  Selected: {} files", app.selected_files.len())),
        Line::from(""),
        Line::from("  Use Space to select files in the Files tab,"),
        Line::from("  then run `diamond-drill export` to export."),
    ];

    let block = Block::default().borders(Borders::ALL).title(" Export ");

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

/// Draw an info tab with a message
fn draw_info_tab(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", title),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("  {}", message)),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title));

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

/// Draw status bar
fn draw_status_bar(frame: &mut Frame, area: Rect, app: &App) {
    let left = Span::styled(
        format!(" {} ", app.status_message),
        Style::default().fg(Color::White),
    );

    let right = Span::styled(
        " ?:Help  j/k:Nav  Space:Select  /:Search  Tab:Switch  q:Quit ",
        Style::default().fg(Color::DarkGray),
    );

    let bar = Paragraph::new(Line::from(vec![left, right])).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    frame.render_widget(bar, area);
}

/// Draw help overlay popup
fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    // Center the popup
    let popup_width = 60.min(area.width.saturating_sub(4));
    let popup_height = 24.min(area.height.saturating_sub(4));
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  KEYBOARD SHORTCUTS",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Navigation",
            Style::default().fg(Color::Yellow),
        )),
        Line::from("    j / ↓        Move down"),
        Line::from("    k / ↑        Move up"),
        Line::from("    g / Home     Jump to first"),
        Line::from("    G / End      Jump to last"),
        Line::from("    PgUp/PgDn    Page up/down"),
        Line::from("    h / ←        Collapse (reserved)"),
        Line::from("    l / →        Expand (reserved)"),
        Line::from(""),
        Line::from(Span::styled(
            "  Selection",
            Style::default().fg(Color::Yellow),
        )),
        Line::from("    Space/Enter  Toggle selection"),
        Line::from("    a            Select all visible"),
        Line::from("    n            Deselect all"),
        Line::from("    i            Invert selection"),
        Line::from(""),
        Line::from(Span::styled(
            "  Document Touching",
            Style::default().fg(Color::Yellow),
        )),
        Line::from("    o            Open in system viewer"),
        Line::from("    r            Reveal in explorer"),
        Line::from(""),
        Line::from(Span::styled(
            "  Tabs & Search",
            Style::default().fg(Color::Yellow),
        )),
        Line::from("    Tab/Shift+Tab Switch tabs"),
        Line::from("    1-5          Jump to tab"),
        Line::from("    /            Enter search mode"),
        Line::from(""),
        Line::from(Span::styled(
            "  General",
            Style::default().fg(Color::Yellow),
        )),
        Line::from("    ?/F1         Show this help"),
        Line::from("    q/Esc        Quit"),
        Line::from("    Ctrl+C       Force quit"),
        Line::from(""),
        Line::from(Span::styled(
            "  Press any key to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Help ")
        .title_alignment(Alignment::Center)
        .padding(Padding::horizontal(1));

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, popup_area);
}

/// Draw the Dedup tab — summary + scrollable group list
fn draw_dedup_tab(frame: &mut Frame, area: Rect, app: &App) {
    let report = match &app.dedup_report {
        Some(r) => r,
        None => {
            draw_info_tab(
                frame,
                area,
                "Dedup",
                "Press 'd' to run dedup analysis on indexed files.",
            );
            return;
        }
    };

    // Split: summary top, groups below
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(3)])
        .split(area);

    // Summary
    let summary = vec![
        Line::from(vec![
            Span::styled("  Scanned: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} files", report.scanned_files),
                Style::default().fg(Color::White),
            ),
            Span::raw("  | "),
            Span::styled("Unique: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", report.unique_files),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Groups: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", report.duplicate_groups),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  | "),
            Span::styled("Duplicates: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", report.total_duplicates),
                Style::default().fg(Color::Red),
            ),
            Span::raw("  | "),
            Span::styled("Wasted: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                humansize::format_size(report.wasted_bytes, humansize::BINARY),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Strategy: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&report.strategy, Style::default().fg(Color::Cyan)),
            Span::raw("  | "),
            Span::styled("Threshold: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}%", report.fuzzy_threshold),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];

    let summary_block = Block::default()
        .borders(Borders::ALL)
        .title(" Dedup Summary ")
        .border_style(Style::default().fg(Color::Yellow));

    frame.render_widget(Paragraph::new(summary).block(summary_block), chunks[0]);

    // Group list
    let inner_height = chunks[1].height.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (i, group) in report.groups.iter().enumerate() {
        let kind_label = if group.similarity == 100 {
            Span::styled(
                "EXACT",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                format!("FUZZY {}%", group.similarity),
                Style::default().fg(Color::Yellow),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  #{:<3} ", i + 1),
                Style::default().fg(Color::DarkGray),
            ),
            kind_label,
            Span::raw("  "),
            Span::styled(
                humansize::format_size(group.wasted_bytes, humansize::BINARY),
                Style::default().fg(Color::Red),
            ),
            Span::styled(" wasted", Style::default().fg(Color::DarkGray)),
        ]));

        // Master
        let master_name = group
            .master
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| group.master.display().to_string());
        lines.push(Line::from(vec![
            Span::styled(
                "    KEEP  ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(master_name, Style::default().fg(Color::White)),
        ]));

        // Duplicates (up to 5)
        for dup in group.duplicates.iter().take(5) {
            let dup_name = dup
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| dup.display().to_string());
            lines.push(Line::from(vec![
                Span::styled("    DEL   ", Style::default().fg(Color::Red)),
                Span::styled(dup_name, Style::default().fg(Color::DarkGray)),
            ]));
        }
        if group.duplicates.len() > 5 {
            lines.push(Line::from(Span::styled(
                format!("    ... +{} more", group.duplicates.len() - 5),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::from(""));
    }

    // Apply scroll
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(app.dedup_scroll)
        .take(inner_height)
        .collect();

    let groups_block = Block::default()
        .borders(Borders::ALL)
        .title(" Duplicate Groups ([ / ] to scroll, 'd' to refresh) ")
        .border_style(Style::default().fg(Color::Yellow));

    frame.render_widget(Paragraph::new(visible_lines).block(groups_block), chunks[1]);
}

/// Draw the Bad Sectors tab — summary + heatmap bars
fn draw_badsector_tab(frame: &mut Frame, area: Rect, app: &App) {
    if app.bad_sector_maps.is_empty() {
        draw_info_tab(
            frame,
            area,
            "Bad Sectors",
            "Press 'b' to scan indexed files for bad sectors.",
        );
        return;
    }

    // Split: summary top, file list below
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
            Span::styled(
                "  Files with errors: ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{}", app.bad_sector_maps.len()),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Total bad blocks: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", total_bad_blocks),
                Style::default().fg(Color::Red),
            ),
            Span::raw("  | "),
            Span::styled("Unreadable: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                humansize::format_size(total_bad, humansize::BINARY),
                Style::default().fg(Color::Red),
            ),
        ]),
    ];

    let summary_block = Block::default()
        .borders(Borders::ALL)
        .title(" Bad Sector Summary ")
        .border_style(Style::default().fg(Color::Red));

    frame.render_widget(Paragraph::new(summary).block(summary_block), chunks[0]);

    // File heatmaps
    let inner_height = chunks[1].height.saturating_sub(2) as usize;
    let heatmap_width = chunks[1].width.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for map in &app.bad_sector_maps {
        let filename = map
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| map.path.display().to_string());

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", filename),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("({:.1}% ok)", map.readable_percent()),
                Style::default().fg(if map.readable_percent() > 95.0 {
                    Color::Green
                } else if map.readable_percent() > 50.0 {
                    Color::Yellow
                } else {
                    Color::Red
                }),
            ),
        ]));

        // Heatmap bar
        let heatmap = map.heatmap();
        let bar = heatmap.summary_bar(heatmap_width.min(60));
        lines.push(Line::from(vec![
            Span::raw("  ["),
            Span::styled(bar, Style::default().fg(Color::Red)),
            Span::raw("]"),
        ]));

        // Bad block details (first 3)
        for block in map.bad_blocks.iter().take(3) {
            lines.push(Line::from(Span::styled(
                format!(
                    "    offset 0x{:08X}, {} bytes, {} retries",
                    block.offset, block.length, block.retry_count
                ),
                Style::default().fg(Color::DarkGray),
            )));
        }
        if map.bad_blocks.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("    ... +{} more bad blocks", map.bad_blocks.len() - 3),
                Style::default().fg(Color::DarkGray),
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
        .title(" Sector Heatmaps ([ / ] to scroll, 'b' to rescan) ")
        .border_style(Style::default().fg(Color::Red));

    frame.render_widget(
        Paragraph::new(visible_lines).block(heatmap_block),
        chunks[1],
    );
}

/// Get icon for file type
fn file_type_icon(ft: &FileType) -> &'static str {
    match ft {
        FileType::Image => "[IMG]",
        FileType::Video => "[VID]",
        FileType::Audio => "[AUD]",
        FileType::Document => "[DOC]",
        FileType::Archive => "[ARC]",
        FileType::Code => "[COD]",
        FileType::Executable => "[EXE]",
        FileType::Database => "[DB ]",
        FileType::Other => "[---]",
    }
}

/// Get color for file type
fn file_type_color(ft: &FileType) -> Color {
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
