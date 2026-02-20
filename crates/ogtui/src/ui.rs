use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph, Tabs, Wrap},
};

use crate::app::{App, ProcessSort, Tab};

// ── Colors (matching the TypeScript TUI) ────────────────────────────────────
const GREEN: Color = Color::Rgb(46, 204, 113); // #2ecc71
const BORDER: Color = Color::Rgb(107, 114, 128); // #6b7280
const TEXT_DIM: Color = Color::Rgb(155, 163, 175); // #9BA3AF
const TEXT_LIGHT: Color = Color::Rgb(209, 213, 219); // #d1d5db
const BG_DARK: Color = Color::Rgb(13, 17, 23); // #0d1117
const CHART_RAW: Color = Color::Rgb(100, 149, 237); // cornflower blue for raw data
const CHART_SMOOTH: Color = Color::Rgb(255, 165, 0); // orange for smoothed
const LOG_INFO_ASH: Color = Color::Rgb(148, 163, 184); // ash
const LOG_ERROR: Color = Color::Rgb(248, 113, 113); // bright red
const LOG_IMPORTANT: Color = Color::Rgb(251, 191, 36); // bright amber

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
            Constraint::Length(3), // header tabs
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
            Tab::Processes => draw_processes_tab(f, app, root_chunks[1]),
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
    // Keep the tab box tight to exactly the visible tab labels.
    let tab_padding: u16 = 1; // ratatui Tabs default padding per side
    let tab_divider_w: u16 = 1; // "│"
    let labels_w: u16 = Tab::ALL
        .iter()
        .map(|t| (t.title().len() as u16) + (tab_padding * 2))
        .sum();
    let dividers_w = tab_divider_w.saturating_mul(Tab::ALL.len().saturating_sub(1) as u16);
    let tabs_required_w = labels_w.saturating_add(dividers_w).saturating_add(2); // block left/right borders

    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(tabs_required_w), // tabs (tight fit)
            Constraint::Min(12),                 // step progress / info
        ])
        .split(area);

    // Compute per-tab hit rects matching ratatui Tabs rendering:
    // Each tab renders as: [1 padding] title [1 padding] [divider]
    // The block border takes 1 col on each side.
    let tabs_outer = header_chunks[0];
    let inner_x = tabs_outer.x + 1; // after left border
    let inner_w = tabs_outer.width.saturating_sub(2); // minus both borders
    let tab_count = Tab::ALL.len();
    let divider_w: u16 = tab_divider_w;
    let padding: u16 = tab_padding;
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
            regions
                .tab_rects
                .push(Rect::new(cur_x, tabs_outer.y, full_w, tabs_outer.height));
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
            .title(Span::styled(" step progress ", Style::default().fg(BORDER))),
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

    let card_width = (inner.width / 4).max(1);
    let cols = (inner.width / card_width).max(1) as usize;
    let card_height: u16 = 12;
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

    // Shorten tag for display: show last path component if tag has slashes
    let max_title_len = (area.width as usize).saturating_sub(4);
    let short_tag = if tag.len() > max_title_len {
        // Try to show the last segment after '/'
        let last_seg = tag.rsplit('/').next().unwrap_or(tag);
        if last_seg.len() <= max_title_len {
            last_seg.to_string()
        } else {
            format!(
                "…{}",
                &last_seg[last_seg
                    .len()
                    .saturating_sub(max_title_len.saturating_sub(1))..]
            )
        }
    } else {
        tag.to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(format!(" {} ", short_tag), title_style));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 3 {
        return;
    }

    if let Some(data) = app.scalars.get(tag) {
        if data.is_empty() {
            let p = Paragraph::new("--").style(Style::default().fg(TEXT_DIM));
            f.render_widget(p, inner);
            return;
        }

        // Show latest value as text at bottom
        let latest = data.last().unwrap();
        let latest_text = format_value(latest.1);

        // Compute bounds
        let x_min = data.first().unwrap().0;
        let x_max = data.last().unwrap().0.max(x_min + 1.0);
        let y_min = data.iter().map(|d| d.1).fold(f64::INFINITY, f64::min);
        let y_max = data.iter().map(|d| d.1).fold(f64::NEG_INFINITY, f64::max);
        let y_margin = (y_max - y_min).abs() * 0.1;
        let y_lo = y_min - y_margin;
        let y_hi = if (y_max - y_min).abs() < 1e-12 {
            y_max + 1.0
        } else {
            y_max + y_margin
        };

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

        // Build dataset
        let dataset = Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(CHART_RAW))
            .data(data);

        let chart = Chart::new(vec![dataset])
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
            Style::default()
                .fg(CHART_SMOOTH)
                .add_modifier(Modifier::BOLD),
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

fn style_for_log_line(line: &str) -> Style {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("[error]") {
        return Style::default().fg(LOG_ERROR).add_modifier(Modifier::BOLD);
    }
    if lower.starts_with("[sucess]") || lower.starts_with("[success]") {
        return Style::default().fg(GREEN).add_modifier(Modifier::BOLD);
    }
    if lower.starts_with("[important]") {
        return Style::default()
            .fg(LOG_IMPORTANT)
            .add_modifier(Modifier::BOLD);
    }
    if lower.starts_with("[info]") {
        return Style::default().fg(LOG_INFO_ASH);
    }
    if trimmed.starts_with("--") {
        return Style::default().fg(BORDER);
    }

    Style::default().fg(TEXT_LIGHT)
}

fn draw_logs_tab(f: &mut Frame, app: &mut App, area: Rect) {
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

    let inner = block.inner(area);
    let viewport_rows = inner.height as usize;
    app.set_logs_viewport_rows(viewport_rows);
    let scroll_y = app.logs_scroll;

    let lines: Vec<Line> = app
        .log_lines
        .iter()
        .map(|line| {
            let style = style_for_log_line(line);
            Line::from(Span::styled(line.as_str(), style))
        })
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_y, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

// ── Processes Tab ───────────────────────────────────────────────────────────

fn state_style(state: &str) -> Style {
    let first = state.chars().next().unwrap_or('S');
    match first {
        'R' => Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        'D' | 'Z' => Style::default().fg(LOG_ERROR).add_modifier(Modifier::BOLD),
        'T' => Style::default().fg(LOG_IMPORTANT),
        _ => Style::default().fg(TEXT_DIM),
    }
}

fn state_label(state: &str) -> &'static str {
    match state.chars().next().unwrap_or('S') {
        'R' => "running",
        'S' => "sleeping",
        'D' => "io-wait",
        'T' => "stopped",
        'Z' => "zombie",
        'I' => "idle",
        _ => "other",
    }
}

fn truncate_text(input: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    let count = input.chars().count();
    if count <= max_len {
        return input.to_string();
    }
    if max_len <= 3 {
        return ".".repeat(max_len);
    }
    let keep = max_len - 3;
    let mut out = String::new();
    for ch in input.chars().take(keep) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn format_ago(delta_secs: u64) -> String {
    if delta_secs < 60 {
        format!("{}s ago", delta_secs)
    } else if delta_secs < 3600 {
        format!("{}m ago", delta_secs / 60)
    } else if delta_secs < 86400 {
        format!("{}h ago", delta_secs / 3600)
    } else {
        format!("{}d ago", delta_secs / 86400)
    }
}

fn draw_processes_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" processes ", Style::default().fg(BORDER)));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 10 || inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let line_width = inner.width.saturating_sub(2) as usize;

    let mut running = app.running_processes.clone();
    running.sort_by(|a, b| match app.process_sort {
        ProcessSort::Cpu => b
            .cpu_pct
            .partial_cmp(&a.cpu_pct)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.mem_pct.partial_cmp(&a.mem_pct).unwrap_or(Ordering::Equal))
            .then_with(|| a.pid.cmp(&b.pid)),
        ProcessSort::Mem => b
            .mem_pct
            .partial_cmp(&a.mem_pct)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(Ordering::Equal))
            .then_with(|| a.pid.cmp(&b.pid)),
        ProcessSort::Pid => a
            .pid
            .cmp(&b.pid)
            .then_with(|| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(Ordering::Equal)),
        ProcessSort::Etime => b
            .elapsed_secs
            .cmp(&a.elapsed_secs)
            .then_with(|| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(Ordering::Equal))
            .then_with(|| a.pid.cmp(&b.pid)),
    });

    let high_cpu = running.iter().filter(|p| p.cpu_pct >= 25.0).count();
    let zombies = running.iter().filter(|p| p.state.starts_with('Z')).count();
    let running_total = running.len();
    running.truncate(app.process_limit);
    let running_shown = running.len();

    lines.push(Line::from(Span::styled(
        format!(
            "live processes: {} running | {} high-cpu | {} zombie",
            running_total, high_cpu, zombies
        ),
        Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "sorted by {} | showing {}",
            app.process_sort.label(),
            running_shown
        ),
        Style::default().fg(BORDER),
    )));

    if running.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no running processes captured yet)",
            Style::default().fg(TEXT_DIM),
        )));
    } else {
        for p in &running {
            let style = state_style(&p.state);
            let row = truncate_text(
                &format!(
                    "[{}] {} | up {} | cpu {:>5.1}% | mem {:>4.1}% | ppid {}",
                    p.pid,
                    state_label(&p.state),
                    p.elapsed,
                    p.cpu_pct,
                    p.mem_pct,
                    p.ppid
                ),
                line_width,
            );
            lines.push(Line::from(Span::styled(row, style)));
            let cmd = truncate_text(&format!("  {}", p.command), line_width);
            lines.push(Line::from(Span::styled(
                cmd,
                Style::default().fg(TEXT_LIGHT),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("recently exited ({})", app.exited_processes.len()),
        Style::default()
            .fg(LOG_IMPORTANT)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "newest first",
        Style::default().fg(BORDER),
    )));

    if app.exited_processes.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no exited processes observed yet)",
            Style::default().fg(TEXT_DIM),
        )));
    } else {
        let now = unix_now_secs();
        for p in &app.exited_processes {
            let ago = format_ago(now.saturating_sub(p.exited_at_unix));
            let row = truncate_text(
                &format!(
                    "[{}] exited {} | was {} | up {} | cpu {:>5.1}% | mem {:>4.1}%",
                    p.snapshot.pid,
                    ago,
                    state_label(&p.snapshot.state),
                    p.snapshot.elapsed,
                    p.snapshot.cpu_pct,
                    p.snapshot.mem_pct
                ),
                line_width,
            );
            lines.push(Line::from(Span::styled(row, Style::default().fg(TEXT_DIM))));
            let cmd = truncate_text(&format!("  {}", p.snapshot.command), line_width);
            lines.push(Line::from(Span::styled(
                cmd,
                Style::default().fg(TEXT_LIGHT),
            )));
        }
    }

    app.set_processes_viewport_rows(inner.height as usize);
    app.set_processes_total_rows(lines.len());

    let paragraph = Paragraph::new(lines)
        .scroll((app.processes_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

// ── Chat Tab ────────────────────────────────────────────────────────────

const CHAT_USER: Color = Color::Rgb(52, 211, 153); // emerald for user
const CHAT_AGENT: Color = Color::Rgb(96, 165, 250); // blue for agent
const CHAT_SYSTEM: Color = Color::Rgb(107, 114, 128); // dim gray for system
// reserved: const CHAT_INPUT_BG: Color = Color::Rgb(30, 35, 44);

fn draw_chat_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
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
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        let prompt = Paragraph::new(Line::from(vec![
            Span::styled(" Press ", Style::default().fg(TEXT_LIGHT)),
            Span::styled("y", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(" to apply │ ", Style::default().fg(TEXT_LIGHT)),
            Span::styled(
                "n",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
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
    let thinking_indicator = if app.agent_thinking {
        " 🔄 thinking..."
    } else {
        ""
    };
    let status_text = format!(
        " {} │ {}{}",
        app.chat_status,
        app.daemon_socket.display(),
        thinking_indicator,
    );
    let status = Paragraph::new(Line::from(Span::styled(status_text, status_style)));
    f.render_widget(status, chunks[4]);
}

fn draw_chat_messages(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            " agent chat ",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ));

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

    let mut lines: Vec<Line> = Vec::new();
    for msg in app.chat_messages.iter() {
        let (prefix, color) = match msg.sender.as_str() {
            "user" => ("You", CHAT_USER),
            "agent" => ("Agent", CHAT_AGENT),
            "system" => ("System", CHAT_SYSTEM),
            other => (other, TEXT_DIM),
        };

        lines.push(Line::from(vec![Span::styled(
            format!("[{}] ", prefix),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )]));

        for l in msg.content.lines() {
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
            lines.push(Line::from(Span::styled(format!("  {}", l), style)));
        }
        lines.push(Line::from(""));
    }

    let viewport_rows = inner.height as usize;
    let max_scroll = lines.len().saturating_sub(viewport_rows) as u16;
    if app.chat_follow_tail {
        app.chat_scroll = max_scroll;
    } else if app.chat_scroll > max_scroll {
        app.chat_scroll = max_scroll;
    }
    if app.chat_scroll >= max_scroll {
        app.chat_follow_tail = true;
    }

    let paragraph = Paragraph::new(lines)
        .scroll((app.chat_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}

fn draw_chat_input(f: &mut Frame, app: &App, area: Rect) {
    let border_color = if app.chat_input_focused {
        GREEN
    } else {
        BORDER
    };
    let title = if app.chat_input_focused {
        " type message or !og command (Enter=send, Esc=unfocus) "
    } else {
        " press 'i' to type "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, Style::default().fg(border_color)));

    let display_text = if app.chat_input.is_empty() && !app.chat_input_focused {
        "Type a message or !og run demo_train.py ...".to_string()
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
        ("F6", "Toggle copy mode (highlight/copy text with mouse)"),
        ("Esc", "Close help / exit detail"),
        ("j / ↓", "Scroll logs/procs/chat down"),
        ("k / ↑", "Scroll logs/procs/chat up"),
        ("l / →", "Next metric"),
        ("h / ←", "Previous metric"),
        ("Enter / Click", "Enlarge metric"),
        ("i", "Focus chat input"),
        ("Enter (chat)", "Send message"),
        ("!og ...", "Run CLI commands in chat"),
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

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, modal_area);
}

// ── Footer ──────────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, _app: &App, area: Rect) {
    if _app.copy_mode {
        let banner = Line::from(vec![
            Span::styled(
                "COPY MODE",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " │ Drag mouse to highlight/copy text │ F6 resume interactive mode",
                Style::default().fg(TEXT_LIGHT),
            ),
        ]);
        let footer = Paragraph::new(banner).alignment(Alignment::Left);
        f.render_widget(footer, area);
        return;
    }

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
            Span::styled("F6", Style::default().fg(GREEN)),
            Span::styled(" copy │ ", Style::default().fg(BORDER)),
            Span::styled("?", Style::default().fg(GREEN)),
            Span::styled(" help │ ", Style::default().fg(BORDER)),
            Span::styled("q", Style::default().fg(GREEN)),
            Span::styled(" quit", Style::default().fg(BORDER)),
            Span::raw("    "),
            Span::styled("opengraphs", Style::default().fg(BORDER)),
        ])
    } else if _app.active_tab == Tab::Graphs {
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
            Span::styled(" scroll │ ", Style::default().fg(BORDER)),
            Span::styled("F6", Style::default().fg(GREEN)),
            Span::styled(" copy", Style::default().fg(BORDER)),
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
            Span::styled("j/k", Style::default().fg(GREEN)),
            Span::styled(" scroll │ ", Style::default().fg(BORDER)),
            Span::styled("F6", Style::default().fg(GREEN)),
            Span::styled(" copy", Style::default().fg(BORDER)),
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
            let p = Paragraph::new("Invalid metric index").style(Style::default().fg(TEXT_DIM));
            f.render_widget(p, area);
            return;
        }
    };

    let data = match app.scalars.get(tag) {
        Some(d) if !d.is_empty() => d,
        _ => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(GREEN))
                .title(Span::styled(
                    format!(" {} ", tag),
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ));
            let p = Paragraph::new("No data")
                .style(Style::default().fg(TEXT_DIM))
                .block(block);
            f.render_widget(p, area);
            return;
        }
    };

    // Compute statistics
    let x_min = data.first().unwrap().0;
    let x_max = data.last().unwrap().0.max(x_min + 1.0);
    let y_min = data.iter().map(|d| d.1).fold(f64::INFINITY, f64::min);
    let y_max = data.iter().map(|d| d.1).fold(f64::NEG_INFINITY, f64::max);
    let y_margin = (y_max - y_min).abs() * 0.05;
    let y_lo = y_min - y_margin;
    let y_hi = if (y_max - y_min).abs() < 1e-12 {
        y_max + 1.0
    } else {
        y_max + y_margin
    };
    let latest = data.last().unwrap().1;
    let count = data.len();

    // Stats line
    let stats_text = format!(
        "latest: {}  │  min: {}  │  max: {}  │  points: {}  │  steps: {:.0}–{:.0}",
        format_value(latest),
        format_value(y_min),
        format_value(y_max),
        count,
        x_min,
        x_max,
    );

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

    let dataset = Dataset::default()
        .name(tag)
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(CHART_RAW));

    let dataset = dataset.data(data);

    let chart = Chart::new(vec![dataset])
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

    // Stats bar
    let stats = Paragraph::new(Line::from(Span::styled(
        stats_text,
        Style::default().fg(CHART_SMOOTH),
    )))
    .alignment(Alignment::Center);
    f.render_widget(stats, chunks[1]);
}
