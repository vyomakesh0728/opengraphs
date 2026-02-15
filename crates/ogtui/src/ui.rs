use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Clear, Dataset, GraphType, List, ListItem,
        Paragraph, Tabs, Wrap,
    },
};

use crate::app::{App, Tab};

// ── Colors (matching the TypeScript TUI) ────────────────────────────────────
const GREEN: Color = Color::Rgb(46, 204, 113);   // #2ecc71
const BORDER: Color = Color::Rgb(107, 114, 128);  // #6b7280
const TEXT_DIM: Color = Color::Rgb(155, 163, 175); // #9BA3AF
const TEXT_LIGHT: Color = Color::Rgb(209, 213, 219); // #d1d5db
const BG_DARK: Color = Color::Rgb(13, 17, 23);    // #0d1117
const CHART_RAW: Color = Color::Rgb(100, 149, 237); // cornflower blue for raw data
const CHART_SMOOTH: Color = Color::Rgb(255, 165, 0); // orange for smoothed

/// Distinct colors for different runs (up to 8, then cycles).
const RUN_COLORS: &[Color] = &[
    Color::Rgb(100, 149, 237), // cornflower blue
    Color::Rgb(255, 165, 0),   // orange
    Color::Rgb(46, 204, 113),  // emerald green
    Color::Rgb(231, 76, 60),   // red
    Color::Rgb(155, 89, 182),  // purple
    Color::Rgb(26, 188, 156),  // teal
    Color::Rgb(241, 196, 15),  // yellow
    Color::Rgb(230, 126, 163), // pink
];

fn run_color(run_idx: usize) -> Color {
    RUN_COLORS[run_idx % RUN_COLORS.len()]
}

/// Clickable regions tracked for mouse hit-testing.
#[derive(Default, Clone)]
pub struct LayoutRegions {
    /// One Rect per tab button in the header.
    pub tab_rects: Vec<Rect>,
    /// One Rect per metric card in the graphs grid.
    pub metric_card_rects: Vec<Rect>,
}

/// Main draw function — returns layout regions for mouse handling.
pub fn draw(f: &mut Frame, app: &mut App) -> LayoutRegions {
    let size = f.area();
    let mut regions = LayoutRegions::default();

    // Root vertical layout: header (3) | body (fill) | footer (1)
    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header tabs
            Constraint::Min(10),   // body
            Constraint::Length(1), // footer
        ])
        .split(size);

    draw_header(f, app, root_chunks[0], &mut regions);

    // Fullscreen focused metric takes over the body
    if let Some(idx) = app.focused_metric {
        draw_focused_metric(f, app, idx, root_chunks[1]);
    } else {
        match app.active_tab {
            Tab::Graphs => draw_graphs_tab(f, app, root_chunks[1], &mut regions),
            Tab::Logs => draw_logs_tab(f, app, root_chunks[1]),
            Tab::Chat => draw_chat_tab(f, app, root_chunks[1]),
        }
    }

    draw_footer(f, app, root_chunks[2]);

    // Help overlay on top
    if app.show_help {
        draw_help_modal(f, size);
    }

    regions
}

// ── Header ──────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, app: &App, area: Rect, regions: &mut LayoutRegions) {
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(30), // tabs
            Constraint::Min(20),   // step progress / info
        ])
        .split(area);

    // Compute per-tab hit rects matching ratatui Tabs rendering:
    // Each tab renders as: [1 padding] title [1 padding] [divider]
    // The block border takes 1 col on each side.
    let tabs_outer = header_chunks[0];
    let inner_x = tabs_outer.x + 1; // after left border
    let inner_w = tabs_outer.width.saturating_sub(2); // minus both borders
    let tab_count = Tab::ALL.len();
    let divider_w: u16 = 1; // "│"
    let padding: u16 = 1;   // ratatui Tabs default padding per side
    {
        let mut cur_x = inner_x;
        regions.tab_rects.clear();
        for (i, t) in Tab::ALL.iter().enumerate() {
            let title_w = t.title().len() as u16;
            let tab_w = padding + title_w + padding; // " title "
            let full_w = if i < tab_count - 1 {
                tab_w + divider_w // include divider in clickable area
            } else {
                // last tab: extend to fill remaining inner space
                (inner_x + inner_w).saturating_sub(cur_x)
            };
            regions.tab_rects.push(Rect::new(
                cur_x,
                tabs_outer.y,
                full_w,
                tabs_outer.height,
            ));
            cur_x += full_w;
        }
    }

    // Tab selector
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|t| {
            let style = if *t == app.active_tab {
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TEXT_DIM)
            };
            Line::from(Span::styled(t.title(), style))
        })
        .collect();

    let active_idx = Tab::ALL
        .iter()
        .position(|t| *t == app.active_tab)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER)),
        )
        .highlight_style(Style::default().fg(GREEN))
        .select(active_idx)
        .divider(Span::styled("│", Style::default().fg(BORDER)));

    f.render_widget(tabs, header_chunks[0]);

    // Step progress area
    let step_info = format!(
        " {} tags │ {} events │ step {}",
        app.tags.len(),
        app.total_events,
        app.max_step,
    );
    let step_block = Paragraph::new(Line::from(Span::styled(
        step_info,
        Style::default().fg(TEXT_DIM),
    )))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .title(Span::styled(
                " step progress ",
                Style::default().fg(BORDER),
            )),
    );
    f.render_widget(step_block, header_chunks[1]);
}

// ── Graphs Tab ──────────────────────────────────────────────────────────────

fn draw_graphs_tab(f: &mut Frame, app: &mut App, area: Rect, regions: &mut LayoutRegions) {
    // Split: left metrics (74%) | right side column (26%)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(74), Constraint::Percentage(26)])
        .split(area);

    draw_metrics_grid(f, app, h_chunks[0], regions);
    draw_side_column(f, app, h_chunks[1]);
}

fn draw_metrics_grid(f: &mut Frame, app: &mut App, area: Rect, regions: &mut LayoutRegions) {
    // Calculate grid dimensions first so we can show scroll info
    let temp_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER));
    let inner = temp_block.inner(area);

    // Calculate responsive grid dimensions based on available space
    let min_card_width: u16 = 20;  // Minimum width for a card to be readable
    let cols = (inner.width / min_card_width).max(1) as usize;
    let card_width = inner.width / cols as u16;
    let _ = card_width; // used implicitly by Constraint::Ratio below

    // Card height: scale to show at least 2-3 rows, minimum 8 for readable charts
    let card_height: u16 = if inner.height >= 36 {
        12
    } else if inner.height >= 24 {
        (inner.height / 3).max(8)
    } else {
        (inner.height / 2).max(6)
    };
    let rows_available = (inner.height / card_height).max(1) as usize;
    let total_rows = (app.tags.len() + cols - 1) / cols;

    // Update app with grid dimensions for auto-scroll
    app.metrics_visible_rows = rows_available;
    app.metrics_cols = cols;

    // Clamp scroll
    let max_scroll = total_rows.saturating_sub(rows_available);
    if app.metrics_scroll > max_scroll {
        app.metrics_scroll = max_scroll;
    }

    // Build title with scroll indicator
    let title = if total_rows > rows_available {
        format!(
            " metrics [{}-{}/{}] ",
            app.metrics_scroll * cols + 1,
            ((app.metrics_scroll + rows_available) * cols).min(app.tags.len()),
            app.tags.len(),
        )
    } else {
        format!(" metrics ({}) ", app.tags.len())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(title, Style::default().fg(BORDER)));

    let inner = block.inner(area);
    f.render_widget(block, area);

    regions.metric_card_rects.clear();

    if app.tags.is_empty() {
        let msg = Paragraph::new("No scalar metrics found")
            .style(Style::default().fg(TEXT_DIM))
            .alignment(Alignment::Center);
        f.render_widget(msg, inner);
        return;
    }

    // Build row constraints for visible rows
    let visible_rows = rows_available.min(total_rows.saturating_sub(app.metrics_scroll));
    let row_constraints: Vec<Constraint> = (0..visible_rows)
        .map(|_| Constraint::Length(card_height))
        .collect();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(inner);

    let col_constraints: Vec<Constraint> = (0..cols)
        .map(|_| Constraint::Ratio(1, cols as u32))
        .collect();

    let start_idx = app.metrics_scroll * cols;

    for vis_row in 0..visible_rows {
        if vis_row >= rows.len() {
            break;
        }

        let col_areas = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints.clone())
            .split(rows[vis_row]);

        for col in 0..cols {
            let i = start_idx + vis_row * cols + col;
            if i >= app.tags.len() {
                break;
            }
            if col >= col_areas.len() {
                break;
            }

            let card_area = col_areas[col];
            regions.metric_card_rects.push(card_area);
            let is_selected = i == app.selected_metric;
            draw_metric_card(f, &app, &app.tags[i].clone(), card_area, is_selected);
        }
    }
}

fn draw_metric_card(f: &mut Frame, app: &App, tag: &str, area: Rect, selected: bool) {
    let border_color = if selected { GREEN } else { BORDER };
    let title_style = if selected {
        Style::default().fg(GREEN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BORDER)
    };

    // Shorten tag for display: marquee scroll when selected, static truncate otherwise
    let max_title_len = (area.width as usize).saturating_sub(4);
    let display_tag = if tag.len() > max_title_len {
        if selected {
            // marquee: ping-pong scroll
            let overflow = tag.len() - max_title_len;
            // Each position holds for 1 tick; total cycle = 2 * overflow (ping-pong)
            let cycle = (overflow * 2).max(1);
            let pos = (app.tick_count as usize / 2) % cycle; // /2 to slow it down
            let offset = if pos <= overflow { pos } else { cycle - pos };
            // Slice carefully at char boundaries (ASCII tags are safe)
            let end = (offset + max_title_len).min(tag.len());
            tag[offset..end].to_string()
        } else {
            // Static: show last path segment
            let last_seg = tag.rsplit('/').next().unwrap_or(tag);
            if last_seg.len() <= max_title_len {
                last_seg.to_string()
            } else {
                format!("…{}", &last_seg[last_seg.len().saturating_sub(max_title_len.saturating_sub(1))..])
            }
        }
    } else {
        tag.to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" {} ", display_tag),
            title_style,
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 3 {
        return;
    }

    if let Some(runs) = app.scalars.get(tag) {
        if runs.is_empty() {
            let p = Paragraph::new("--").style(Style::default().fg(TEXT_DIM));
            f.render_widget(p, inner);
            return;
        }

        // Compute bounds across all runs
        let mut x_min = f64::INFINITY;
        let mut x_max = f64::NEG_INFINITY;
        let mut y_min = f64::INFINITY;
        let mut y_max = f64::NEG_INFINITY;
        let mut latest_val: Option<f64> = None;

        for data in runs.values() {
            if data.is_empty() { continue; }
            x_min = x_min.min(data.first().unwrap().0);
            x_max = x_max.max(data.last().unwrap().0);
            for &(_, v) in data {
                y_min = y_min.min(v);
                y_max = y_max.max(v);
            }
            if latest_val.is_none() {
                latest_val = Some(data.last().unwrap().1);
            }
        }

        if x_min >= x_max { x_max = x_min + 1.0; }
        let y_margin = (y_max - y_min).abs() * 0.1;
        let y_lo = y_min - y_margin;
        let y_hi = if (y_max - y_min).abs() < 1e-12 { y_max + 1.0 } else { y_max + y_margin };

        let latest_text = latest_val.map(format_value).unwrap_or_else(|| "--".into());

        // Chart area = inner minus 1 line for value text
        let chart_area_height = inner.height.saturating_sub(1);
        let chart_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: chart_area_height,
        };
        let value_area = Rect {
            x: inner.x,
            y: inner.y + chart_area_height,
            width: inner.width,
            height: 1,
        };

        // Build one dataset per run
        let datasets: Vec<Dataset> = runs.iter().enumerate()
            .map(|(run_idx, (_run_name, data))| {
                let color = run_color(
                    app.run_names.iter().position(|r| r == _run_name).unwrap_or(run_idx)
                );
                Dataset::default()
                    .marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(color))
                    .data(data)
            })
            .collect();

        let chart = Chart::new(datasets)
            .x_axis(
                Axis::default()
                    .bounds([x_min, x_max])
                    .style(Style::default().fg(BORDER)),
            )
            .y_axis(
                Axis::default()
                    .bounds([y_lo, y_hi])
                    .style(Style::default().fg(BORDER)),
            );

        f.render_widget(chart, chart_area);

        // Value label
        let val_label = Paragraph::new(Line::from(Span::styled(
            latest_text,
            Style::default().fg(CHART_SMOOTH).add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Right);
        f.render_widget(val_label, value_area);
    } else {
        let p = Paragraph::new("--").style(Style::default().fg(TEXT_DIM));
        f.render_widget(p, inner);
    }
}

fn draw_side_column(f: &mut Frame, app: &App, area: Rect) {
    draw_stats_panel(f, app, area);
}

fn draw_stats_panel(f: &mut Frame, app: &App, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            format!("path:   {}", app.events_path.display()),
            Style::default().fg(TEXT_DIM),
        )),
        Line::from(Span::styled(
            format!("tags:   {}", app.tags.len()),
            Style::default().fg(TEXT_DIM),
        )),
        Line::from(Span::styled(
            format!("events: {}", app.total_events),
            Style::default().fg(TEXT_DIM),
        )),
        Line::from(Span::styled(
            format!("step:   {}", app.max_step),
            Style::default().fg(TEXT_DIM),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" stats ", Style::default().fg(BORDER)));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

// ── Logs Tab ────────────────────────────────────────────────────────────────

fn draw_logs_tab(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" logs ", Style::default().fg(BORDER)));

    if app.log_lines.is_empty() {
        let msg = Paragraph::new("No events loaded")
            .style(Style::default().fg(TEXT_DIM))
            .block(block)
            .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }

    let items: Vec<ListItem> = app
        .log_lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let style = if i == 0 {
                Style::default().fg(BORDER)
            } else {
                Style::default().fg(TEXT_LIGHT)
            };
            ListItem::new(Line::from(Span::styled(line.as_str(), style)))
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

// ── Chat Tab ────────────────────────────────────────────────────────────

const CHAT_USER: Color = Color::Rgb(52, 211, 153);    // emerald for user
const CHAT_AGENT: Color = Color::Rgb(96, 165, 250);   // blue for agent
const CHAT_SYSTEM: Color = Color::Rgb(107, 114, 128); // dim gray for system
// reserved: const CHAT_INPUT_BG: Color = Color::Rgb(30, 35, 44);

fn draw_chat_tab(f: &mut Frame, app: &App, area: Rect) {
    // Layout: banner (0-2) | messages (fill) | refactor prompt (0 or 3) | input (3) | status (1)
    let has_banner = app.auto_mode;
    let has_refactor = app.pending_refactor.is_some();
    let banner_h = if has_banner { 1 } else { 0 };
    let refactor_h: u16 = if has_refactor { 3 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(banner_h),
            Constraint::Min(5),
            Constraint::Length(refactor_h),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    // Auto-mode banner
    if app.auto_mode {
        let banner = Paragraph::new(Line::from(Span::styled(
            " ⚡ Auto mode: Agent will apply fixes automatically ",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center);
        f.render_widget(banner, chunks[0]);
    }

    // Messages area
    draw_chat_messages(f, app, chunks[1]);

    // Pending refactor approve/reject prompt
    if has_refactor {
        let refactor_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(Span::styled(
                " pending refactor ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        let prompt = Paragraph::new(Line::from(vec![
            Span::styled(" Press ", Style::default().fg(TEXT_LIGHT)),
            Span::styled("y", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(" to apply │ ", Style::default().fg(TEXT_LIGHT)),
            Span::styled("n", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(" to reject", Style::default().fg(TEXT_LIGHT)),
        ]))
        .block(refactor_block);
        f.render_widget(prompt, chunks[2]);
    }

    // Input area
    draw_chat_input(f, app, chunks[3]);

    // Status bar
    let status_style = if app.pending_refactor.is_some() {
        Style::default().fg(Color::Yellow)
    } else if app.daemon_connected {
        Style::default().fg(GREEN)
    } else {
        Style::default().fg(Color::Red)
    };
    let thinking_indicator = if app.agent_thinking { " 🔄 thinking..." } else { "" };
    let status_text = format!(
        " {} │ {}{}",
        app.chat_status,
        app.daemon_socket.display(),
        thinking_indicator,
    );
    let status = Paragraph::new(Line::from(Span::styled(status_text, status_style)));
    f.render_widget(status, chunks[4]);
}

fn draw_chat_messages(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" agent chat ", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)));

    if app.chat_messages.is_empty() && !app.daemon_connected {
        let help_lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Agent chat is not connected.",
                Style::default().fg(TEXT_DIM),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Start with built-in daemon:",
                Style::default().fg(TEXT_DIM),
            )),
            Line::from(Span::styled(
                "  ogtui --path runs/ --training-file train.py",
                Style::default().fg(TEXT_LIGHT),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Or start daemon separately:",
                Style::default().fg(TEXT_DIM),
            )),
            Line::from(Span::styled(
                "  python3 -m og_agent_chat.server --training-file train.py",
                Style::default().fg(TEXT_LIGHT),
            )),
        ];
        let p = Paragraph::new(help_lines)
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
        return;
    }

    if app.chat_messages.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "No messages yet. Type a message and press Enter.",
            Style::default().fg(TEXT_DIM),
        )))
        .block(block)
        .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Render messages as a scrollable list
    let items: Vec<ListItem> = app
        .chat_messages
        .iter()
        .map(|msg| {
            let (prefix, color) = match msg.sender.as_str() {
                "user" => ("You", CHAT_USER),
                "agent" => ("Agent", CHAT_AGENT),
                "system" => ("System", CHAT_SYSTEM),
                other => (other, TEXT_DIM),
            };

            // Wrap long messages into multiple lines
            let header = Line::from(vec![
                Span::styled(
                    format!("[{}] ", prefix),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]);

            let content_lines: Vec<Line> = msg
                .content
                .lines()
                .map(|l| {
                    let style = if l.starts_with('+') && !l.starts_with("+++") {
                        Style::default().fg(GREEN)
                    } else if l.starts_with('-') && !l.starts_with("---") {
                        Style::default().fg(Color::Red)
                    } else if l.starts_with("@@") {
                        Style::default().fg(Color::Cyan)
                    } else if l.starts_with("---") || l.starts_with("+++") {
                        Style::default().fg(TEXT_DIM).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(TEXT_LIGHT)
                    };
                    Line::from(Span::styled(format!("  {}", l), style))
                })
                .collect();

            let mut all_lines = vec![header];
            all_lines.extend(content_lines);
            all_lines.push(Line::from("")); // spacing between messages

            ListItem::new(all_lines)
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

fn draw_chat_input(f: &mut Frame, app: &App, area: Rect) {
    let border_color = if app.chat_input_focused { GREEN } else { BORDER };
    let title = if app.chat_input_focused {
        " type message (Enter=send, Esc=unfocus) "
    } else {
        " press 'i' to type "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, Style::default().fg(border_color)));

    let display_text = if app.chat_input.is_empty() && !app.chat_input_focused {
        "Type a message to the agent...".to_string()
    } else if app.chat_input_focused {
        format!("{}█", app.chat_input)
    } else {
        app.chat_input.clone()
    };

    let style = if app.chat_input.is_empty() && !app.chat_input_focused {
        Style::default().fg(TEXT_DIM)
    } else {
        Style::default().fg(TEXT_LIGHT)
    };

    let input = Paragraph::new(Line::from(Span::styled(display_text, style)))
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(input, area);
}

// ── Help Modal ──────────────────────────────────────────────────────────────

fn draw_help_modal(f: &mut Frame, area: Rect) {
    let w = (area.width * 60 / 100).min(60);
    let h = (area.height * 60 / 100).min(22);
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let modal_area = Rect::new(x, y, w, h);

    f.render_widget(Clear, modal_area);

    let shortcuts = vec![
        ("Tab / Shift+Tab", "Cycle tabs"),
        ("q", "Quit"),
        ("?", "Toggle this help"),
        ("Esc", "Close help / exit detail"),
        ("j / ↓", "Scroll logs/chat down"),
        ("k / ↑", "Scroll logs/chat up"),
        ("l / →", "Next metric"),
        ("h / ←", "Previous metric"),
        ("Enter / Click", "Enlarge metric"),
        ("i", "Focus chat input"),
        ("Enter (chat)", "Send message"),
        ("Esc (chat)", "Unfocus chat input"),
        ("y (chat)", "Apply pending refactor"),
        ("n (chat)", "Reject pending refactor"),
    ];

    let lines: Vec<Line> = shortcuts
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<20}", key),
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ),
                Span::styled(*desc, Style::default().fg(TEXT_LIGHT)),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            " shortcuts ",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(BG_DARK));

    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(paragraph, modal_area);
}

// ── Footer ──────────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, _app: &App, area: Rect) {
    let hints = if _app.active_tab == Tab::Chat {
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(GREEN)),
            Span::styled(" switch │ ", Style::default().fg(BORDER)),
            Span::styled("i", Style::default().fg(GREEN)),
            Span::styled(" type │ ", Style::default().fg(BORDER)),
            Span::styled("Enter", Style::default().fg(GREEN)),
            Span::styled(" send │ ", Style::default().fg(BORDER)),
            Span::styled("j/k", Style::default().fg(GREEN)),
            Span::styled(" scroll │ ", Style::default().fg(BORDER)),
            Span::styled("?", Style::default().fg(GREEN)),
            Span::styled(" help │ ", Style::default().fg(BORDER)),
            Span::styled("q", Style::default().fg(GREEN)),
            Span::styled(" quit", Style::default().fg(BORDER)),
            Span::raw("    "),
            Span::styled("opengraphs", Style::default().fg(BORDER)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(GREEN)),
            Span::styled(" switch │ ", Style::default().fg(BORDER)),
            Span::styled("?", Style::default().fg(GREEN)),
            Span::styled(" help │ ", Style::default().fg(BORDER)),
            Span::styled("q", Style::default().fg(GREEN)),
            Span::styled(" quit │ ", Style::default().fg(BORDER)),
            Span::styled("h/l", Style::default().fg(GREEN)),
            Span::styled(" metrics │ ", Style::default().fg(BORDER)),
            Span::styled("j/k", Style::default().fg(GREEN)),
            Span::styled(" scroll", Style::default().fg(BORDER)),
            Span::raw("    "),
            Span::styled("opengraphs", Style::default().fg(BORDER)),
        ])
    };

    let footer = Paragraph::new(hints).alignment(Alignment::Right);
    f.render_widget(footer, area);
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn format_value(v: f64) -> String {
    if v.abs() < 0.001 && v != 0.0 {
        format!("{:.2e}", v)
    } else if v.abs() >= 10000.0 {
        format!("{:.2e}", v)
    } else {
        format!("{:.4}", v)
    }
}

// ── Focused Metric Detail View ──────────────────────────────────────────────

fn draw_focused_metric(f: &mut Frame, app: &App, metric_idx: usize, area: Rect) {
    let tag = match app.tags.get(metric_idx) {
        Some(t) => t.as_str(),
        None => {
            let p = Paragraph::new("Invalid metric index")
                .style(Style::default().fg(TEXT_DIM));
            f.render_widget(p, area);
            return;
        }
    };

    let runs = match app.scalars.get(tag) {
        Some(r) if !r.is_empty() => r,
        _ => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(GREEN))
                .title(Span::styled(
                    format!(" {} ", tag),
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ));
            let p = Paragraph::new("No data").style(Style::default().fg(TEXT_DIM)).block(block);
            f.render_widget(p, area);
            return;
        }
    };

    // Compute statistics across all runs
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_min = f64::INFINITY;
    let mut y_max = f64::NEG_INFINITY;
    let mut total_points: usize = 0;

    for data in runs.values() {
        if data.is_empty() { continue; }
        x_min = x_min.min(data.first().unwrap().0);
        x_max = x_max.max(data.last().unwrap().0);
        for &(_, v) in data {
            y_min = y_min.min(v);
            y_max = y_max.max(v);
        }
        total_points += data.len();
    }

    if x_min >= x_max { x_max = x_min + 1.0; }
    let y_margin = (y_max - y_min).abs() * 0.05;
    let y_lo = y_min - y_margin;
    let y_hi = if (y_max - y_min).abs() < 1e-12 { y_max + 1.0 } else { y_max + y_margin };

    // Build stats line with per-run legend
    let mut stats_spans: Vec<Span> = Vec::new();
    stats_spans.push(Span::styled(
        format!("min: {}  │  max: {}  │  points: {}  │  steps: {:.0}–{:.0}",
            format_value(y_min), format_value(y_max), total_points, x_min, x_max),
        Style::default().fg(CHART_SMOOTH),
    ));

    if runs.len() > 1 {
        stats_spans.push(Span::styled("  │  ", Style::default().fg(BORDER)));
        for (run_name, data) in runs.iter() {
            let run_idx = app.run_names.iter().position(|r| r == run_name).unwrap_or(0);
            let color = run_color(run_idx);
            let latest = data.last().map(|d| format_value(d.1)).unwrap_or_else(|| "--".into());
            // Short run name (first 8 chars)
            let short_name: String = run_name.chars().take(8).collect();
            stats_spans.push(Span::styled(format!("■ {}:{} ", short_name, latest), Style::default().fg(color)));
        }
    }

    // Layout: chart body | stats line (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(area);

    // X-axis labels
    let x_labels = vec![
        Span::styled(format!("{:.0}", x_min), Style::default().fg(TEXT_DIM)),
        Span::styled(
            format!("{:.0}", (x_min + x_max) / 2.0),
            Style::default().fg(TEXT_DIM),
        ),
        Span::styled(format!("{:.0}", x_max), Style::default().fg(TEXT_DIM)),
    ];

    // Y-axis labels
    let y_labels = vec![
        Span::styled(format_value(y_lo), Style::default().fg(TEXT_DIM)),
        Span::styled(
            format_value((y_lo + y_hi) / 2.0),
            Style::default().fg(TEXT_DIM),
        ),
        Span::styled(format_value(y_hi), Style::default().fg(TEXT_DIM)),
    ];

    // Build one dataset per run with color and name
    let datasets: Vec<Dataset> = runs.iter().enumerate()
        .map(|(i, (run_name, data))| {
            let run_idx = app.run_names.iter().position(|r| r == run_name).unwrap_or(i);
            let color = run_color(run_idx);
            let short_name: String = run_name.chars().take(12).collect();
            Dataset::default()
                .name(short_name)
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(color))
                .data(data)
        })
        .collect();

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(GREEN))
                .title(Span::styled(
                    format!(" {} ", tag),
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ))
                .title_bottom(
                    Line::from(Span::styled(
                        " Esc to close ",
                        Style::default().fg(TEXT_DIM),
                    ))
                    .alignment(Alignment::Right),
                ),
        )
        .x_axis(
            Axis::default()
                .title(Span::styled("step", Style::default().fg(TEXT_DIM)))
                .bounds([x_min, x_max])
                .labels(x_labels)
                .style(Style::default().fg(BORDER)),
        )
        .y_axis(
            Axis::default()
                .title(Span::styled("value", Style::default().fg(TEXT_DIM)))
                .bounds([y_lo, y_hi])
                .labels(y_labels)
                .style(Style::default().fg(BORDER)),
        );

    f.render_widget(chart, chunks[0]);

    // Stats bar with run legend
    let stats = Paragraph::new(Line::from(stats_spans))
        .alignment(Alignment::Center);
    f.render_widget(stats, chunks[1]);
}
