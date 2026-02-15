mod app;
mod tfevents;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind, MouseButton},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io;
use std::path::PathBuf;

use app::App;

/// opengraphs TUI — terminal-native experiment tracking viewer
#[derive(Parser)]
#[command(name = "ogtui", version, about)]
struct Cli {
    /// Path to a directory containing .tfevents files, or a single .tfevents file
    #[arg(short, long, default_value = "runs/")]
    path: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // ── Parse TF events via incremental reader ───────────────────────────
    let reader = tfevents::IncrementalReader::new(&cli.path)
        .with_context(|| format!("loading events from {}", cli.path.display()))?;

    let app = App::new(
        reader.scalars.clone(),
        reader.log_lines.clone(),
        cli.path.clone(),
        reader.total_events,
        reader.max_step,
    );

    // ── Setup terminal ──────────────────────────────────────────────────
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, app, reader);

    // ── Restore terminal ────────────────────────────────────────────────
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e:?}");
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App, mut reader: tfevents::IncrementalReader) -> Result<()> {
    // Track layout regions for mouse hit-testing
    let mut layout = ui::LayoutRegions::default();

    // Poll for new data every ~3 seconds (15 ticks × 200ms)
    const POLL_INTERVAL: u64 = 15;

    loop {
        app.tick_count = app.tick_count.wrapping_add(1);

        // Periodically check for new data from tfevents files
        if app.tick_count % POLL_INTERVAL == 0 {
            if let Ok(true) = reader.poll() {
                // New data found — update app state
                let new_tags: Vec<String> = reader.scalars.keys().cloned().collect();
                app.scalars = reader.scalars.clone();
                app.tags = new_tags;
                app.run_names = reader.run_names.clone();
                app.log_lines = reader.log_lines.clone();
                app.total_events = reader.total_events;
                app.max_step = reader.max_step;
            }
        }

        terminal.draw(|f| {
            layout = ui::draw(f, &mut app);
        })?;

        // Poll with timeout so we get periodic redraws for title scroll animation
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Help modal intercepts keys
                if app.show_help {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('?') => app.show_help = false,
                        _ => {}
                    }
                    continue;
                }

                // Focused metric view intercepts Esc
                if app.focused_metric.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            app.unfocus_metric();
                            continue;
                        }
                        KeyCode::Char('q') => {
                            app.should_quit = true;
                            return Ok(());
                        }
                        _ => continue,
                    }
                }

                match key.code {
                    KeyCode::Char('q') => {
                        app.should_quit = true;
                        return Ok(());
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.should_quit = true;
                        return Ok(());
                    }
                    KeyCode::Char('?') => app.toggle_help(),
                    KeyCode::Tab => app.cycle_tab(),
                    KeyCode::BackTab => app.cycle_tab(),
                    KeyCode::Char('j') | KeyCode::Down => {
                        match app.active_tab {
                            app::Tab::Graphs => app.scroll_metrics_down(),
                            app::Tab::Logs => app.scroll_logs_down(),
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        match app.active_tab {
                            app::Tab::Graphs => app.scroll_metrics_up(),
                            app::Tab::Logs => app.scroll_logs_up(),
                        }
                    }
                    KeyCode::Char('l') | KeyCode::Right => app.next_metric(),
                    KeyCode::Char('h') | KeyCode::Left => app.prev_metric(),
                    KeyCode::Enter => app.focus_metric(app.selected_metric),
                    KeyCode::Esc => app.unfocus_metric(),
                    _ => {}
                }
            }
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        let x = mouse.column;
                        let y = mouse.row;

                        // Close help modal on any click
                        if app.show_help {
                            app.show_help = false;
                            continue;
                        }

                        // Click on tab headers
                        for (i, tab_rect) in layout.tab_rects.iter().enumerate() {
                            if x >= tab_rect.x
                                && x < tab_rect.x + tab_rect.width
                                && y >= tab_rect.y
                                && y < tab_rect.y + tab_rect.height
                            {
                                if let Some(&tab) = app::Tab::ALL.get(i) {
                                    app.active_tab = tab;
                                }
                            }
                        }

                        // Click on metric cards: select, then focus if already selected
                        // Card rects are indexed from 0 but correspond to scrolled indices
                        let scroll_offset = app.metrics_scroll * app.metrics_cols;
                        for (vis_i, card_rect) in layout.metric_card_rects.iter().enumerate() {
                            if x >= card_rect.x
                                && x < card_rect.x + card_rect.width
                                && y >= card_rect.y
                                && y < card_rect.y + card_rect.height
                            {
                                let actual_idx = scroll_offset + vis_i;
                                if app.selected_metric == actual_idx {
                                    // Already selected → focus (enlarge)
                                    app.focus_metric(actual_idx);
                                } else {
                                    app.selected_metric = actual_idx;
                                }
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        match app.active_tab {
                            app::Tab::Graphs => app.scroll_metrics_down(),
                            app::Tab::Logs => app.scroll_logs_down(),
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        match app.active_tab {
                            app::Tab::Graphs => app.scroll_metrics_up(),
                            app::Tab::Logs => app.scroll_logs_up(),
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
