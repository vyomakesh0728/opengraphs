mod app;
mod socket_client;
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
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use app::App;

/// opengraphs TUI — terminal-native experiment tracking viewer
#[derive(Parser)]
#[command(name = "ogtui", version, about)]
struct Cli {
    /// Path to a directory containing .tfevents files, or a single .tfevents file
    #[arg(short, long, default_value = "runs/")]
    path: PathBuf,

    /// Training script to monitor (enables agent daemon)
    #[arg(short = 'f', long)]
    training_file: Option<PathBuf>,

    /// Codebase root for agent indexing (default: current dir)
    #[arg(long, default_value = ".")]
    codebase_root: PathBuf,

    /// Enable auto-refactor mode (agent applies code fixes automatically)
    #[arg(long)]
    auto: bool,

    /// Unix socket path for daemon communication
    #[arg(long, env = "OGD_SOCKET")]
    socket: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // ── Parse TF events ─────────────────────────────────────────────────
    let scalars = tfevents::load_scalars(&cli.path)
        .with_context(|| format!("loading events from {}", cli.path.display()))?;

    // Build log lines from scalar events
    let all_events = if cli.path.is_file() {
        tfevents::parse_events_file(&cli.path)?
    } else {
        // Re-parse to get the log lines ordered by step
        let mut evts = Vec::new();
        if cli.path.is_dir() {
            for entry in std::fs::read_dir(&cli.path).into_iter().flatten() {
                if let Ok(entry) = entry {
                    let p = entry.path();
                    if p.is_dir() {
                        // Look inside subdirectory
                        for sub in std::fs::read_dir(&p).into_iter().flatten() {
                            if let Ok(sub) = sub {
                                let sp = sub.path();
                                if sp.file_name().unwrap_or_default().to_string_lossy().contains("tfevents") {
                                    if let Ok(e) = tfevents::parse_events_file(&sp) {
                                        evts.extend(e);
                                    }
                                }
                            }
                        }
                    } else if p.file_name().unwrap_or_default().to_string_lossy().contains("tfevents") {
                        if let Ok(e) = tfevents::parse_events_file(&p) {
                            evts.extend(e);
                        }
                    }
                }
            }
        }
        evts
    };

    let total_events = all_events.len();
    let max_step = all_events.iter().map(|e| e.step).max().unwrap_or(0);

    // Build log lines
    let mut log_lines = vec!["-- parsed events log --".to_string(), String::new()];
    let mut sorted_events = all_events;
    sorted_events.sort_by_key(|e| e.step);
    for ev in &sorted_events {
        log_lines.push(format!(
            "step {:>6} │ {:<30} │ {:.6}",
            ev.step, ev.tag, ev.value
        ));
    }

    let mut app = App::new(scalars, log_lines, cli.path.clone(), total_events, max_step);

    // Override socket path if provided
    if let Some(ref sock) = cli.socket {
        app.daemon_socket = sock.clone();
    }

    // ── Spawn agent daemon if --training-file is provided ───────────────
    let mut daemon_child: Option<Child> = None;
    if let Some(ref training_file) = cli.training_file {
        match spawn_daemon(training_file, &cli.codebase_root, cli.auto, &app.daemon_socket) {
            Ok(child) => {
                daemon_child = Some(child);
                app.chat_status = "Daemon starting...".to_string();
            }
            Err(e) => {
                app.chat_status = format!("Daemon failed: {}", e);
            }
        }
    }

    // ── Setup terminal ──────────────────────────────────────────────────
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, app);

    // ── Restore terminal ────────────────────────────────────────────────
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
    terminal.show_cursor()?;

    // ── Kill daemon child if we spawned it ──────────────────────────────
    if let Some(ref mut child) = daemon_child {
        let _ = child.kill();
        let _ = child.wait();
    }

    if let Err(e) = result {
        eprintln!("Error: {e:?}");
    }

    Ok(())
}

/// Spawn the Python agent daemon as a child process.
fn spawn_daemon(
    training_file: &PathBuf,
    codebase_root: &PathBuf,
    auto_mode: bool,
    socket_path: &PathBuf,
) -> Result<Child> {
    // Try python3 first, then python
    let python = find_python();

    let mut cmd = Command::new(&python);
    cmd.arg("-m")
        .arg("og_agent_chat.server")
        .arg("--training-file")
        .arg(training_file)
        .arg("--codebase-root")
        .arg(codebase_root)
        .arg("--socket")
        .arg(socket_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if auto_mode {
        cmd.arg("--auto");
    }

    let child = cmd.spawn()
        .with_context(|| format!("Failed to spawn daemon with '{}'. Is og-agent-chat installed?", python))?;

    Ok(child)
}

/// Find a working Python interpreter.
fn find_python() -> String {
    for candidate in ["python3", "python"] {
        if Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return candidate.to_string();
        }
    }
    "python3".to_string()
}

/// Messages from background threads to the main event loop.
enum BgMessage {
    DaemonConnected(bool),
    ChatHistory(Vec<socket_client::ChatMessage>),
    ChatSendResult {
        plan: socket_client::ActionPlanResponse,
        messages: Vec<socket_client::ChatMessage>,
    },
    ChatSendError(String),
    RunStateUpdate {
        auto_mode: bool,
    },
    RefactorApplied {
        success: bool,
        messages: Vec<socket_client::ChatMessage>,
    },
    RefactorError(String),
    LiveMetrics {
        metrics: serde_json::Map<String, serde_json::Value>,
        logs: Vec<String>,
        current_step: i64,
    },
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<()> {
    // Track layout regions for mouse hit-testing
    let mut layout = ui::LayoutRegions::default();

    // Channel for background daemon communication
    let (bg_tx, bg_rx) = mpsc::channel::<BgMessage>();

    // Periodic polling state
    let mut last_poll = Instant::now();
    let poll_interval = Duration::from_millis(2000);
    let tick_rate = Duration::from_millis(100);

    // Initial daemon connection check
    {
        let tx = bg_tx.clone();
        let sock = app.daemon_socket.clone();
        std::thread::spawn(move || {
            let connected = socket_client::ping(&sock).is_ok();
            let _ = tx.send(BgMessage::DaemonConnected(connected));
            if connected {
                if let Ok(history) = socket_client::get_chat_history(&sock) {
                    let _ = tx.send(BgMessage::ChatHistory(history));
                }
                if let Ok(rs) = socket_client::get_run_state(&sock) {
                    let _ = tx.send(BgMessage::RunStateUpdate {
                        auto_mode: rs.auto_mode,
                    });
                }
            }
        });
    }

    loop {
        terminal.draw(|f| {
            layout = ui::draw(f, &mut app);
        })?;

        // Drain background messages
        while let Ok(msg) = bg_rx.try_recv() {
            match msg {
                BgMessage::DaemonConnected(c) => {
                    app.daemon_connected = c;
                    app.chat_status = if c {
                        "Connected".to_string()
                    } else {
                        "Disconnected".to_string()
                    };
                }
                BgMessage::ChatHistory(messages) => {
                    app.update_chat_messages(messages);
                }
                BgMessage::ChatSendResult { plan, messages } => {
                    app.agent_thinking = false;
                    app.update_chat_messages(messages);
                    // Store pending refactor if agent proposed code changes (non-auto)
                    if plan.action == "refactor" && !plan.code_changes.is_empty() && !app.auto_mode {
                        app.pending_refactor = Some(plan);
                        app.chat_status = "Refactor proposed — press y to apply, n to reject".to_string();
                    } else {
                        app.chat_status = "Connected".to_string();
                    }
                }
                BgMessage::ChatSendError(err) => {
                    app.agent_thinking = false;
                    app.chat_status = format!("Error: {}", err);
                }
                BgMessage::RunStateUpdate { auto_mode } => {
                    app.auto_mode = auto_mode;
                }
                BgMessage::RefactorApplied { success, messages } => {
                    app.agent_thinking = false;
                    app.pending_refactor = None;
                    app.update_chat_messages(messages);
                    app.chat_status = if success {
                        "Refactor applied successfully".to_string()
                    } else {
                        "Refactor failed (rolled back)".to_string()
                    };
                }
                BgMessage::RefactorError(err) => {
                    app.agent_thinking = false;
                    app.chat_status = format!("Refactor error: {}", err);
                }
                BgMessage::LiveMetrics { metrics, logs, current_step } => {
                    // Merge daemon metrics into TUI scalars
                    for (metric, values) in &metrics {
                        if let Some(arr) = values.as_array() {
                            for val in arr {
                                if let Some(v) = val.as_f64() {
                                    let step = current_step as f64;
                                    let entry = app.scalars.entry(metric.clone()).or_default();
                                    // Avoid duplicate step values
                                    if entry.last().map_or(true, |last| (last.0 - step).abs() > 0.5) {
                                        entry.push((step, v));
                                    }
                                }
                            }
                        }
                    }
                    // Update tags list
                    app.tags = app.scalars.keys().cloned().collect();
                    // Merge daemon logs
                    if !logs.is_empty() {
                        let daemon_log_marker = "[daemon]";
                        // Only add new daemon log lines
                        let existing_daemon_count = app.log_lines.iter()
                            .filter(|l| l.starts_with(daemon_log_marker))
                            .count();
                        for line in logs.iter().skip(existing_daemon_count) {
                            app.log_lines.push(format!("{} {}", daemon_log_marker, line));
                        }
                    }
                    if current_step > app.max_step {
                        app.max_step = current_step;
                    }
                }
            }
        }

        // Periodic daemon poll
        if last_poll.elapsed() >= poll_interval {
            last_poll = Instant::now();
            let tx = bg_tx.clone();
            let sock = app.daemon_socket.clone();
            std::thread::spawn(move || {
                let connected = socket_client::ping(&sock).is_ok();
                let _ = tx.send(BgMessage::DaemonConnected(connected));
                if connected {
                    if let Ok(history) = socket_client::get_chat_history(&sock) {
                        let _ = tx.send(BgMessage::ChatHistory(history));
                    }
                    if let Ok(rs) = socket_client::get_run_state(&sock) {
                        let _ = tx.send(BgMessage::RunStateUpdate {
                            auto_mode: rs.auto_mode,
                        });
                        let _ = tx.send(BgMessage::LiveMetrics {
                            metrics: rs.metrics,
                            logs: rs.logs,
                            current_step: rs.current_step,
                        });
                    }
                }
            });
        }

        // Poll for events with timeout so we can process bg messages
        if !event::poll(tick_rate)? {
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

                // Chat input mode intercepts all keys
                if app.chat_input_focused && app.active_tab == app::Tab::Chat {
                    match key.code {
                        KeyCode::Esc => {
                            app.chat_input_focused = false;
                        }
                        KeyCode::Enter => {
                            let content = app.chat_input_take();
                            if !content.is_empty() && app.daemon_connected {
                                app.agent_thinking = true;
                                app.chat_status = "Sending...".to_string();
                                let tx = bg_tx.clone();
                                let sock = app.daemon_socket.clone();
                                std::thread::spawn(move || {
                                    match socket_client::send_chat_message(&content, &sock) {
                                        Ok((plan, messages)) => {
                                            let _ = tx.send(BgMessage::ChatSendResult {
                                                plan,
                                                messages,
                                            });
                                        }
                                        Err(e) => {
                                            let _ = tx.send(BgMessage::ChatSendError(
                                                e.to_string(),
                                            ));
                                        }
                                    }
                                });
                            }
                        }
                        KeyCode::Backspace => {
                            app.chat_input_pop();
                        }
                        KeyCode::Char(c) => {
                            app.chat_input_push(c);
                        }
                        _ => {}
                    }
                    continue;
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
                            app::Tab::Chat => app.scroll_chat_down(),
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        match app.active_tab {
                            app::Tab::Graphs => app.scroll_metrics_up(),
                            app::Tab::Logs => app.scroll_logs_up(),
                            app::Tab::Chat => app.scroll_chat_up(),
                        }
                    }
                    KeyCode::Char('l') | KeyCode::Right => app.next_metric(),
                    KeyCode::Char('h') | KeyCode::Left => app.prev_metric(),
                    KeyCode::Char('i') if app.active_tab == app::Tab::Chat => {
                        app.chat_input_focused = true;
                    }
                    KeyCode::Char('y') if app.active_tab == app::Tab::Chat && app.pending_refactor.is_some() => {
                        if let Some(plan) = app.pending_refactor.clone() {
                            app.agent_thinking = true;
                            app.chat_status = "Applying refactor...".to_string();
                            let tx = bg_tx.clone();
                            let sock = app.daemon_socket.clone();
                            std::thread::spawn(move || {
                                match socket_client::apply_refactor(&plan, &sock) {
                                    Ok((success, messages)) => {
                                        let _ = tx.send(BgMessage::RefactorApplied {
                                            success,
                                            messages,
                                        });
                                    }
                                    Err(e) => {
                                        let _ = tx.send(BgMessage::RefactorError(e.to_string()));
                                    }
                                }
                            });
                        }
                    }
                    KeyCode::Char('n') if app.active_tab == app::Tab::Chat && app.pending_refactor.is_some() => {
                        app.pending_refactor = None;
                        app.chat_status = "Refactor rejected".to_string();
                    }
                    KeyCode::Enter => {
                        if app.active_tab == app::Tab::Chat {
                            app.chat_input_focused = true;
                        } else {
                            app.focus_metric(app.selected_metric);
                        }
                    }
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
                            app::Tab::Chat => app.scroll_chat_down(),
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        match app.active_tab {
                            app::Tab::Graphs => app.scroll_metrics_up(),
                            app::Tab::Logs => app.scroll_logs_up(),
                            app::Tab::Chat => app.scroll_chat_up(),
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
