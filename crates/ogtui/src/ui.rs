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
            Constraint::Length(22), // tabs
            Constraint::Min(20),   // step progress / info
        ])
        .split(area);

    // Compute per-tab hit rects inside the tabs area
    let tabs_inner = header_chunks[0];
    let tab_count = Tab::ALL.len() as u16;
    let per_tab_w = if tab_count > 0 { tabs_inner.width / tab_count } else { 0 };
    regions.tab_rects = (0..tab_count)
        .map(|i| Rect::new(tabs_inner.x + i * per_tab_w, tabs_inner.y, per_tab_w, tabs_inner.height))
        .collect();

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
            format!("…{}", &last_seg[last_seg.len().saturating_sub(max_title_len.saturating_sub(1))..])
        }
    } else {
        tag.to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            format!(" {} ", short_tag),
            title_style,
        ));

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
        ("j / ↓", "Scroll logs down"),
        ("k / ↑", "Scroll logs up"),
        ("l / →", "Next metric"),
        ("h / ←", "Previous metric"),
        ("Enter / Click", "Enlarge metric"),
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
    let hints = Line::from(vec![
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
    ]);

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
            let p = Paragraph::new("No data").style(Style::default().fg(TEXT_DIM)).block(block);
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
    let y_hi = if (y_max - y_min).abs() < 1e-12 { y_max + 1.0 } else { y_max + y_margin };
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
