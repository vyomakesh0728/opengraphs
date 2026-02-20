mod app;
mod socket_client;
mod tfevents;
mod ui;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use app::{App, ProcessSnapshot, ProcessSort};

#[derive(Debug, Clone, Args)]
struct TuiArgs {
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

    /// Sort key used in the procs tab
    #[arg(long = "procs-sort", value_enum, default_value = "cpu")]
    procs_sort: ProcessSort,

    /// Process sampling interval in milliseconds (0 disables process polling)
    #[arg(long = "procs-interval-ms", default_value_t = 1000)]
    procs_interval_ms: u64,

    /// Maximum number of running/exited processes retained in the procs tab
    #[arg(long = "procs-limit", default_value_t = 300)]
    procs_limit: usize,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum AutoModeArg {
    Observe,
    Suggest,
    Supervised,
    Autonomous,
}

impl AutoModeArg {
    fn daemon_auto_enabled(self) -> bool {
        matches!(self, AutoModeArg::Autonomous)
    }
}

#[derive(Debug, Clone, Serialize)]
struct GraphFilter {
    metrics: Vec<String>,
    sys: Vec<String>,
}

#[derive(Debug, Clone, Args)]
struct RunArgs {
    /// Training file to run
    file: PathBuf,

    /// Path to run directory where .tfevents files are written/read
    #[arg(long, default_value = "runs/")]
    path: PathBuf,

    /// Graph selection JSON, e.g. '{"metrics":"loss","sys":"gpu"}'
    #[arg(long)]
    graph: Option<String>,

    /// Optional prompt sent to the agent after startup
    #[arg(long)]
    prompt: Option<String>,

    /// Auto mode policy
    #[arg(long = "auto", value_enum, default_value = "suggest")]
    auto_mode: AutoModeArg,

    /// Command used to launch training process
    #[arg(long)]
    training_cmd: Option<String>,

    /// Codebase root for agent indexing (default: current dir)
    #[arg(long, default_value = ".")]
    codebase_root: PathBuf,

    /// Unix socket path for daemon communication
    #[arg(long, env = "OGD_SOCKET")]
    socket: Option<PathBuf>,

    /// Refresh interval for live reload in milliseconds (0 disables live reload)
    #[arg(long, default_value_t = 1000)]
    refresh_ms: u64,

    /// Sort key used in the procs tab
    #[arg(long = "procs-sort", value_enum, default_value = "cpu")]
    procs_sort: ProcessSort,

    /// Process sampling interval in milliseconds (0 disables process polling)
    #[arg(long = "procs-interval-ms", default_value_t = 1000)]
    procs_interval_ms: u64,

    /// Maximum number of running/exited processes retained in the procs tab
    #[arg(long = "procs-limit", default_value_t = 300)]
    procs_limit: usize,
}

#[derive(Debug, Clone, Args)]
struct TailArgs {
    /// Run id or a direct log/tfevents path
    target: String,

    /// Number of lines/events to print from the tail
    #[arg(long, default_value_t = 120)]
    lines: usize,

    /// Root runs directory for run-id resolution
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
}

#[derive(Debug, Clone, Args)]
struct ResumeArgs {
    /// Run id to resume
    run_id: String,

    /// Checkpoint id or 'latest'
    #[arg(long, default_value = "latest")]
    checkpoint: String,

    /// Checkpoint directory root
    #[arg(long, default_value = ".og_checkpoints")]
    checkpoint_dir: PathBuf,

    /// Root runs directory
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
}

#[derive(Debug, Clone, Args)]
struct ListArgs {
    #[command(subcommand)]
    cmd: ListSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ListSubcommand {
    /// List available projects
    Projects(ListProjectsArgs),
    /// List runs for a project/root
    Runs(ListRunsArgs),
    /// List metrics in a run
    Metrics(ListMetricsArgs),
    /// List system metrics in a run
    #[command(name = "system-metrics")]
    SystemMetrics(ListMetricsArgs),
}

#[derive(Debug, Clone, Args)]
struct ListProjectsArgs {
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
}

#[derive(Debug, Clone, Args)]
struct ListRunsArgs {
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
    #[arg(long)]
    project: Option<String>,
    #[arg(long = "tag")]
    tag: Vec<String>,
    #[arg(long = "config")]
    config: Vec<String>,
    #[arg(long)]
    status: Option<String>,
}

#[derive(Debug, Clone, Args)]
struct ListMetricsArgs {
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    run: String,
}

#[derive(Debug, Clone, Args)]
struct GetArgs {
    #[command(subcommand)]
    cmd: GetSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
enum GetSubcommand {
    /// Get run summary
    Run(GetRunArgs),
    /// Get metric series from a run
    Metric(GetMetricArgs),
}

#[derive(Debug, Clone, Args)]
struct GetRunArgs {
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    run: String,
}

#[derive(Debug, Clone, Args)]
struct GetMetricArgs {
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    run: String,
    #[arg(long)]
    metric: String,
}

#[derive(Debug, Clone, Args)]
struct CompareArgs {
    /// Comma-separated run ids or paths
    #[arg(long, value_delimiter = ',')]
    runs: Vec<String>,
    /// Metric to compare
    #[arg(long)]
    metric: String,
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
    #[arg(long)]
    project: Option<String>,
}

#[derive(Debug, Clone, Args)]
struct SearchArgs {
    #[command(subcommand)]
    cmd: SearchSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
enum SearchSubcommand {
    /// Search metric names
    Metrics(SearchMetricsArgs),
}

#[derive(Debug, Clone, Args)]
struct SearchMetricsArgs {
    #[arg(long)]
    query: String,
    #[arg(long, default_value = "runs/")]
    path: PathBuf,
    #[arg(long)]
    project: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
enum OgCommand {
    /// Launch run in TUI
    Run(RunArgs),
    /// Tail logs/event stream
    Tail(TailArgs),
    /// Resolve resume checkpoint info
    Resume(ResumeArgs),
    /// List entities
    List(ListArgs),
    /// Get details
    Get(GetArgs),
    /// Compare metric across runs
    Compare(CompareArgs),
    /// Search entities
    Search(SearchArgs),
}

/// OpenGraphs command surface.
#[derive(Parser, Debug, Clone)]
#[command(name = "og", version, about = "OpenGraphs CLI + TUI")]
struct Cli {
    /// Output JSON instead of plain text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<OgCommand>,

    #[command(flatten)]
    tui: TuiArgs,
}

#[derive(Debug, Clone, Serialize)]
struct CommandOutput {
    command: String,
    data: Value,
    text: String,
}

struct ViewData {
    scalars: BTreeMap<String, Vec<(f64, f64)>>,
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
    if let Some(command) = cli.command.clone() {
        return execute_cli_command(command, cli.json);
    }

    run_tui(&cli.tui, None, None)
}

fn execute_cli_command(command: OgCommand, json: bool) -> Result<()> {
    match command {
        OgCommand::Run(args) => {
            let graph_filter = args.graph.as_deref().map(parse_graph_filter).transpose()?;
            let tui = run_args_to_tui(&args);
            run_tui(&tui, args.prompt.clone(), graph_filter)
        }
        other => {
            let output = execute_query_command(other)?;
            print_command_output(&output, json)
        }
    }
}

fn run_args_to_tui(args: &RunArgs) -> TuiArgs {
    TuiArgs {
        path: args.path.clone(),
        training_file: Some(args.file.clone()),
        training_cmd: args.training_cmd.clone(),
        start_training: true,
        fresh_run: false,
        codebase_root: args.codebase_root.clone(),
        auto: args.auto_mode.daemon_auto_enabled(),
        socket: args.socket.clone(),
        refresh_ms: args.refresh_ms,
        procs_sort: args.procs_sort,
        procs_interval_ms: args.procs_interval_ms,
        procs_limit: args.procs_limit,
    }
}

fn run_tui(
    tui: &TuiArgs,
    startup_prompt: Option<String>,
    graph_filter: Option<GraphFilter>,
) -> Result<()> {
    let mut initial = load_view_data(&tui.path)?;
    if let Some(filter) = graph_filter.as_ref() {
        initial.scalars = filter_scalars(initial.scalars, filter);
    }
    let mut app = App::new(
        initial.scalars,
        initial.log_lines,
        tui.path.clone(),
        initial.total_events,
        initial.max_step,
    );
    app.set_process_preferences(tui.procs_sort, tui.procs_limit);

    // Override socket path if provided
    if let Some(ref sock) = tui.socket {
        app.daemon_socket = sock.clone();
    }

    // ── Spawn agent daemon if --training-file is provided ───────────────
    let mut daemon_child: Option<Child> = None;
    if let Some(ref training_file) = tui.training_file {
        let start_training = tui.start_training || tui.training_cmd.is_some();
        match spawn_daemon(
            training_file,
            &tui.codebase_root,
            &tui.path,
            tui.auto,
            start_training,
            tui.fresh_run,
            tui.training_cmd.as_deref(),
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

    let result = run_app(
        &mut terminal,
        app,
        &tui.path,
        tui.refresh_ms,
        tui.procs_interval_ms,
        startup_prompt,
        graph_filter,
    );

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

fn print_command_output(output: &CommandOutput, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&output.data)?);
    } else if !output.text.is_empty() {
        println!("{}", output.text);
    }
    Ok(())
}

fn parse_graph_filter(raw: &str) -> Result<GraphFilter> {
    fn parse_string_or_array(value: Option<&Value>, key: &str) -> Result<Vec<String>> {
        let Some(value) = value else {
            return Ok(Vec::new());
        };
        match value {
            Value::String(s) => Ok(vec![s.clone()]),
            Value::Array(items) => {
                let mut out = Vec::new();
                for item in items {
                    let Some(s) = item.as_str() else {
                        bail!("'{}' array must contain only strings", key);
                    };
                    out.push(s.to_string());
                }
                Ok(out)
            }
            _ => bail!("'{}' must be a string or an array of strings", key),
        }
    }

    let parsed: Value =
        serde_json::from_str(raw).with_context(|| "parsing --graph JSON".to_string())?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("--graph must be a JSON object"))?;
    let metrics = parse_string_or_array(obj.get("metrics"), "metrics")?;
    let sys = parse_string_or_array(obj.get("sys"), "sys")?;
    Ok(GraphFilter { metrics, sys })
}

fn filter_scalars(
    scalars: BTreeMap<String, Vec<(f64, f64)>>,
    filter: &GraphFilter,
) -> BTreeMap<String, Vec<(f64, f64)>> {
    if filter.metrics.is_empty() && filter.sys.is_empty() {
        return scalars;
    }

    let metrics: Vec<String> = filter
        .metrics
        .iter()
        .map(|m| m.to_ascii_lowercase())
        .collect();
    let sys: Vec<String> = filter.sys.iter().map(|m| m.to_ascii_lowercase()).collect();
    let mut filtered = BTreeMap::new();

    for (tag, series) in scalars.iter() {
        let tag_l = tag.to_ascii_lowercase();
        let metric_match = metrics.iter().any(|m| tag_l.contains(m));
        let sys_match = sys.iter().any(|m| tag_l.contains(m));
        if metric_match || sys_match {
            filtered.insert(tag.clone(), series.clone());
        }
    }

    if filtered.is_empty() {
        scalars
    } else {
        filtered
    }
}

fn metric_matches_filter(metric: &str, filter: &GraphFilter) -> bool {
    if filter.metrics.is_empty() && filter.sys.is_empty() {
        return true;
    }
    let metric_l = metric.to_ascii_lowercase();
    filter
        .metrics
        .iter()
        .any(|needle| metric_l.contains(&needle.to_ascii_lowercase()))
        || filter
            .sys
            .iter()
            .any(|needle| metric_l.contains(&needle.to_ascii_lowercase()))
}

#[derive(Debug, Clone, Serialize)]
struct RunSummaryData {
    id: String,
    path: String,
    metric_count: usize,
    event_count: usize,
    max_step: i64,
    status: String,
    last_updated_unix: Option<u64>,
}

fn execute_query_command(command: OgCommand) -> Result<CommandOutput> {
    match command {
        OgCommand::Run(_) => bail!("run must be executed in run mode"),
        OgCommand::Tail(args) => execute_tail(args),
        OgCommand::Resume(args) => execute_resume(args),
        OgCommand::List(args) => execute_list(args),
        OgCommand::Get(args) => execute_get(args),
        OgCommand::Compare(args) => execute_compare(args),
        OgCommand::Search(args) => execute_search(args),
    }
}

fn execute_tail(args: TailArgs) -> Result<CommandOutput> {
    let target_path = {
        let explicit = PathBuf::from(&args.target);
        if explicit.exists() {
            explicit
        } else {
            args.path.join(&args.target)
        }
    };

    if !target_path.exists() {
        bail!("target '{}' not found", target_path.display());
    }

    let mut lines: Vec<String> = Vec::new();
    let kind: &str;
    if target_path.is_dir() {
        kind = "run_events";
        let view = load_view_data(&target_path)?;
        let start = view.log_lines.len().saturating_sub(args.lines);
        lines.extend(view.log_lines[start..].iter().cloned());
    } else if target_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|n| n.contains("tfevents"))
        .unwrap_or(false)
    {
        kind = "tfevents";
        let events = tfevents::parse_events_file(&target_path)?;
        let start = events.len().saturating_sub(args.lines);
        for ev in &events[start..] {
            lines.push(format!(
                "step {:>6} │ {:<30} │ {:.6}",
                ev.step, ev.tag, ev.value
            ));
        }
    } else {
        kind = "text";
        let raw = fs::read_to_string(&target_path)
            .with_context(|| format!("reading {}", target_path.display()))?;
        let all: Vec<&str> = raw.lines().collect();
        let start = all.len().saturating_sub(args.lines);
        lines.extend(all[start..].iter().map(|s| s.to_string()));
    }

    let data = serde_json::json!({
        "target": target_path.display().to_string(),
        "kind": kind,
        "line_count": lines.len(),
        "lines": lines,
    });

    let text = lines.join("\n");
    Ok(CommandOutput {
        command: "tail".to_string(),
        data,
        text,
    })
}

fn execute_resume(args: ResumeArgs) -> Result<CommandOutput> {
    let run_path = args.path.join(&args.run_id);
    let checkpoint_path = resolve_checkpoint_path(&args.checkpoint_dir, &args.checkpoint)?;

    let mut snapshot_files = Vec::new();
    for entry in fs::read_dir(&checkpoint_path)? {
        let entry = entry?;
        if entry.path().is_file() && entry.file_name() != "state.json" {
            snapshot_files.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    snapshot_files.sort();

    let data = serde_json::json!({
        "run_id": args.run_id,
        "run_path": run_path.display().to_string(),
        "run_exists": run_path.exists(),
        "checkpoint": args.checkpoint,
        "checkpoint_path": checkpoint_path.display().to_string(),
        "snapshot_files": snapshot_files,
        "note": "Checkpoint resolution is available. Resume apply/restore is handled by the agent refactor loop."
    });

    let text = format!(
        "run: {}\ncheckpoint: {}\nfiles: {}\nnote: resume metadata resolved; apply/restore is agent-managed.",
        run_path.display(),
        checkpoint_path.display(),
        snapshot_files.join(", ")
    );
    Ok(CommandOutput {
        command: "resume".to_string(),
        data,
        text,
    })
}

fn execute_list(args: ListArgs) -> Result<CommandOutput> {
    match args.cmd {
        ListSubcommand::Projects(a) => execute_list_projects(a),
        ListSubcommand::Runs(a) => execute_list_runs(a),
        ListSubcommand::Metrics(a) => execute_list_metrics(a, false),
        ListSubcommand::SystemMetrics(a) => execute_list_metrics(a, true),
    }
}

fn execute_get(args: GetArgs) -> Result<CommandOutput> {
    match args.cmd {
        GetSubcommand::Run(a) => execute_get_run(a),
        GetSubcommand::Metric(a) => execute_get_metric(a),
    }
}

fn execute_search(args: SearchArgs) -> Result<CommandOutput> {
    match args.cmd {
        SearchSubcommand::Metrics(a) => execute_search_metrics(a),
    }
}

fn execute_list_projects(args: ListProjectsArgs) -> Result<CommandOutput> {
    let base = args.path;
    let mut projects = Vec::new();
    for dir in list_immediate_dirs(&base)? {
        let runs = list_run_dirs(&dir)?;
        projects.push(serde_json::json!({
            "name": dir.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
            "path": dir.display().to_string(),
            "run_count": runs.len(),
        }));
    }
    projects.sort_by(|a, b| {
        let an = a.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let bn = b.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        an.cmp(bn)
    });

    let mut text_lines = vec![format!("projects in {}", base.display())];
    for p in &projects {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let runs = p.get("run_count").and_then(|v| v.as_u64()).unwrap_or(0);
        text_lines.push(format!("- {} ({})", name, runs));
    }
    if projects.is_empty() {
        text_lines.push("- none".to_string());
    }

    let data = serde_json::json!({
        "root": base.display().to_string(),
        "projects": projects,
    });
    Ok(CommandOutput {
        command: "list.projects".to_string(),
        data,
        text: text_lines.join("\n"),
    })
}

fn execute_list_runs(args: ListRunsArgs) -> Result<CommandOutput> {
    let base = project_base(&args.path, args.project.as_deref());
    let mut runs = Vec::new();
    for run_dir in list_run_dirs(&base)? {
        let summary = summarize_run(&run_dir)?;
        if let Some(status_filter) = args.status.as_deref() {
            if !summary.status.eq_ignore_ascii_case(status_filter) {
                continue;
            }
        }
        let id_l = summary.id.to_ascii_lowercase();
        if !args
            .tag
            .iter()
            .all(|needle| id_l.contains(&needle.to_ascii_lowercase()))
        {
            continue;
        }
        if !args
            .config
            .iter()
            .all(|needle| id_l.contains(&needle.to_ascii_lowercase()))
        {
            continue;
        }
        runs.push(summary);
    }

    runs.sort_by(|a, b| match b.last_updated_unix.cmp(&a.last_updated_unix) {
        Ordering::Equal => a.id.cmp(&b.id),
        other => other,
    });

    let mut text_lines = vec![format!("runs in {}", base.display())];
    for run in &runs {
        text_lines.push(format!(
            "- {} | status={} | metrics={} | step={}",
            run.id, run.status, run.metric_count, run.max_step
        ));
    }
    if runs.is_empty() {
        text_lines.push("- none".to_string());
    }

    let data = serde_json::json!({
        "base": base.display().to_string(),
        "count": runs.len(),
        "runs": runs,
    });
    Ok(CommandOutput {
        command: "list.runs".to_string(),
        data,
        text: text_lines.join("\n"),
    })
}

fn execute_list_metrics(args: ListMetricsArgs, system_only: bool) -> Result<CommandOutput> {
    let run_path = resolve_run_path(&args.path, args.project.as_deref(), &args.run);
    let view = load_view_data(&run_path)?;
    let mut names: Vec<String> = view.scalars.keys().cloned().collect();
    names.sort();
    if system_only {
        names.retain(|n| looks_system_metric(n));
    }

    let header = if system_only {
        "system metrics"
    } else {
        "metrics"
    };
    let mut text_lines = vec![format!("{} in {}", header, run_path.display())];
    for name in &names {
        text_lines.push(format!("- {}", name));
    }
    if names.is_empty() {
        text_lines.push("- none".to_string());
    }

    let data = serde_json::json!({
        "run": run_path.display().to_string(),
        "system_only": system_only,
        "count": names.len(),
        "metrics": names,
    });
    Ok(CommandOutput {
        command: if system_only {
            "list.system-metrics".to_string()
        } else {
            "list.metrics".to_string()
        },
        data,
        text: text_lines.join("\n"),
    })
}

fn execute_get_run(args: GetRunArgs) -> Result<CommandOutput> {
    let run_path = resolve_run_path(&args.path, args.project.as_deref(), &args.run);
    let view = load_view_data(&run_path)?;
    let mut latest = serde_json::Map::new();
    for (metric, series) in &view.scalars {
        if let Some((step, value)) = series.last() {
            latest.insert(
                metric.clone(),
                serde_json::json!({"step": step, "value": value}),
            );
        }
    }
    let summary = summarize_run(&run_path)?;

    let mut text_lines = vec![
        format!("run {}", summary.id),
        format!("path: {}", summary.path),
        format!("status: {}", summary.status),
        format!("metrics: {}", summary.metric_count),
        format!("events: {}", summary.event_count),
        format!("max_step: {}", summary.max_step),
        "latest metrics:".to_string(),
    ];
    let mut keys: Vec<String> = latest.keys().cloned().collect();
    keys.sort();
    for k in keys {
        if let Some(v) = latest.get(&k) {
            text_lines.push(format!("- {}: {}", k, v));
        }
    }

    let data = serde_json::json!({
        "run": summary,
        "latest_metrics": latest,
    });
    Ok(CommandOutput {
        command: "get.run".to_string(),
        data,
        text: text_lines.join("\n"),
    })
}

fn execute_get_metric(args: GetMetricArgs) -> Result<CommandOutput> {
    let run_path = resolve_run_path(&args.path, args.project.as_deref(), &args.run);
    let view = load_view_data(&run_path)?;
    let Some(series) = view.scalars.get(&args.metric) else {
        bail!(
            "metric '{}' not found in run {}",
            args.metric,
            run_path.display()
        );
    };

    let count = series.len();
    let (min, max, last) = summarize_series(series);
    let tail_n = 20usize.min(count);
    let tail = &series[count.saturating_sub(tail_n)..];

    let mut text_lines = vec![
        format!("run: {}", run_path.display()),
        format!("metric: {}", args.metric),
        format!("count: {}", count),
        format!("min: {:.6}", min),
        format!("max: {:.6}", max),
        format!("last: {:.6}", last),
        "tail:".to_string(),
    ];
    for (step, value) in tail {
        text_lines.push(format!("- step {} => {:.6}", step, value));
    }

    let points: Vec<Value> = series
        .iter()
        .map(|(step, value)| serde_json::json!({"step": step, "value": value}))
        .collect();
    let data = serde_json::json!({
        "run": run_path.display().to_string(),
        "metric": args.metric,
        "count": count,
        "min": min,
        "max": max,
        "last": last,
        "points": points,
    });
    Ok(CommandOutput {
        command: "get.metric".to_string(),
        data,
        text: text_lines.join("\n"),
    })
}

fn execute_compare(args: CompareArgs) -> Result<CommandOutput> {
    if args.runs.is_empty() {
        bail!("--runs must include at least one run id/path");
    }

    let mut comparisons = Vec::new();
    let mut text_lines = vec![format!("compare metric '{}'", args.metric)];
    for run in &args.runs {
        let run_path = resolve_run_path(&args.path, args.project.as_deref(), run);
        let view = load_view_data(&run_path)?;
        let Some(series) = view.scalars.get(&args.metric) else {
            text_lines.push(format!(
                "- {}: metric '{}' not found",
                run_path.display(),
                args.metric
            ));
            comparisons.push(serde_json::json!({
                "run": run_path.display().to_string(),
                "found": false,
            }));
            continue;
        };
        let (min, max, last) = summarize_series(series);
        let first = series.first().map(|(_, v)| *v).unwrap_or(last);
        let delta = last - first;
        text_lines.push(format!(
            "- {} | first={:.6} last={:.6} delta={:.6} min={:.6} max={:.6}",
            run_path.display(),
            first,
            last,
            delta,
            min,
            max
        ));
        comparisons.push(serde_json::json!({
            "run": run_path.display().to_string(),
            "found": true,
            "count": series.len(),
            "first": first,
            "last": last,
            "delta": delta,
            "min": min,
            "max": max,
        }));
    }

    let data = serde_json::json!({
        "metric": args.metric,
        "comparisons": comparisons,
    });
    Ok(CommandOutput {
        command: "compare".to_string(),
        data,
        text: text_lines.join("\n"),
    })
}

fn execute_search_metrics(args: SearchMetricsArgs) -> Result<CommandOutput> {
    let query = args.query.to_ascii_lowercase();
    let base = project_base(&args.path, args.project.as_deref());
    let mut matches = Vec::new();
    for run_dir in list_run_dirs(&base)? {
        let view = load_view_data(&run_dir)?;
        let run_id = run_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        for name in view.scalars.keys() {
            if name.to_ascii_lowercase().contains(&query) {
                matches.push(serde_json::json!({
                    "metric": name,
                    "run": run_id,
                    "path": run_dir.display().to_string(),
                }));
            }
        }
    }
    matches.sort_by(|a, b| {
        let am = a.get("metric").and_then(|v| v.as_str()).unwrap_or_default();
        let bm = b.get("metric").and_then(|v| v.as_str()).unwrap_or_default();
        am.cmp(bm)
    });

    let mut text_lines = vec![format!("search metrics query='{}'", args.query)];
    for m in &matches {
        let metric = m.get("metric").and_then(|v| v.as_str()).unwrap_or_default();
        let run = m.get("run").and_then(|v| v.as_str()).unwrap_or_default();
        text_lines.push(format!("- {} ({})", metric, run));
    }
    if matches.is_empty() {
        text_lines.push("- none".to_string());
    }

    let data = serde_json::json!({
        "query": args.query,
        "count": matches.len(),
        "matches": matches,
    });
    Ok(CommandOutput {
        command: "search.metrics".to_string(),
        data,
        text: text_lines.join("\n"),
    })
}

fn project_base(path: &Path, project: Option<&str>) -> PathBuf {
    if let Some(project) = project {
        path.join(project)
    } else {
        path.to_path_buf()
    }
}

fn resolve_run_path(path: &Path, project: Option<&str>, run: &str) -> PathBuf {
    let direct = PathBuf::from(run);
    if direct.exists() {
        return direct;
    }
    project_base(path, project).join(run)
}

fn list_immediate_dirs(path: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    if !path.exists() {
        return Ok(dirs);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            dirs.push(entry_path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn list_run_dirs(path: &Path) -> Result<Vec<PathBuf>> {
    let mut runs = Vec::new();
    for dir in list_immediate_dirs(path)? {
        if contains_tfevents(&dir)? {
            runs.push(dir);
        }
    }

    if runs.is_empty() && path.exists() && contains_tfevents(path)? {
        runs.push(path.to_path_buf());
    }
    runs.sort();
    Ok(runs)
}

fn contains_tfevents(path: &Path) -> Result<bool> {
    if path.is_file() {
        return Ok(path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|n| n.contains("tfevents"))
            .unwrap_or(false));
    }

    if !path.exists() || !path.is_dir() {
        return Ok(false);
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            if contains_tfevents(&entry_path)? {
                return Ok(true);
            }
        } else if entry_path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|n| n.contains("tfevents"))
            .unwrap_or(false)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn summarize_run(path: &Path) -> Result<RunSummaryData> {
    let view = load_view_data(path)?;
    let id = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string());
    let last_updated_unix = latest_mtime_unix(path)?;
    let status = match last_updated_unix {
        Some(last) if unix_now_secs().saturating_sub(last) <= 120 => "running",
        Some(_) => "inactive",
        None => "unknown",
    }
    .to_string();

    Ok(RunSummaryData {
        id,
        path: path.display().to_string(),
        metric_count: view.scalars.len(),
        event_count: view.total_events,
        max_step: view.max_step,
        status,
        last_updated_unix,
    })
}

fn latest_mtime_unix(path: &Path) -> Result<Option<u64>> {
    fn inner(path: &Path, best: &mut Option<u64>) -> Result<()> {
        if path.is_file() {
            let mtime = fs::metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            if let Some(mtime) = mtime {
                *best = Some(best.map_or(mtime, |current| current.max(mtime)));
            }
            return Ok(());
        }
        if !path.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            inner(&entry.path(), best)?;
        }
        Ok(())
    }

    let mut best = None;
    inner(path, &mut best)?;
    Ok(best)
}

fn looks_system_metric(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("sys/")
        || n.contains("gpu")
        || n.contains("vram")
        || n.contains("cpu")
        || n.contains("memory")
        || n.contains("/mem")
        || n.contains("disk")
        || n.contains("net")
}

fn summarize_series(series: &[(f64, f64)]) -> (f64, f64, f64) {
    if series.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for (_, value) in series {
        min = min.min(*value);
        max = max.max(*value);
    }
    let last = series.last().map(|(_, v)| *v).unwrap_or(0.0);
    (min, max, last)
}

fn resolve_checkpoint_path(checkpoint_dir: &Path, checkpoint: &str) -> Result<PathBuf> {
    if checkpoint == "latest" {
        let mut checkpoints = Vec::new();
        if checkpoint_dir.exists() {
            for entry in fs::read_dir(checkpoint_dir)? {
                let entry = entry?;
                if entry.path().is_dir() {
                    checkpoints.push(entry.path());
                }
            }
        }
        checkpoints.sort();
        let Some(latest) = checkpoints.last() else {
            bail!("no checkpoints found under {}", checkpoint_dir.display());
        };
        return Ok(latest.clone());
    }

    let direct = checkpoint_dir.join(checkpoint);
    if direct.is_dir() {
        return Ok(direct);
    }
    let prefixed = checkpoint_dir.join(format!("ckpt_{}", checkpoint));
    if prefixed.is_dir() {
        return Ok(prefixed);
    }
    bail!(
        "checkpoint '{}' not found under {}",
        checkpoint,
        checkpoint_dir.display()
    )
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

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_process_line(line: &str) -> Option<ProcessSnapshot> {
    let mut fields = line.split_whitespace();
    let pid_s = fields.next()?;
    let ppid_s = fields.next()?;
    let state_s = fields.next()?;
    let elapsed_s = fields.next()?;
    let cpu_s = fields.next()?;
    let mem_s = fields.next()?;
    let command = fields.collect::<Vec<_>>().join(" ");
    if command.is_empty() {
        return None;
    }

    let pid = pid_s.parse::<i32>().ok()?;
    let ppid = ppid_s.parse::<i32>().ok()?;
    let elapsed_secs = parse_elapsed_secs(elapsed_s).unwrap_or(0);
    let cpu_pct = cpu_s.parse::<f32>().unwrap_or(0.0);
    let mem_pct = mem_s.parse::<f32>().unwrap_or(0.0);

    Some(ProcessSnapshot {
        pid,
        ppid,
        state: state_s.to_string(),
        elapsed: elapsed_s.to_string(),
        elapsed_secs,
        cpu_pct,
        mem_pct,
        command,
    })
}

fn parse_elapsed_secs(etime: &str) -> Option<u64> {
    let (days, clock) = match etime.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, etime),
    };

    let parts: Vec<u64> = clock
        .split(':')
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;

    let clock_secs = match parts.as_slice() {
        [mm, ss] => mm.saturating_mul(60).saturating_add(*ss),
        [hh, mm, ss] => hh
            .saturating_mul(3600)
            .saturating_add(mm.saturating_mul(60))
            .saturating_add(*ss),
        [ss] => *ss,
        _ => return None,
    };

    Some(days.saturating_mul(86_400).saturating_add(clock_secs))
}

fn sample_processes() -> Result<Vec<ProcessSnapshot>> {
    let output = Command::new("ps")
        .args(["-x", "-o", "pid=,ppid=,state=,etime=,%cpu=,%mem=,command="])
        .output()
        .with_context(|| "sampling process list with ps".to_string())?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("ps failed with status {}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut processes: Vec<ProcessSnapshot> = Vec::new();

    for raw in stdout.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(process) = parse_process_line(line) {
            processes.push(process);
        }
    }

    Ok(processes)
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

fn unix_now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn push_chat_message(app: &mut App, sender: &str, content: String) {
    app.chat_messages.push(socket_client::ChatMessage {
        sender: sender.to_string(),
        content,
        timestamp: unix_now_secs_f64(),
    });
    app.chat_follow_tail = true;
    let len = app.chat_messages.len();
    app.chat_scroll = len.saturating_sub(1) as u16;
}

fn parse_bang_og_cli(content: &str) -> Result<Cli> {
    let trimmed = content.trim();
    if !trimmed.starts_with("!og") {
        bail!("not an !og command");
    }

    let without_bang = trimmed.trim_start_matches('!');
    let words = shlex::split(without_bang)
        .ok_or_else(|| anyhow::anyhow!("failed to parse command line"))?;
    if words.is_empty() {
        bail!("empty command");
    }

    let head = words[0].to_ascii_lowercase();
    if head != "og" && head != "ogtui" {
        bail!("expected !og prefix");
    }

    let mut argv = vec!["og".to_string()];
    argv.extend(words.iter().skip(1).cloned());
    Cli::try_parse_from(argv).map_err(|e| anyhow::anyhow!(e.to_string()))
}

fn execute_in_app_run_command(
    args: RunArgs,
    app: &mut App,
    bg_tx: &mpsc::Sender<BgMessage>,
) -> Result<CommandOutput> {
    if !app.daemon_connected {
        bail!("daemon is not connected; start og with --training-file first");
    }

    let graph_filter = args.graph.as_deref().map(parse_graph_filter).transpose()?;

    socket_client::set_training_file(&args.file, &app.daemon_socket)?;
    let auto_enabled = args.auto_mode.daemon_auto_enabled();
    let resolved_auto = socket_client::set_auto_mode(auto_enabled, &app.daemon_socket)?;
    update_auto_mode(app, resolved_auto);
    socket_client::start_training(&app.daemon_socket)?;

    if let Some(prompt) = args.prompt.clone() {
        app.agent_thinking = true;
        app.chat_status = "Sending prompt...".to_string();
        let tx = bg_tx.clone();
        let sock = app.daemon_socket.clone();
        std::thread::spawn(
            move || match socket_client::send_chat_message(&prompt, &sock) {
                Ok((plan, messages)) => {
                    let _ = tx.send(BgMessage::ChatSendResult { plan, messages });
                }
                Err(e) => {
                    let _ = tx.send(BgMessage::ChatSendError(e.to_string()));
                }
            },
        );
    }

    let mode = match args.auto_mode {
        AutoModeArg::Observe => "observe",
        AutoModeArg::Suggest => "suggest",
        AutoModeArg::Supervised => "supervised",
        AutoModeArg::Autonomous => "autonomous",
    };

    let data = serde_json::json!({
        "training_file": args.file.display().to_string(),
        "mode": mode,
        "auto_enabled": resolved_auto,
        "prompt_sent": args.prompt.is_some(),
        "graph": graph_filter,
    });

    let mut notes = vec![format!("started training: {}", args.file.display())];
    notes.push(format!("mode: {}", mode));
    if let Some(graph) = &args.graph {
        notes.push(format!("graph filter: {}", graph));
    }
    if args.prompt.is_some() {
        notes.push("startup prompt sent".to_string());
    }

    Ok(CommandOutput {
        command: "run".to_string(),
        data,
        text: notes.join("\n"),
    })
}

fn handle_in_app_og_command(
    content: &str,
    app: &mut App,
    bg_tx: &mpsc::Sender<BgMessage>,
) -> Result<()> {
    let cli = parse_bang_og_cli(content)?;
    let Some(command) = cli.command else {
        bail!("usage: !og <run|tail|resume|list|get|compare|search> ...");
    };

    let output = match command {
        OgCommand::Run(args) => execute_in_app_run_command(args, app, bg_tx)?,
        other => execute_query_command(other)?,
    };

    let rendered = if cli.json {
        serde_json::to_string_pretty(&output.data)?
    } else {
        output.text
    };
    push_chat_message(app, "system", rendered);
    app.chat_status = "Command executed".to_string();
    Ok(())
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    events_path: &Path,
    refresh_ms: u64,
    procs_interval_ms: u64,
    startup_prompt: Option<String>,
    graph_filter: Option<GraphFilter>,
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
    let process_poll_interval =
        (procs_interval_ms > 0).then(|| Duration::from_millis(procs_interval_ms));
    let mut last_process_poll = Instant::now();
    let tick_rate = Duration::from_millis(100);
    let mut startup_prompt = startup_prompt;

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

    if process_poll_interval.is_some() {
        if let Ok(processes) = sample_processes() {
            app.update_processes(processes, unix_now_secs());
        }
    }

    loop {
        if let Some(interval) = process_poll_interval {
            if last_process_poll.elapsed() >= interval {
                if let Ok(processes) = sample_processes() {
                    app.update_processes(processes, unix_now_secs());
                }
                last_process_poll = Instant::now();
            }
        }

        if let Some(interval) = refresh_interval {
            if last_refresh.elapsed() >= interval {
                if let Ok(mut updated) = load_view_data(events_path) {
                    if let Some(filter) = graph_filter.as_ref() {
                        updated.scalars = filter_scalars(updated.scalars, filter);
                    }
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
                    } else if let Some(prompt) = startup_prompt.take() {
                        app.agent_thinking = true;
                        app.chat_status = "Sending startup prompt...".to_string();
                        let tx = bg_tx.clone();
                        let sock = app.daemon_socket.clone();
                        std::thread::spawn(move || {
                            match socket_client::send_chat_message(&prompt, &sock) {
                                Ok((plan, messages)) => {
                                    let _ = tx.send(BgMessage::ChatSendResult { plan, messages });
                                }
                                Err(e) => {
                                    let _ = tx.send(BgMessage::ChatSendError(e.to_string()));
                                }
                            }
                        });
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
                        if let Some(filter) = graph_filter.as_ref() {
                            if !metric_matches_filter(metric, filter) {
                                continue;
                            }
                        }
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
                            if content.trim().is_empty() {
                                continue;
                            }

                            push_chat_message(&mut app, "user", content.clone());

                            if content.trim_start().starts_with("!og") {
                                if let Err(err) =
                                    handle_in_app_og_command(&content, &mut app, &bg_tx)
                                {
                                    app.chat_status = format!("Command error: {}", err);
                                    push_chat_message(
                                        &mut app,
                                        "system",
                                        format!("command failed: {}", err),
                                    );
                                }
                                continue;
                            }

                            if app.daemon_connected {
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
                            } else {
                                app.chat_status =
                                    "Daemon not connected (message not sent)".to_string();
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
                        app::Tab::Processes => app.scroll_processes_down(),
                        app::Tab::Chat => app.scroll_chat_down(),
                    },
                    KeyCode::Char('k') | KeyCode::Up => match app.active_tab {
                        app::Tab::Graphs => app.scroll_metrics_up(),
                        app::Tab::Logs => app.scroll_logs_up(),
                        app::Tab::Processes => app.scroll_processes_up(),
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
                        app::Tab::Processes => app.scroll_processes_down(),
                        app::Tab::Chat => app.scroll_chat_down(),
                    },
                    MouseEventKind::ScrollUp => match app.active_tab {
                        app::Tab::Graphs => app.scroll_metrics_up(),
                        app::Tab::Logs => app.scroll_logs_up(),
                        app::Tab::Processes => app.scroll_processes_up(),
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
    use super::{
        AutoModeArg, ListArgs, ListSubcommand, OgCommand, handle_in_app_og_command,
        normalize_live_log_line, parse_bang_og_cli, parse_elapsed_secs, parse_graph_filter,
        parse_process_line, tail_overlap,
    };
    use crate::app::App;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::mpsc;

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

    #[test]
    fn parse_process_line_parses_valid_ps_row() {
        let line = "1234 1 R 00:12 34.5 12.3 python train.py --epochs 10";
        let parsed = parse_process_line(line).expect("expected valid process row");
        assert_eq!(parsed.pid, 1234);
        assert_eq!(parsed.ppid, 1);
        assert_eq!(parsed.state, "R");
        assert_eq!(parsed.elapsed, "00:12");
        assert_eq!(parsed.elapsed_secs, 12);
        assert!((parsed.cpu_pct - 34.5).abs() < 0.001);
        assert!((parsed.mem_pct - 12.3).abs() < 0.001);
        assert_eq!(parsed.command, "python train.py --epochs 10");
    }

    #[test]
    fn parse_process_line_rejects_incomplete_row() {
        let line = "1234 1 R 00:12";
        assert!(parse_process_line(line).is_none());
    }

    #[test]
    fn parse_elapsed_secs_handles_day_hour_format() {
        assert_eq!(parse_elapsed_secs("1-02:03:04"), Some(93_784));
        assert_eq!(parse_elapsed_secs("04:05"), Some(245));
    }

    #[test]
    fn parse_bang_og_list_runs_command() {
        let cli = parse_bang_og_cli("!og list runs --path runs/").expect("parse command");
        match cli.command {
            Some(OgCommand::List(ListArgs {
                cmd: ListSubcommand::Runs(_),
            })) => {}
            _ => panic!("expected list runs command"),
        }
    }

    #[test]
    fn parse_bang_og_run_auto_mode() {
        let cli =
            parse_bang_og_cli("!og run demo_train.py --auto autonomous").expect("parse command");
        match cli.command {
            Some(OgCommand::Run(args)) => {
                assert_eq!(args.file, PathBuf::from("demo_train.py"));
                assert!(matches!(args.auto_mode, AutoModeArg::Autonomous));
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parse_graph_filter_accepts_mixed_shapes() {
        let raw = r#"{"metrics":"loss","sys":["gpu","vram"]}"#;
        let parsed = parse_graph_filter(raw).expect("parse graph filter");
        assert_eq!(parsed.metrics, vec!["loss"]);
        assert_eq!(parsed.sys, vec!["gpu", "vram"]);
    }

    #[test]
    fn in_app_og_list_command_appends_system_message() {
        let mut app = App::new(BTreeMap::new(), Vec::new(), PathBuf::from("runs"), 0, 0);
        let (tx, _rx) = mpsc::channel();
        handle_in_app_og_command("!og list projects --path runs/", &mut app, &tx)
            .expect("execute in-app command");
        let last = app.chat_messages.last().expect("chat message");
        assert_eq!(last.sender, "system");
        assert!(last.content.contains("projects"));
    }
}
