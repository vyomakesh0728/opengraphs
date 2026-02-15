# OpenGraphs Hack Day Brief

## 1) One-line Pitch
OpenGraphs (`og`) is a local-first, TUI-native experiment tracker for AI runs over SSH: live metrics, logs, alerts, and optional agent guidance without browser dashboards or port-forwarding.

## 2) The Problem
Training on remote GPUs over unstable SSH is common, but current workflows are painful:

1. Browser-first trackers are clunky over port-forwarding.
2. Plain logs are not enough for fast experiment debugging.
3. Bad runs are often discovered too late, after wasting GPU time.

## 3) Who It’s For
Builders and small research teams who:

1. Run training jobs on remote machines/labs.
2. Live in terminal + tmux.
3. Need fast iteration, not heavyweight MLOps stacks.

## 4) What Makes OpenGraphs Different
The moat is the tight loop:

1. `og run` starts and attaches quickly in terminal.
2. Live graphs + stats + logs update in one TUI.
3. Alerts detect bad trends early.
4. Agent explains likely causes and suggests next checks.
5. Resume/checkpoint workflow shortens recovery time.

## 5) Demo Scope (Hack Day v0)
What we show:

1. Run tracking in terminal tabs (`chat`, `graphs`, `logs`).
2. Live metric and system graph panels.
3. Log tail in-app.
4. Basic alert + agent explanation flow.
5. Local-first architecture with Unix domain socket internals.

What we explicitly do not claim today:

1. Cloud sync, team collaboration, model registry.
2. Full artifact management.
3. Fully autonomous code edits without safety boundaries.

## 6) Architecture Snapshot
High-level runtime path:

1. Training script emits metrics.
2. Trackio-compatible ingest enters `ogd`.
3. `ogd` stores/query-prepares local run data.
4. TUI reads from local daemon and renders live panes.
5. Python sidecar agent provides explain/suggest signals. -> this isn't optional 

Current internal transport:

1. `ogd` serves over Unix domain socket (`OGD_SOCKET`).
2. TUI health checks and local flows use the same socket path.

## 7) Demo Script
### 90-second version
1. Ask 3 questions:
   - Who uses W&B/Trackio/TensorBoard?
   - Who has fought SSH or port-forwarding issues for live metrics?
   - Who has discovered bad runs too late?
2. Run `og run <file>`.
3. Show live `graphs` + `stats` + `logs`.
4. Trigger a visible bad trend and show alert + agent explanation.
5. Close with: "Same terminal workflow, faster feedback loop, less wasted GPU time."

### 3-minute version
1. Open with pain + one-line thesis.
2. Show `chat`, `graphs`, `logs` tab flow.
3. Show metric trend interpretation (raw/smooth if enabled).
4. Demonstrate alert handling and suggested checks.
5. Show checkpoint/resume path (or describe if partial).
6. End on why local-first TUI wins over browser-heavy flows for SSH users.

## 8) Demo Backup Plan
If something breaks live:

1. Use a pre-recorded run with stable metric streams.
2. Manually trigger alert state and show agent explanation output.
3. Fall back to CLI + logs + one graph view and narrate expected flow.

## 9) Judge Questions To Prepare For
### "How is this different from Trackio/W&B?"
OpenGraphs is SSH-native and terminal-first. It optimizes the day-to-day inner loop for remote training, while keeping Trackio-compatible ingestion.

### "Is the agent safe?"
`--auto` is scoped with guardrails. We separate suggest/alert from action and keep risky operations explicit.

### "Can this scale?"
Roadmap includes downsampling, filtering, grouped comparisons, and durable local storage for long runs.

### "Why not just use logs?"
Logs show events, but not trend shape and run-to-run comparisons. OpenGraphs gives fast visual diagnosis in terminal.

## 10) Success Criteria For Hack Day
We should leave judges with proof on these metrics:

1. Time-to-first-live-metric is fast.
2. TUI updates stay responsive during training.
3. Alert-to-explanation loop is clear and useful.
4. Demo runs end-to-end without browser dependency.

## 11) Immediate Post-Hack Priorities
1. Durable storage and crash-safe run lifecycle.
2. Strong compare/filter/search/grouping UX.
3. Clear `--auto` policy and audit trail.
4. Better defaults for graph panels and alert thresholds.
