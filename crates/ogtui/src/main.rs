mod app;
mod socket_client;
mod tfevents;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io;
use std::path::{Path, PathBuf};
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

    /// Command used to launch training (e.g. "torchrun --standalone --nproc_per_node=1 train_gpt.py")
    #[arg(long)]
    training_cmd: Option<String>,

    /// Start the training process automatically when daemon starts
    #[arg(long)]
    start_training: bool,

    /// Delete existing TensorBoard event files under --path before auto-start
    #[arg(long)]
    fresh_run: bool,

    /// Codebase root for agent indexing (default: current dir)
    #[arg(long, default_value = ".")]
    codebase_root: PathBuf,

    /// Enable auto-refactor mode (agent applies code fixes automatically)
    #[arg(long)]
    auto: bool,

    /// Unix socket path for daemon communication
    #[arg(long, env = "OGD_SOCKET")]
    socket: Option<PathBuf>,

    /// Refresh interval for live reload in milliseconds (0 disables live reload)
    #[arg(long, default_value_t = 1000)]
    refresh_ms: u64,
}

struct ViewData {
    scalars: std::collections::BTreeMap<String, Vec<(f64, f64)>>,
    log_lines: Vec<String>,
    total_events: usize,
    max_step: i64,
}

fn load_view_data(path: &Path) -> Result<ViewData> {
    let loaded = tfevents::load_run(path)
        .with_context(|| format!("loading events from {}", path.display()))?;

    let mut sorted_events = loaded.events;
    sorted_events.sort_by_key(|e| e.step);
    let total_events = sorted_events.len();
    let max_step = sorted_events.iter().map(|e| e.step).max().unwrap_or(0);

    let mut log_lines = vec!["-- parsed events log --".to_string(), String::new()];
    for ev in &sorted_events {
        log_lines.push(format!(
            "step {:>6} │ {:<30} │ {:.6}",
            ev.step, ev.tag, ev.value
        ));
    }

    Ok(ViewData {
        scalars: loaded.scalars,
        log_lines,
        total_events,
        max_step,
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let initial = load_view_data(&cli.path)?;
    let mut app = App::new(
        initial.scalars,
        initial.log_lines,
        cli.path.clone(),
        initial.total_events,
        initial.max_step,
    );

    // Override socket path if provided
    if let Some(ref sock) = cli.socket {
        app.daemon_socket = sock.clone();
    }

    // ── Spawn agent daemon if --training-file is provided ───────────────
    let mut daemon_child: Option<Child> = None;
    if let Some(ref training_file) = cli.training_file {
        let start_training = cli.start_training || cli.training_cmd.is_some();
        match spawn_daemon(
            training_file,
            &cli.codebase_root,
            &cli.path,
            cli.auto,
            start_training,
            cli.fresh_run,
            cli.training_cmd.as_deref(),
            &app.daemon_socket,
        ) {
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
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, app, &cli.path, cli.refresh_ms);

    // ── Restore terminal ────────────────────────────────────────────────
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
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
    run_dir: &PathBuf,
    auto_mode: bool,
    start_training: bool,
    fresh_run: bool,
    training_cmd: Option<&str>,
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
        .arg("--run-dir")
        .arg(run_dir)
        .arg("--socket")
        .arg(socket_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(training_cmd) = training_cmd.map(str::trim).filter(|s| !s.is_empty()) {
        cmd.arg("--training-cmd").arg(training_cmd);
    }

    if auto_mode {
        cmd.arg("--auto");
    }
    if start_training {
        cmd.arg("--start-training");
    }
    if fresh_run {
        cmd.arg("--fresh-run");
    }

    let child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn daemon with '{}'. Is og-agent-chat installed?",
            python
        )
    })?;

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

fn tail_overlap(previous: &[String], current: &[String]) -> usize {
    let max_overlap = previous.len().min(current.len());
    for overlap in (0..=max_overlap).rev() {
        if previous[previous.len().saturating_sub(overlap)..] == current[..overlap] {
            return overlap;
        }
    }
    0
}

fn is_known_log_prefix(token: &str) -> bool {
    matches!(
        token,
        "daemon"
            | "system"
            | "info"
            | "sucess"
            | "success"
            | "error"
            | "important"
            | "alert"
            | "auto"
    )
}

fn strip_known_log_prefixes(mut line: &str) -> &str {
    loop {
        let trimmed = line.trim_start();
        let Some(after_bracket) = trimmed.strip_prefix('[') else {
            return trimmed;
        };
        let Some(end) = after_bracket.find(']') else {
            return trimmed;
        };
        let token = after_bracket[..end].trim().to_ascii_lowercase();
        if !is_known_log_prefix(token.as_str()) {
            return trimmed;
        }
        line = after_bracket[end + 1..].trim_start();
    }
}

fn normalize_live_log_line(raw_line: &str) -> Option<String> {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    let stripped = strip_known_log_prefixes(trimmed);
    let message = if stripped.is_empty() {
        trimmed
    } else {
        stripped
    };

    let level = if lower.starts_with("[important]")
        || lower.starts_with("[alert]")
        || lower.starts_with("[auto]")
    {
        "important"
    } else if lower.starts_with("[error]")
        || lower.contains("traceback")
        || lower.contains("exception")
        || lower.contains("panic")
        || lower.contains("fatal")
        || lower.contains("failed")
        || lower.contains("error")
        || (lower.contains("exited with code") && !lower.contains("code 0"))
    {
        "error"
    } else if lower.starts_with("[sucess]")
        || lower.starts_with("[success]")
        || lower.contains("succeeded")
        || lower.contains("completed")
        || lower.contains("restarted")
        || lower.contains("exited with code 0")
    {
        "sucess"
    } else {
        "info"
    };

    Some(format!("[{}] {}", level, message))
}

fn format_alert_log_line(alert: &socket_client::AlertInfo) -> String {
    format!(
        "[important] alert {}: {} (current {:.6}, threshold {:.6})",
        alert.metric, alert.message, alert.current, alert.threshold
    )
}

fn update_auto_mode(app: &mut App, auto_mode: bool) -> bool {
    let changed = app.auto_mode != auto_mode;
    app.auto_mode = auto_mode;
    if changed && app.live_logs_active {
        let state = if auto_mode { "enabled" } else { "disabled" };
        app.append_live_log(format!("[important] auto mode {}", state));
    }
    changed
}

fn set_copy_mode(app: &mut App, enabled: bool) -> Result<()> {
    if app.copy_mode == enabled {
        return Ok(());
    }

    let mut stdout = io::stdout();
    if enabled {
        execute!(stdout, crossterm::event::DisableMouseCapture)?;
        app.copy_mode = true;
        app.chat_status =
            "Copy mode enabled — drag mouse to highlight text, press F6 to resume".to_string();
    } else {
        execute!(stdout, crossterm::event::EnableMouseCapture)?;
        app.copy_mode = false;
        app.chat_status = "Copy mode disabled — interactive mouse controls restored".to_string();
    }
    Ok(())
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
        alerts: Vec<socket_client::AlertInfo>,
        current_step: i64,
        auto_mode: bool,
    },
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    events_path: &Path,
    refresh_ms: u64,
) -> Result<()> {
    // Track layout regions for mouse hit-testing
    let mut layout = ui::LayoutRegions::default();
    let refresh_interval = (refresh_ms > 0).then(|| Duration::from_millis(refresh_ms));
    let mut last_refresh = Instant::now();

    // Channel for background daemon communication
    let (bg_tx, bg_rx) = mpsc::channel::<BgMessage>();

    // Periodic polling state
    let mut last_poll = Instant::now();
    let poll_interval = Duration::from_millis(500);
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
        if let Some(interval) = refresh_interval {
            if last_refresh.elapsed() >= interval {
                if let Ok(updated) = load_view_data(events_path) {
                    let prev_events = app.total_events;
                    let prev_step = app.max_step;
                    let events_grew = updated.total_events > prev_events;
                    let step_changed = updated.max_step != prev_step;
                    let daemon_live_metrics_active = app.daemon_connected && app.live_logs_active;
                    if daemon_live_metrics_active {
                        // Keep daemon-fed metrics visible even when event-file refresh is empty.
                        app.total_events = app.total_events.max(updated.total_events);
                        app.max_step = app.max_step.max(updated.max_step);
                    } else {
                        app.replace_data(
                            updated.scalars,
                            updated.log_lines,
                            updated.total_events,
                            updated.max_step,
                        );
                    }

                    // Event-file refresh is also a live source (even when daemon is connected).
                    if !app.live_logs_active && (events_grew || step_changed) {
                        app.activate_live_logs();
                        app.append_live_log(
                            "[important] live mode: watching event stream updates".to_string(),
                        );
                        app.last_logged_step = prev_step;
                    }

                    if app.live_logs_active {
                        if updated.total_events > prev_events {
                            let delta = updated.total_events - prev_events;
                            let suffix = if delta == 1 { "" } else { "s" };
                            app.append_live_log(format!(
                                "[info] {} new event{} parsed (total {})",
                                delta, suffix, updated.total_events
                            ));
                        }

                        if !app.daemon_connected && updated.max_step < app.last_logged_step {
                            app.append_live_log(format!(
                                "[info] step counter reset to {}",
                                updated.max_step
                            ));
                            app.last_logged_step = updated.max_step;
                        } else if updated.max_step > app.last_logged_step {
                            let delta = updated.max_step - app.last_logged_step;
                            app.append_live_log(format!(
                                "[sucess] step {} completed (+{})",
                                updated.max_step, delta
                            ));
                            app.last_logged_step = updated.max_step;
                        }
                    }
                }
                last_refresh = Instant::now();
            }
        }

        terminal.draw(|f| {
            layout = ui::draw(f, &mut app);
        })?;

        // Drain background messages
        while let Ok(msg) = bg_rx.try_recv() {
            match msg {
                BgMessage::DaemonConnected(c) => {
                    let was_connected = app.daemon_connected;
                    app.daemon_connected = c;
                    app.chat_status = if c {
                        "Connected".to_string()
                    } else {
                        "Disconnected".to_string()
                    };
                    if was_connected != c && app.live_logs_active {
                        let line = if c {
                            "[info] daemon connected"
                        } else {
                            "[error] daemon disconnected"
                        };
                        app.append_live_log(line);
                    }
                    if !c {
                        app.last_daemon_log_tail.clear();
                    }
                }
                BgMessage::ChatHistory(messages) => {
                    app.update_chat_messages(messages);
                }
                BgMessage::ChatSendResult { plan, messages } => {
                    app.agent_thinking = false;
                    app.update_chat_messages(messages);
                    // Store pending refactor if agent proposed code changes (non-auto)
                    if plan.action == "refactor" && !plan.code_changes.is_empty() && !app.auto_mode
                    {
                        app.pending_refactor = Some(plan);
                        app.chat_status =
                            "Refactor proposed — press y to apply, n to reject".to_string();
                    } else {
                        app.chat_status = "Connected".to_string();
                    }
                }
                BgMessage::ChatSendError(err) => {
                    app.agent_thinking = false;
                    app.chat_status = format!("Error: {}", err);
                }
                BgMessage::RunStateUpdate { auto_mode } => {
                    update_auto_mode(&mut app, auto_mode);
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
                BgMessage::LiveMetrics {
                    metrics,
                    logs,
                    alerts,
                    current_step,
                    auto_mode,
                } => {
                    let just_activated = !app.live_logs_active;
                    if just_activated {
                        app.activate_live_logs();
                    }
                    let auto_mode_changed = update_auto_mode(&mut app, auto_mode);
                    if just_activated && !auto_mode_changed {
                        let state = if auto_mode { "enabled" } else { "disabled" };
                        app.append_live_log(format!("[important] auto mode {}", state));
                    }

                    if current_step < app.last_logged_step {
                        // Training restarted; reset live points so the x-axis stays monotonic.
                        app.scalars.clear();
                        app.tags.clear();
                        app.selected_metric = 0;
                        app.focused_metric = None;
                        app.metrics_scroll = 0;
                        app.append_live_log(format!(
                            "[info] step counter reset to {}",
                            current_step
                        ));
                        app.last_logged_step = current_step;
                        app.max_step = current_step;
                    }

                    // Merge daemon metrics into TUI scalars
                    for (metric, values) in &metrics {
                        if let Some(arr) = values.as_array() {
                            let total = arr.len() as i64;
                            for (idx, val) in arr.iter().enumerate() {
                                if let Some(v) = val.as_f64() {
                                    let offset = total.saturating_sub((idx as i64) + 1);
                                    let inferred_step = current_step.saturating_sub(offset);
                                    let step = inferred_step as f64;
                                    let entry = app.scalars.entry(metric.clone()).or_default();
                                    if let Some(last) = entry.last_mut() {
                                        if (last.0 - step).abs() < 0.5 {
                                            // Refresh value for same step instead of duplicate point.
                                            last.1 = v;
                                        } else if step > last.0 {
                                            entry.push((step, v));
                                        }
                                    } else {
                                        entry.push((step, v));
                                    }
                                }
                            }
                        }
                    }
                    // Update tags list
                    app.tags = app.scalars.keys().cloned().collect();

                    // Merge daemon logs using overlap to handle tail window shifts.
                    if !logs.is_empty() {
                        let overlap = tail_overlap(&app.last_daemon_log_tail, &logs);
                        for line in logs.iter().skip(overlap) {
                            if let Some(formatted) = normalize_live_log_line(line) {
                                app.append_live_log(formatted);
                            }
                        }
                        app.last_daemon_log_tail = logs;
                    }

                    // Surface daemon alerts as important lines.
                    if alerts.len() < app.seen_alert_count {
                        app.seen_alert_count = 0;
                    }
                    for alert in alerts.iter().skip(app.seen_alert_count) {
                        app.append_live_log(format_alert_log_line(alert));
                    }
                    app.seen_alert_count = alerts.len();

                    if current_step > app.last_logged_step {
                        let delta = current_step - app.last_logged_step;
                        app.append_live_log(format!(
                            "[sucess] step {} completed (+{})",
                            current_step, delta
                        ));
                        app.last_logged_step = current_step;
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
            let thinking = app.agent_thinking;
            std::thread::spawn(move || {
                let connected = socket_client::ping(&sock).is_ok();
                let _ = tx.send(BgMessage::DaemonConnected(connected));
                if connected {
                    if !thinking {
                        if let Ok(history) = socket_client::get_chat_history(&sock) {
                            let _ = tx.send(BgMessage::ChatHistory(history));
                        }
                    }
                    if let Ok(rs) = socket_client::get_run_state(&sock) {
                        let _ = tx.send(BgMessage::RunStateUpdate {
                            auto_mode: rs.auto_mode,
                        });
                        let _ = tx.send(BgMessage::LiveMetrics {
                            metrics: rs.metrics,
                            logs: rs.logs,
                            alerts: rs.alerts,
                            current_step: rs.current_step,
                            auto_mode: rs.auto_mode,
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

                if key.code == KeyCode::F(6) {
                    let enable_copy_mode = !app.copy_mode;
                    set_copy_mode(&mut app, enable_copy_mode)?;
                    continue;
                }

                if app.copy_mode {
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
                                // Show user message immediately
                                app.chat_messages.push(socket_client::ChatMessage {
                                    sender: "user".to_string(),
                                    content: content.clone(),
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs_f64())
                                        .unwrap_or(0.0),
                                });
                                let len = app.chat_messages.len();
                                app.chat_scroll = len.saturating_sub(1) as u16;
                                app.agent_thinking = true;
                                app.chat_status = "Sending...".to_string();
                                let tx = bg_tx.clone();
                                let sock = app.daemon_socket.clone();
                                std::thread::spawn(move || match socket_client::send_chat_message(
                                    &content, &sock,
                                ) {
                                    Ok((plan, messages)) => {
                                        let _ =
                                            tx.send(BgMessage::ChatSendResult { plan, messages });
                                    }
                                    Err(e) => {
                                        let _ = tx.send(BgMessage::ChatSendError(e.to_string()));
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
                    KeyCode::Char('j') | KeyCode::Down => match app.active_tab {
                        app::Tab::Graphs => app.scroll_metrics_down(),
                        app::Tab::Logs => app.scroll_logs_down(),
                        app::Tab::Chat => app.scroll_chat_down(),
                    },
                    KeyCode::Char('k') | KeyCode::Up => match app.active_tab {
                        app::Tab::Graphs => app.scroll_metrics_up(),
                        app::Tab::Logs => app.scroll_logs_up(),
                        app::Tab::Chat => app.scroll_chat_up(),
                    },
                    KeyCode::Char('l') | KeyCode::Right => app.next_metric(),
                    KeyCode::Char('h') | KeyCode::Left => app.prev_metric(),
                    KeyCode::Char('i') if app.active_tab == app::Tab::Chat => {
                        app.chat_input_focused = true;
                    }
                    KeyCode::Char('y')
                        if app.active_tab == app::Tab::Chat && app.pending_refactor.is_some() =>
                    {
                        if let Some(plan) = app.pending_refactor.clone() {
                            app.agent_thinking = true;
                            app.chat_status = "Applying refactor...".to_string();
                            let tx = bg_tx.clone();
                            let sock = app.daemon_socket.clone();
                            std::thread::spawn(move || {
                                match socket_client::apply_refactor(&plan, &sock) {
                                    Ok((success, messages)) => {
                                        let _ = tx
                                            .send(BgMessage::RefactorApplied { success, messages });
                                    }
                                    Err(e) => {
                                        let _ = tx.send(BgMessage::RefactorError(e.to_string()));
                                    }
                                }
                            });
                        }
                    }
                    KeyCode::Char('n')
                        if app.active_tab == app::Tab::Chat && app.pending_refactor.is_some() =>
                    {
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
                    MouseEventKind::ScrollDown => match app.active_tab {
                        app::Tab::Graphs => app.scroll_metrics_down(),
                        app::Tab::Logs => app.scroll_logs_down(),
                        app::Tab::Chat => app.scroll_chat_down(),
                    },
                    MouseEventKind::ScrollUp => match app.active_tab {
                        app::Tab::Graphs => app.scroll_metrics_up(),
                        app::Tab::Logs => app.scroll_logs_up(),
                        app::Tab::Chat => app.scroll_chat_up(),
                    },
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_live_log_line, tail_overlap};

    #[test]
    fn tail_overlap_handles_sliding_windows() {
        let previous = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let current = vec![
            "c".to_string(),
            "d".to_string(),
            "e".to_string(),
            "f".to_string(),
        ];
        assert_eq!(tail_overlap(&previous, &current), 2);
    }

    #[test]
    fn normalize_strips_known_prefixes_and_keeps_info() {
        let line = "[daemon] [info] training warmup started";
        assert_eq!(
            normalize_live_log_line(line),
            Some("[info] training warmup started".to_string())
        );
    }

    #[test]
    fn normalize_marks_restart_as_success() {
        let line = "[system] training restarted (pid=1234)";
        assert_eq!(
            normalize_live_log_line(line),
            Some("[sucess] training restarted (pid=1234)".to_string())
        );
    }

    #[test]
    fn normalize_marks_traceback_as_error() {
        let line = "Traceback (most recent call last):";
        assert_eq!(
            normalize_live_log_line(line),
            Some("[error] Traceback (most recent call last):".to_string())
        );
    }

    #[test]
    fn normalize_marks_alert_as_important() {
        let line = "[alert] val/loss crossed threshold";
        assert_eq!(
            normalize_live_log_line(line),
            Some("[important] val/loss crossed threshold".to_string())
        );
    }
}
