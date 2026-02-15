use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::socket_client::{ActionPlanResponse, ChatMessage};

/// Which tab is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Graphs,
    Logs,
    Chat,
}

impl Tab {
    pub const ALL: &[Tab] = &[Tab::Graphs, Tab::Logs, Tab::Chat];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Graphs => "graphs",
            Tab::Logs => "logs",
            Tab::Chat => "chat",
        }
    }

    pub fn next(self) -> Tab {
        match self {
            Tab::Graphs => Tab::Logs,
            Tab::Logs => Tab::Chat,
            Tab::Chat => Tab::Graphs,
        }
    }
}

/// Application state.
pub struct App {
    pub active_tab: Tab,
    /// tag → sorted (step, value) pairs
    pub scalars: BTreeMap<String, Vec<(f64, f64)>>,
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
}

impl App {
    pub fn new(
        scalars: BTreeMap<String, Vec<(f64, f64)>>,
        log_lines: Vec<String>,
        events_path: PathBuf,
        total_events: usize,
        max_step: i64,
    ) -> Self {
        let tags: Vec<String> = scalars.keys().cloned().collect();
        let daemon_socket = crate::socket_client::socket_path();
        Self {
            active_tab: Tab::Graphs,
            scalars,
            tags,
            log_lines,
            show_help: false,
            events_path,
            logs_scroll: 0,
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
        }
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
        let max_logs_scroll = self.log_lines.len().saturating_sub(1) as u16;
        if self.logs_scroll > max_logs_scroll {
            self.logs_scroll = max_logs_scroll;
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
        self.logs_scroll = self.log_lines.len().saturating_sub(1) as u16;
        self.last_daemon_log_tail.clear();
        self.seen_alert_count = 0;
    }

    pub fn append_live_log(&mut self, line: impl Into<String>) {
        self.log_lines.push(line.into());
        self.logs_scroll = self.log_lines.len().saturating_sub(1) as u16;
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn scroll_logs_down(&mut self) {
        let max = self.log_lines.len().saturating_sub(1) as u16;
        self.logs_scroll = (self.logs_scroll + 1).min(max);
    }

    pub fn scroll_logs_up(&mut self) {
        self.logs_scroll = self.logs_scroll.saturating_sub(1);
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
        let max = self.chat_messages.len().saturating_sub(1) as u16;
        self.chat_scroll = (self.chat_scroll + 1).min(max);
    }

    pub fn scroll_chat_up(&mut self) {
        self.chat_scroll = self.chat_scroll.saturating_sub(1);
    }

    pub fn update_chat_messages(&mut self, messages: Vec<ChatMessage>) {
        self.chat_messages = messages;
        // Auto-scroll to bottom
        let len = self.chat_messages.len();
        if len > 0 {
            self.chat_scroll = len.saturating_sub(1) as u16;
        }
    }
}
