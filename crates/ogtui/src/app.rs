use std::collections::BTreeMap;
use std::path::PathBuf;

/// Which tab is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Graphs,
    Logs,
}

impl Tab {
    pub const ALL: &[Tab] = &[Tab::Graphs, Tab::Logs];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Graphs => "graphs",
            Tab::Logs => "logs",
        }
    }

    pub fn next(self) -> Tab {
        match self {
            Tab::Graphs => Tab::Logs,
            Tab::Logs => Tab::Graphs,
        }
    }
}

/// Application state.
pub struct App {
    pub active_tab: Tab,
    /// tag → { run_name → sorted (step, value) pairs }
    pub scalars: BTreeMap<String, BTreeMap<String, Vec<(f64, f64)>>>,
    /// Ordered list of tag names for grid iteration
    pub tags: Vec<String>,
    /// Ordered list of run names for color assignment
    pub run_names: Vec<String>,
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
    /// Frame counter for animations (title scrolling)
    pub tick_count: u64,
}

impl App {
    pub fn new(
        scalars: BTreeMap<String, BTreeMap<String, Vec<(f64, f64)>>>,
        log_lines: Vec<String>,
        events_path: PathBuf,
        total_events: usize,
        max_step: i64,
    ) -> Self {
        let tags: Vec<String> = scalars.keys().cloned().collect();
        // Collect all unique run names across all tags
        let mut run_set = std::collections::BTreeSet::new();
        for runs in scalars.values() {
            for run_name in runs.keys() {
                run_set.insert(run_name.clone());
            }
        }
        let run_names: Vec<String> = run_set.into_iter().collect();
        Self {
            active_tab: Tab::Graphs,
            scalars,
            tags,
            run_names,
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
            tick_count: 0,
        }
    }

    pub fn cycle_tab(&mut self) {
        self.active_tab = self.active_tab.next();
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
        if self.metrics_cols == 0 { return; }
        let selected_row = self.selected_metric / self.metrics_cols;
        if selected_row < self.metrics_scroll {
            self.metrics_scroll = selected_row;
        } else if selected_row >= self.metrics_scroll + self.metrics_visible_rows {
            self.metrics_scroll = selected_row + 1 - self.metrics_visible_rows;
        }
    }

    pub fn scroll_metrics_down(&mut self) {
        if self.tags.is_empty() || self.metrics_cols == 0 { return; }
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
        if self.tags.is_empty() || self.metrics_cols == 0 { return; }
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
}
