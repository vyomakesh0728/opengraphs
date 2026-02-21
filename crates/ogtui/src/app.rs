use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use clap::ValueEnum;

use crate::socket_client::{ActionPlanResponse, ChatMessage};

/// Which tab is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Graphs,
    Logs,
    Processes,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProcessSort {
    Cpu,
    Mem,
    Pid,
    Etime,
}

impl ProcessSort {
    pub fn label(self) -> &'static str {
        match self {
            ProcessSort::Cpu => "CPU usage",
            ProcessSort::Mem => "memory usage",
            ProcessSort::Pid => "PID",
            ProcessSort::Etime => "elapsed time",
        }
    }
}

impl Tab {
    pub const ALL: &[Tab] = &[Tab::Chat, Tab::Graphs, Tab::Processes, Tab::Logs];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Graphs => "graphs",
            Tab::Logs => "logs",
            Tab::Processes => "procs",
            Tab::Chat => "chat",
        }
    }

    pub fn next(self) -> Tab {
        match self {
            Tab::Chat => Tab::Graphs,
            Tab::Graphs => Tab::Processes,
            Tab::Processes => Tab::Logs,
            Tab::Logs => Tab::Chat,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub pid: i32,
    pub ppid: i32,
    pub state: String,
    pub elapsed: String,
    pub elapsed_secs: u64,
    pub cpu_pct: f32,
    pub mem_pct: f32,
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct ExitedProcess {
    pub snapshot: ProcessSnapshot,
    pub exited_at_unix: u64,
}

/// Application state.
pub struct App {
    pub active_tab: Tab,
    /// tag → sorted (step, value) pairs
    pub scalars: BTreeMap<String, Vec<(f64, f64)>>,
    /// Optional display labels keyed by metric tag
    pub metric_labels: BTreeMap<String, String>,
    /// Ordered list of tag names for grid iteration
    pub tags: Vec<String>,
    /// Log lines derived from events
    pub log_lines: Vec<String>,
    /// Whether the help overlay is shown
    pub show_help: bool,
    /// Path that was loaded
    pub events_path: PathBuf,
    /// Scroll offset in the logs tab
    pub logs_scroll: u16,
    /// Whether logs should auto-follow incoming lines
    pub logs_follow_tail: bool,
    /// Number of visible rows in the logs viewport (set by UI)
    pub logs_viewport_rows: usize,
    /// Whether the app should quit
    pub should_quit: bool,
    /// Selected metric index for highlight
    pub selected_metric: usize,
    /// Focused metric index for fullscreen detail view (None = grid view)
    pub focused_metric: Option<usize>,
    /// Metrics grid scroll offset (in rows)
    pub metrics_scroll: usize,
    /// Number of visible rows in the metrics grid (set by UI)
    pub metrics_visible_rows: usize,
    /// Number of columns in the metrics grid (set by UI)
    pub metrics_cols: usize,
    /// Total number of events parsed
    pub total_events: usize,
    /// Total steps (max step value)
    pub max_step: i64,

    // ── Agent chat state ─────────────────────────────────────────────────
    /// Chat messages from the daemon
    pub chat_messages: Vec<ChatMessage>,
    /// Current input buffer for the chat
    pub chat_input: String,
    /// Scroll offset in the chat message list
    pub chat_scroll: u16,
    /// Whether chat should auto-follow incoming messages
    pub chat_follow_tail: bool,
    /// Whether the chat input is focused (typing mode)
    pub chat_input_focused: bool,
    /// Whether the agent is currently processing
    pub agent_thinking: bool,
    /// Whether the daemon is connected
    pub daemon_connected: bool,
    /// Socket path for the daemon
    pub daemon_socket: PathBuf,
    /// Status message shown in chat footer
    pub chat_status: String,
    /// Auto-mode flag from daemon
    pub auto_mode: bool,
    /// Pending refactor plan awaiting user approval (non-auto mode)
    pub pending_refactor: Option<ActionPlanResponse>,
    /// Whether the logs tab is currently showing live daemon logs
    pub live_logs_active: bool,
    /// Last daemon log tail seen from get_run_state, used for dedupe
    pub last_daemon_log_tail: Vec<String>,
    /// Number of daemon alerts already surfaced in the log tab
    pub seen_alert_count: usize,
    /// Last step for which a success log line was added
    pub last_logged_step: i64,
    /// Copy mode disables mouse capture so terminal text selection works
    pub copy_mode: bool,
    /// Current running processes from host snapshot
    pub running_processes: Vec<ProcessSnapshot>,
    /// Recently exited processes that were seen in prior snapshots
    pub exited_processes: Vec<ExitedProcess>,
    /// Scroll offset in the processes tab
    pub processes_scroll: u16,
    /// Whether processes view should follow tail
    pub processes_follow_tail: bool,
    /// Number of visible rows in processes viewport
    pub processes_viewport_rows: usize,
    /// Total rendered rows in the processes tab
    pub processes_total_rows: usize,
    /// Sort mode in process view
    pub process_sort: ProcessSort,
    /// Maximum processes to show/store in lists
    pub process_limit: usize,
}

impl App {
    pub fn new(
        scalars: BTreeMap<String, Vec<(f64, f64)>>,
        metric_labels: BTreeMap<String, String>,
        log_lines: Vec<String>,
        events_path: PathBuf,
        total_events: usize,
        max_step: i64,
    ) -> Self {
        let tags: Vec<String> = scalars.keys().cloned().collect();
        let daemon_socket = crate::socket_client::socket_path();
        Self {
            active_tab: Tab::Chat,
            scalars,
            metric_labels,
            tags,
            log_lines,
            show_help: false,
            events_path,
            logs_scroll: 0,
            logs_follow_tail: true,
            logs_viewport_rows: 1,
            should_quit: false,
            selected_metric: 0,
            focused_metric: None,
            metrics_scroll: 0,
            metrics_visible_rows: 3,
            metrics_cols: 4,
            total_events,
            max_step,
            chat_messages: Vec::new(),
            chat_input: String::new(),
            chat_scroll: 0,
            chat_follow_tail: true,
            chat_input_focused: false,
            agent_thinking: false,
            daemon_connected: false,
            daemon_socket,
            chat_status: "Disconnected".to_string(),
            auto_mode: false,
            pending_refactor: None,
            live_logs_active: false,
            last_daemon_log_tail: Vec::new(),
            seen_alert_count: 0,
            last_logged_step: max_step,
            copy_mode: false,
            running_processes: Vec::new(),
            exited_processes: Vec::new(),
            processes_scroll: 0,
            processes_follow_tail: false,
            processes_viewport_rows: 1,
            processes_total_rows: 1,
            process_sort: ProcessSort::Cpu,
            process_limit: 300,
        }
    }

    pub fn metric_display_name<'a>(&'a self, tag: &'a str) -> &'a str {
        self.metric_labels
            .get(tag)
            .map(String::as_str)
            .unwrap_or(tag)
    }

    pub fn set_process_preferences(&mut self, sort: ProcessSort, limit: usize) {
        self.process_sort = sort;
        self.process_limit = limit.max(1);
        self.running_processes.truncate(self.process_limit);
        self.exited_processes.truncate(self.process_limit);
    }

    pub fn cycle_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    pub fn replace_data(
        &mut self,
        scalars: BTreeMap<String, Vec<(f64, f64)>>,
        log_lines: Vec<String>,
        total_events: usize,
        max_step: i64,
    ) {
        let prev_tag = self.tags.get(self.selected_metric).cloned();

        self.scalars = scalars;
        self.tags = self.scalars.keys().cloned().collect();
        self.total_events = total_events;
        self.max_step = max_step;
        if !self.live_logs_active {
            self.log_lines = log_lines;
            self.last_logged_step = max_step;
        }

        if self.tags.is_empty() {
            self.selected_metric = 0;
            self.focused_metric = None;
            self.metrics_scroll = 0;
        } else if let Some(tag) = prev_tag {
            if let Some(index) = self.tags.iter().position(|t| t == &tag) {
                self.selected_metric = index;
            } else if self.selected_metric >= self.tags.len() {
                self.selected_metric = self.tags.len() - 1;
            }
        } else if self.selected_metric >= self.tags.len() {
            self.selected_metric = self.tags.len() - 1;
        }

        if let Some(focused) = self.focused_metric {
            if focused >= self.tags.len() {
                self.focused_metric = None;
            }
        }

        self.ensure_metric_visible();
        if self.logs_follow_tail {
            self.logs_scroll = self.logs_max_scroll();
        } else {
            self.clamp_logs_scroll();
        }
    }

    pub fn activate_live_logs(&mut self) {
        if self.live_logs_active {
            return;
        }
        self.live_logs_active = true;
        self.log_lines = vec![
            "-- live run log --".to_string(),
            "[info] listening to live daemon updates".to_string(),
        ];
        self.logs_follow_tail = true;
        self.logs_scroll = self.logs_max_scroll();
        self.last_daemon_log_tail.clear();
        self.seen_alert_count = 0;
    }

    pub fn append_live_log(&mut self, line: impl Into<String>) {
        self.log_lines.push(line.into());
        if self.logs_follow_tail {
            self.logs_scroll = self.logs_max_scroll();
        } else {
            self.clamp_logs_scroll();
        }
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn scroll_logs_down(&mut self) {
        let max = self.logs_max_scroll();
        self.logs_scroll = (self.logs_scroll + 1).min(max);
        if self.logs_scroll >= max {
            self.logs_follow_tail = true;
        }
    }

    pub fn scroll_logs_up(&mut self) {
        self.logs_follow_tail = false;
        self.logs_scroll = self.logs_scroll.saturating_sub(1);
    }

    pub fn set_logs_viewport_rows(&mut self, rows: usize) {
        self.logs_viewport_rows = rows.max(1);
        if self.logs_follow_tail {
            self.logs_scroll = self.logs_max_scroll();
        } else {
            self.clamp_logs_scroll();
        }
    }

    fn logs_max_scroll(&self) -> u16 {
        self.log_lines
            .len()
            .saturating_sub(self.logs_viewport_rows.max(1)) as u16
    }

    fn clamp_logs_scroll(&mut self) {
        let max_scroll = self.logs_max_scroll();
        if self.logs_scroll > max_scroll {
            self.logs_scroll = max_scroll;
        }
    }

    pub fn next_metric(&mut self) {
        if !self.tags.is_empty() {
            self.selected_metric = (self.selected_metric + 1) % self.tags.len();
            self.ensure_metric_visible();
        }
    }

    pub fn prev_metric(&mut self) {
        if !self.tags.is_empty() {
            self.selected_metric = if self.selected_metric == 0 {
                self.tags.len() - 1
            } else {
                self.selected_metric - 1
            };
            self.ensure_metric_visible();
        }
    }

    /// Scroll the metrics grid so the selected metric is visible.
    fn ensure_metric_visible(&mut self) {
        if self.tags.is_empty() || self.metrics_cols == 0 {
            return;
        }
        let selected_row = self.selected_metric / self.metrics_cols;
        if selected_row < self.metrics_scroll {
            self.metrics_scroll = selected_row;
        } else if selected_row >= self.metrics_scroll + self.metrics_visible_rows {
            self.metrics_scroll = selected_row + 1 - self.metrics_visible_rows;
        }
    }

    pub fn scroll_metrics_down(&mut self) {
        if self.tags.is_empty() || self.metrics_cols == 0 {
            return;
        }
        let new = self.selected_metric + self.metrics_cols;
        if new < self.tags.len() {
            self.selected_metric = new;
        } else {
            // Jump to last item
            self.selected_metric = self.tags.len() - 1;
        }
        self.ensure_metric_visible();
    }

    pub fn scroll_metrics_up(&mut self) {
        if self.tags.is_empty() || self.metrics_cols == 0 {
            return;
        }
        if self.selected_metric >= self.metrics_cols {
            self.selected_metric -= self.metrics_cols;
        } else {
            self.selected_metric = 0;
        }
        self.ensure_metric_visible();
    }

    pub fn focus_metric(&mut self, index: usize) {
        if index < self.tags.len() {
            self.focused_metric = Some(index);
        }
    }

    pub fn unfocus_metric(&mut self) {
        self.focused_metric = None;
    }

    // ── Chat methods ────────────────────────────────────────────────────

    pub fn chat_input_push(&mut self, c: char) {
        self.chat_input.push(c);
    }

    pub fn chat_input_pop(&mut self) {
        self.chat_input.pop();
    }

    pub fn chat_input_take(&mut self) -> String {
        std::mem::take(&mut self.chat_input)
    }

    pub fn scroll_chat_down(&mut self) {
        self.chat_scroll = self.chat_scroll.saturating_add(1);
    }

    pub fn scroll_chat_up(&mut self) {
        self.chat_follow_tail = false;
        self.chat_scroll = self.chat_scroll.saturating_sub(1);
    }

    pub fn update_processes(&mut self, running: Vec<ProcessSnapshot>, exited_at_unix: u64) {
        let current_pids: HashSet<i32> = running.iter().map(|p| p.pid).collect();
        for previous in &self.running_processes {
            if !current_pids.contains(&previous.pid) {
                self.exited_processes.insert(
                    0,
                    ExitedProcess {
                        snapshot: previous.clone(),
                        exited_at_unix,
                    },
                );
            }
        }

        self.running_processes = running;
        self.exited_processes.truncate(self.process_limit);

        if self.processes_follow_tail {
            self.processes_scroll = self.processes_max_scroll();
        } else {
            self.clamp_processes_scroll();
        }
    }

    pub fn scroll_processes_down(&mut self) {
        let max = self.processes_max_scroll();
        self.processes_scroll = (self.processes_scroll + 1).min(max);
        if self.processes_scroll >= max {
            self.processes_follow_tail = true;
        }
    }

    pub fn scroll_processes_up(&mut self) {
        self.processes_follow_tail = false;
        self.processes_scroll = self.processes_scroll.saturating_sub(1);
    }

    pub fn set_processes_viewport_rows(&mut self, rows: usize) {
        self.processes_viewport_rows = rows.max(1);
        if self.processes_follow_tail {
            self.processes_scroll = self.processes_max_scroll();
        } else {
            self.clamp_processes_scroll();
        }
    }

    pub fn set_processes_total_rows(&mut self, rows: usize) {
        self.processes_total_rows = rows;
        if self.processes_follow_tail {
            self.processes_scroll = self.processes_max_scroll();
        } else {
            self.clamp_processes_scroll();
        }
    }

    fn processes_max_scroll(&self) -> u16 {
        self.processes_total_rows
            .saturating_sub(self.processes_viewport_rows.max(1)) as u16
    }

    fn clamp_processes_scroll(&mut self) {
        let max_scroll = self.processes_max_scroll();
        if self.processes_scroll > max_scroll {
            self.processes_scroll = max_scroll;
        }
    }

    pub fn update_chat_messages(&mut self, messages: Vec<ChatMessage>) {
        self.chat_messages = messages;
        if self.chat_follow_tail {
            // Clamp to bottom during render once wrapped line count is known.
            self.chat_scroll = u16::MAX;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{App, ProcessSnapshot};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn empty_app() -> App {
        App::new(
            BTreeMap::new(),
            BTreeMap::new(),
            Vec::new(),
            PathBuf::from("runs"),
            0,
            0,
        )
    }

    #[test]
    fn update_processes_tracks_recently_exited() {
        let mut app = empty_app();
        let p1 = ProcessSnapshot {
            pid: 100,
            ppid: 1,
            state: "R".to_string(),
            elapsed: "00:01".to_string(),
            elapsed_secs: 1,
            cpu_pct: 10.0,
            mem_pct: 2.0,
            command: "python train.py".to_string(),
        };
        let p2 = ProcessSnapshot {
            pid: 101,
            ppid: 1,
            state: "S".to_string(),
            elapsed: "00:02".to_string(),
            elapsed_secs: 2,
            cpu_pct: 1.0,
            mem_pct: 0.5,
            command: "bash".to_string(),
        };

        app.update_processes(vec![p1.clone(), p2.clone()], 1_700_000_000);
        assert_eq!(app.running_processes.len(), 2);
        assert!(app.exited_processes.is_empty());

        app.update_processes(vec![p2], 1_700_000_100);
        assert_eq!(app.running_processes.len(), 1);
        assert_eq!(app.exited_processes.len(), 1);
        assert_eq!(app.exited_processes[0].snapshot.pid, p1.pid);
    }

    #[test]
    fn metric_display_name_uses_label_override() {
        let mut app = empty_app();
        app.metric_labels
            .insert("train/loss".to_string(), "Loss".to_string());
        assert_eq!(app.metric_display_name("train/loss"), "Loss");
        assert_eq!(app.metric_display_name("train/accuracy"), "train/accuracy");
    }
}
