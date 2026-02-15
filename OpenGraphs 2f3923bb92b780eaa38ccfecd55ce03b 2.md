# OpenGraphs

friction you’re trying to avoid (port forwarding, browser overhead, less pleasant over SSH

We want to build opengraphs (og) – a local‑first, TUI‑native experiment tracker for AI runs. A lot of us train models on remote GPUs or labs over shaky SSH; spinning up a browser dashboard with port‑forwarding is clunky, and plain logs don’t give good experiment management.

With opengraphs, you instrument once and get a fast terminal UI for live metrics and run graphs: quick comparison and filtering, overlays, metric search, and tag/config‑based grouping, all without leaving SSH. The goal is to cover the important parts of experiment tracking with agents (e.g., “Claude/codex explains why loss spiked and suggests checks that could later be extended into autonomous workflows”), but with a lightweight, Rust‑based workflow that fits how builders and small teams actually work day‑to‑day.

This is a strong hackathon idea: “htop for experiments” is a crisp wedge, and the pain (flaky SSH + no browser + W&B overhead) is real. The key is to stay terminal-native + local-first in a way that feels instant.

What I’d do for a hackathon MVP:

- Nail one tight workflow: og run python [train.py](http://train.py/) → auto-captures config/env, tails logs, and streams metrics live in a TUI.
- 3 killer screens: (1) runs list + filters (tags/config search), (2) run detail (metadata + log tail), (3) metric chart with overlay/compare (even 2-run compare is a huge wow).
- Simple storage that survives SSH pain: per-run folder with run.json (metadata/config), metrics.jsonl (append-only), stdout.log; optional index.sqlite later for fast querying.
- “W&B-like value without W&B scope”: config diff, tags, notes/annotations, metric search; skip artifacts/sync/auth for now.

Biggest scope traps to avoid this week: artifact management, cloud syncing, UI perfection, supporting every framework. Focus on fast ingestion + fast TUI + compare.

raw response observability.. 

## Trackio-rs client vs what to build

quick questions: 

- trackio-rs today is basically a write client, not a full experiment platform.
1. What it currently has (Rust SDK)
- Client::new() from env (TRACKIO_SERVER_URL, TRACKIO_PROJECT, TRACKIO_RUN, etc.).
- Builder setters: with_base_url, with_project, with_run.
- log(metrics, step, timestamp), in-memory batching, flush(), close().
- Auto-discovers bulk endpoint: /api/bulk_log or /gradio_api/bulk_log.
- Optional write-token header support.
- Inference: [batcher.rs](http://batcher.rs/) exists but [lib.rs](http://lib.rs/) only exports client, so batcher helpers are not part of the public API right now.
1. What OG should build on top
- Keep Trackio-compatible ingest, but make OG’s own Rust layer the real product:
- og-sdk-rs with typed APIs: run lifecycle, tags/config, system metrics, checkpoints.
- Durable local spool + retry/backoff (so logs survive crashes/network issues).
- UDS-native transport for local daemon.
- Read/query API (not just write): list runs, metric keys, compare, grouped views.
- Alert engine + agent hooks (explain, suggest, --auto guardrails).
1. How to build quick comparison/filter/overlay/search/grouping
- Store model:
- runs table: run_id, project, name, status, timestamps, tags, config.
- metrics table (long format): run_id, key, step/time, value.
- run_meta_kv flattened config/tags for fast filtering.
- API/query layer:
- GET /runs?filter=... (status/tag/config/date filters).
- GET /metrics/keys?q=... (metric search/autocomplete).
- POST /compare (run_ids + metric_key + align_by=step|time + smoothing + downsample).
- POST /group (group by tag/config key, aggregate mean/p50/p95).
- TUI behaviors:
- metric search box in Graphs tab,
- multi-select runs for overlays,
- group-by toggle (tag:dataset, config:lr),
- consistent smoothing/downsampling for responsive redraws.

## Quick thoughts

- I think we should have a sub tab for recent runs overview [ascii art since its static and could hover maybe]
- how should we build checkpoints for the training runs? 
- how should we build the agent chat messaging system? 
- don’t you think we need storage for both agent chat messaging system, checkpoints and runs metedata, config etc? 
- we plan on shipping thru curl and zerobrew, how do you think we should do it? 
- when you said we goona use unicode/braille should rust produce it again meeting efficiency and speed? 

## PRD

```python
Copy/paste this as your Codex prompt to produce a PRD.

***

You are writing a **Product Requirements Document (PRD)** for a new open-source CLI/TUI tool called **opengraphs** (alias `og`). It is a **local-first** terminal dashboard for ML training runs that extends **Trackio** for metrics storage/derived-metrics, and uses **OpenTUI** to render real-time graphs directly in the terminal (no browser, no port forwarding). Trackio’s default visualization is a local Gradio dashboard; opengraphs must provide a native terminal alternative. [huggingface](https://huggingface.co/blog/trackio)

## Goals
- Instant, low-latency visualization of training metrics in terminal (SSH/tmux friendly).
- Works when training runs are fully local/offline.
- Minimal friction: one command to start, sensible defaults, robust exit/help controls (inspired by btop/top UX where `h/?` = help and `q` = quit). [tecmint](https://www.tecmint.com/btop-system-monitoring-tool-for-linux/)

## Non-goals (v0)
- No cloud syncing, collaboration, or web UI.
- No artifact hosting or model registry.

## PRD must include (sections)
1. **Problem & user stories** (single-node dev, SSH to cluster, long-running runs, DDP runs).
2. **CLI surface area**: exact commands, flags, examples, exit/help behavior.
3. **TUI UX**: screens, layout, keybindings, interactions.
4. **Data model**: what you read/write from Trackio, run discovery, how “live” updates work.
5. **Production requirements**: reliability, performance, crash-safety, security, logging, testing.
6. **Milestones**: v0 / v0.1 / v1 scope.

## Command design (propose and specify)
Use `og` as the primary binary, `opengraphs` as synonym.

### Core commands
- `og --help` (must exist, concise usage).
- `og view [PATH|RUN_ID]` opens TUI for an existing run or project directory.
- `og watch -- python train.py ...` runs a training command and attaches the TUI; on exit, returns the same exit code as the training process.
- `og list [PATH]` lists runs found locally (with filters).
- `og doctor` validates environment and trackio store, prints diagnostics.
- `og export [RUN_ID] --format csv|json` exports scalars (optional in v0 if easy).

### Key flags
- `--project`, `--run`, `--store PATH` (where Trackio data lives).
- `--refresh-ms`, `--follow` (tail live updates).
- `--filter metric_regex`, `--tags key=value`.
- `--no-color` and `--theme`.
- `--ui minimal|full` (optional).

### TUI controls
- `h` or `?`: help overlay with keybindings. [man7](https://man7.org/linux/man-pages/man1/top.1.html)
- `q`: quit (prefer “exit menu” like btop: Options / Help / Quit). [tecmint](https://www.tecmint.com/btop-system-monitoring-tool-for-linux/)
- `/`: search metrics.
- `tab`: switch panels.
- `enter`: focus/select a chart.
- `r`: refresh/rescan.
- `p`: pause/resume live updates.
- `s`: screenshot/export current view (optional).

## Important production features to think through
Include requirements for:
- **Low overhead**: logging should not slow training.
- **Crash-safe reads**: handle partially-written files; never corrupt stores.
- **Multi-process**: DDP/ranks writing concurrently; safe aggregation story.
- **Large scale**: thousands of metrics, millions of points; downsampling strategy.
- **Robust discovery**: find runs under `./trackio` or user-provided store.
- **Deterministic derived metrics**: compute moving averages, throughput, ETA.
- **Resource view (optional)**: CPU/RAM/GPU utilization panel (btop inspiration).
- **Extensibility**: plugin hooks for new panels/metric transforms.

## Deliverable format
- PRD in Markdown.
- Keep it concrete: tables for commands/flags, explicit acceptance criteria per milestone.
- Include at least one “Happy path” and one “Failure mode” flow for `og watch -- ...`.
- No code; only product + technical requirements.

Now write the PRD.

---
```

## Shortcuts

```python
current TUI keybindings we explicitly handle (for the search input):

  - ⌘C: copy selection or whole input
  - ⌘V / Ctrl+V / ^V (\u0016): paste (clipboard fallback)
  - ⌃U (\u0015) or ⌘⌫: delete all left (line start)
  - ⌃W (\u0017) or ⌥⌫ (incl. ESC+DEL / ESC+BS): delete word backward
  - ⌃A (\u0001) or ⌘←: cursor to line start
  - ⌃E (\u0005) or ⌘→: cursor to line end
  - ⌃K (\u000b): delete all right (to line end)

  And we also keep:

  - / focuses the input (if not already focused)
  - Esc blurs the input
  - q quits the TUI
```

## ~ Remotion

```python
Codex prompt (creative brief) — build a 30–40s promo in Remotion with a MacOS terminal-window hero frame (1280×1000), plus 16:9 and 9:16 exports derived from the same layout.[remotion+1](https://www.remotion.dev/docs/composition)

## Goal

Create a designer-level promo for “OpenGraphs”: a TUI-native graph dashboard that attaches to an ML training run and auto-refreshes live, btop vibes but for experiment metrics.[terminaltrove+1](https://terminaltrove.com/btop/)

## Deliverables

- 3 compositions: `TerminalHero_1280x1000`, `Landscape_1920x1080`, `Vertical_1080x1920` (keep everything crop-safe from the 1280×1000 master).[[remotion](https://www.remotion.dev/docs/composition)]
- Dark premium UI: subtle grain, neon accents (cyan/lime), crisp mono typography, clean separators, minimal glow.

## Motion language

- Everything should feel “systems-fast”: snap-ins, elastic overshoot on panels, micro-jitter-free counters, no floaty slow fades.[remotion+1](https://www.remotion.dev/docs/interpolate)
- Prefer spring-based entrances for panels and badges (slight overshoot then settle), use `interpolate()` + `Easing` for small UI nudges, and clamp values to avoid drift.[remotion+2](https://www.remotion.dev/docs/spring)

## Visual design spec (terminal hero)

- MacOS terminal frame (rounded corners, top traffic-light dots) with a subtle background blur behind it.
- Inside the terminal: a top status bar: `run_name | state=running | step=###` (step increments continuously).
- Main area: 2×2 panel grid inspired by btop layout: each panel has a compact title, y-axis min/max ticks, and two lines: `raw` (thin, noisy) + `ma(15)` (thicker, smooth).[[terminaltrove](https://terminaltrove.com/btop/)]
- Bottom status line: `source: train.jsonl | refresh: 250ms | mode: live` (timestamp updates).

## Data + realism

Use stylized-but-believable metric shapes like the attached plot: rewards trending up with variance, eval reward/acc gradually rising, KL loss slowly creeping with occasional spikes.

Make the “live” feel obvious: new points push in from the right, the window scrolls, and the last-value readout updates every refresh.

## Storyboard (timecoded, 30–40s)

0–2s

Black → cursor blink → type command: `opengraphs attach --source train.jsonl` (keystroke audio, crisp).

2–6s

Terminal window pops in (spring overshoot) → status line shows `state=running` and `step=000` begins ticking upward.[[remotion](https://www.remotion.dev/docs/spring)]

6–10s

Dashboard grid “builds itself”: 4 panels snap into place one-by-one (each with a subtle bass hit), lines start drawing from left to right as if streaming.

10–14s

Auto-discovery moment: a toast appears: `new series discovered: critic/rewards/mean` → a new panel animates in, layout reflows smoothly (no jank).

14–18s

“Link anything” moment: a selector highlights metric names (`loss`, `lr`, `gpu_mem`, `reward_mean`) and “bind” animations connect them to panels (clean vector arrows, then arrows dissolve).

18–24s

Performance flex: zoom into a dense region → show a quick “downsample / multi-res” effect: raw line stays visually consistent while point density reduces; overlay text: `smooth live redraw`.

24–30s

Remote workflow: show an `ssh user@box` badge in the corner; despite “remote”, the dashboard stays live; bottom line flashes: `offline, local-first`.

30–36s

End card inside the terminal (keep the terminal framing):

“OpenGraphs” (logo text), then tagline: “Observability that lives in your terminal.”

36–40s

CTA: `Star / Join beta / Coming soon` with a final cursor blink.

## On-screen copy (short, punchy)

- “TUI-native metrics.”
- “Attach to any run.”
- “Autonomous refresh.”
- “Fast. Local-first. Hackable.”
- “No browser required.”

## Implementation notes (so Codex can execute)

- Register compositions with explicit `width` / `height` in `<Composition>` for 1280×1000, 1920×1080, 1080×1920.[[remotion](https://www.remotion.dev/docs/composition)]
- Ensure H.264 renders use even dimensions (all above are even) and avoid odd scaling artifacts.[[github](https://github.com/remotion-dev/remotion/issues/808)]
- Use `useVideoConfig()` to adapt spacing/font sizes per composition so vertical doesn’t feel cramped.[[remotion](https://www.remotion.dev/docs/use-video-config)]
- Use `spring()` for panel entrances and badge pops (intentional overshoot), and `interpolate()` for counters/opacity/position timing.[remotion+1](https://www.remotion.dev/docs/interpolate)

If you want, I can also turn this into a concrete “shot list JSON” (timings, components, and animation primitives) that Codex can translate directly into Remotion scenes.

Here are more natural, human-sounding versions you can paste directly, with a bit of imperfection and voice so they don’t feel AI‑generated.
```

## Features

```python
features like 
fix the agent chat first
agent chat from graphs tab [redirecting] when hit enter (agent chat should be in sync)
byok/claude api setup
step progress bar (like htop)

backend
metrics graphs
stats (attached image comes in stats)
sys graphs
scrolls for each window  (chat, graphs, log)
log tab - tail (simplest feat)

user preferences like if the user wants only two/more graphs on the graphs tab (rewards, loss) should be a flag input something like for instance; --graphs reward: --graphs {reward, loss}
training checkpoints (restart from training checkpoint if the run crashes)

auto flag for users to autonomously iterate on it in their workflows og run —auto <file> (agent keeps tracking the graphs and if something’s wrong it’ll give an alert maybe then automatically refactor the code and restart at checkpoint) -> think this kinda imp to stand out of competitors no? share thoughts tho

agent (optional)
resume
feedback (like codex)

currently the graphs tab opens when user prompts !og run <file> in the in-app chat this is fine but we need to get this working from outside of the in-app chat as well (I think that's kinda simple) 

alias
og/opengraphs
og/opengraphs run/-r <file>
og/opengraphs tail/-t  <log >
og/opengraphs resume (optinal)
og/opengraphs run/-r --graphs reward <file>
og/opengraphs run/-r --graphs {reward, loss} <file>

og/opengraphs run/-r <file> <prompt> (prompt in quotes)
 

```

```python
1. Reproducibility bundle: save everything needed to rerun an experiment exactly later (code version, settings, environment, seed, machine info).
  2. Strong compare UX: make it very easy to answer “what changed between run A and run B, and did it help?”
  3. Saved views/presets: let users save favorite filters and graph layouts so they don’t rebuild the same screen every time.
  4. Checkpoint lineage: show a simple “family tree” of runs (this run resumed from that checkpoint).
  5. Reliable ingest on bad SSH: don’t lose logs/metrics when network is flaky; queue and retry safely.
  6. Good alerts (not noisy alerts): alert when something is truly wrong, avoid spamming duplicates.
  7. Rich eval artifacts: show actual model outputs/examples (not just numbers), so users can debug quality faster.

```

# MOAT

Real product: also yes, if you lock down a few core things next:

1. Reliability first: run durability, crash-safe writes, resume/checkpoint flow.
2. Clear “aha” loop: og run -> live graphs -> alert -> actionable agent suggestion.
3. Agent safety boundaries: explicit allowlist for what --auto can change/restart.
4. Strong defaults: great out-of-box metric panels, sensible alerts, zero-friction setup.
5. Interop: keep Trackio-compatible ingest stable while your Rust path gets richer.

Your core wedge is strong: local-first TUI over SSH + fast graphing + optional agentic workflows. That’s a real product direction, not just a hackathon demo.

› whats our moat

- Your moat is the tight loop, not any single feature:
1. SSH-native local-first UX that just works on flaky remote setups without browser/port-forward pain.
2. Fast Rust hot path + TUI-first rendering for low-latency monitoring where people actually work.
3. Actionable intelligence loop: metrics -> alerts -> agent explanation -> checkpoint-aware fix/restart (--auto).
4. Workflow lock-in via convenience: one instrumentation path, consistent CLI, presets, and run context that teams don’t want to rebuild elsewhere.
5. Open compatibility edge (Trackio-compatible ingest) plus your own higher-level operator workflow on top.

If you execute --auto safely and reliably, that becomes the strongest moat because most trackers stop at visualization.

## Questions to ask users

1. “How many of you track training with W&B or Trackio?
2. “How many have fought SSH/port-forwarding or reconnect issues just to see live metrics?”
3. “How many have noticed a bad run too late and lost hours of GPU time?”

Then pivot:

- “OpenGraphs is for that exact pain: local-first, SSH-native, TUI tracking with alerts and agent help.”

Demo flow (2-3 mins):

1. Start a run with og run ... in terminal.
2. Show live graphs + stats + logs in one TUI.
3. Trigger a bad trend and show alert + agent explanation + suggested next check (and checkpoint resume if ready).

# judging

Technicality | Originality | Practicality | Aesthetics | Wow factor

- Technicality: 7.0/10
Strong base (Rust daemon, TUI, UDS, live ingestion), but still missing deeper reliability and query depth for production confidence.
- Originality: 8.5/10
SSH-native, local-first TUI + agent-assisted debugging loop is genuinely differentiated.
- Practicality: 6.5/10
Real painkiller for your target users, but needs stronger run lifecycle features (resume/checkpoint, robust compare/filter/search/grouping) to be daily-driver ready.
- Aesthetics: 8.0/10
The TUI direction is clean and intentional; much stronger than typical hackathon terminal tools.
- Wow factor: 7.5/10
High ceiling, but wow depends on demoing one full “bad run -> alert -> agent insight -> action” loop live.

Simple questions 
• What if deployment fails? Do I have a backup ready just in case?
• Is my judging demo well balanced or too technical/explanatory ?
• What are the hard questions I should prep that could be asked on this project?

## Main Context flow

1. Use this flow: message -> context builder -> model -> action plan -> guarded executor -> result.
2. Context builder should include: latest alerts, recent metric trends, log tail, selected files.

### Agent chat

```python
```markdown
# Task: Implement Agent Chat System for OpenGraphs

## Context
OpenGraphs is a terminal-first ML experiment tracker. We need to add an agent chat system that monitors training runs, detects issues via alerts, and optionally refactors code to fix problems.

## Architecture Overview
- `ogd` (daemon): Python sidecar handling metrics, logs, and agent logic
- TUI: Rust terminal UI with chat/graphs/logs tabs
- Communication: Unix domain socket (`OGD_SOCKET`)
- BYOK: User provides their own LLM API key

## Implementation Requirements

### 1. Agent Module in `ogd` (`agent.py`)

Create agent loop that:
- Monitors `RunState` for active alerts
- Builds focused context when alert fires
- Calls user's LLM (OpenAI/Claude/etc via their API key)
- Parses response for action plan
- Executes actions with safety guards

**Key Classes:**

```python
class ContextBuilder:
    """Assembles LLM context from run state"""
    def build_context(self, run_state: RunState) -> str:
        # Include:
        # - System prompt (role definition)
        # - Latest alert details (metric, threshold, current value)
        # - Recent metric trends (last 20 points)
        # - Log tail (last 50 lines, capture errors/warnings)
        # - Training file content (full script)
        pass

class ActionPlanner:
    """Parses LLM output into structured plan"""
    def parse_response(self, llm_output: str) -> ActionPlan:
        # Extract sections: DIAGNOSIS, ACTION, CODE_CHANGES
        # ACTION types: "explain" (no code) or "refactor" (modify code)
        pass

class GuardedExecutor:
    """Safely executes action plans with rollback"""
    def __init__(self, auto_mode: bool):
        self.auto_mode = auto_mode
        self.checkpoint_dir = Path(".og_checkpoints")
    
    async def execute(self, plan: ActionPlan, run_state: RunState) -> ExecutionResult:
        # 1. Create checkpoint (copy training file + save state)
        # 2. If action == "explain": just return diagnosis
        # 3. If action == "refactor" and auto_mode:
        #    - Validate diff format
        #    - Apply unified diff to training file
        #    - Restart training process
        # 4. On failure: restore from checkpoint
        pass
```

**System Prompt Template:**
```
You are an ML training assistant for OpenGraphs.
Role: Diagnose issues and suggest code fixes when metrics plateau/degrade.

Response format:
DIAGNOSIS: <analysis of the problem>
ACTION: <explain|refactor>
CODE_CHANGES: <if refactor, provide unified diff starting with --- and +++>
```

### 2. Chat Endpoint in `ogd`

Expose over Unix socket:
```python
# Add to ogd socket handlers
async def handle_chat_message(self, user_message: str):
    """User sent message from TUI"""
    # Append to chat history
    # Trigger agent response
    # Return agent's reply

async def get_chat_history(self):
    """TUI polls for chat messages"""
    return self.chat_messages  # List of {sender, content, timestamp}
```

### 3. TUI Chat Tab (`chat.rs`)

Add chat rendering:
- Poll `ogd` for chat messages every 500ms
- Display as scrollable list with colored senders:
  - User messages: cyan
  - Agent messages: green
  - System messages: dim gray
- Show streaming indicator when agent is thinking
- In `--auto` mode, show "⚡ Auto mode: Agent will apply fixes" banner

### 4. Alert-Triggered Agent Flow

```python
async def agent_loop(run_state: RunState, auto_mode: bool):
    """Main agent monitoring loop"""
    while run_state.is_active:
        if alert := check_for_alert(run_state):
            # Build context
            context = context_builder.build_context(run_state)
            
            # Call LLM
            llm_response = await call_llm(
                context, 
                model=os.getenv("OG_AGENT_MODEL", "gpt-4"),
                api_key=os.getenv("OPENAI_API_KEY")
            )
            
            # Parse plan
            plan = action_planner.parse_response(llm_response)
            
            # Add to chat
            self.add_chat_message("agent", plan.diagnosis)
            
            # Execute if auto
            if auto_mode and plan.action == "refactor":
                result = await executor.execute(plan, run_state)
                if result.success:
                    self.add_chat_message("system", 
                        f"✓ Code refactored. Training restarted from checkpoint {result.checkpoint_id}")
                else:
                    self.add_chat_message("system", 
                        f"✗ Failed: {result.error}. Rolled back.")
        
        await asyncio.sleep(5)  # Check every 5 seconds
```

### 5. Checkpoint/Rollback Logic

```python
def create_checkpoint(run_state: RunState) -> str:
    checkpoint_id = f"ckpt_{int(time.time())}"
    ckpt_path = Path(".og_checkpoints") / checkpoint_id
    ckpt_path.mkdir(parents=True, exist_ok=True)
    
    # Copy training file
    shutil.copy(run_state.training_file, ckpt_path / "training_script.py")
    
    # Save state
    with open(ckpt_path / "state.json", 'w') as f:
        json.dump({
            "metrics": run_state.metrics,
            "step": run_state.current_step,
        }, f)
    
    return checkpoint_id

def restore_checkpoint(checkpoint_id: str, run_state: RunState):
    ckpt_path = Path(".og_checkpoints") / checkpoint_id
    shutil.copy(ckpt_path / "training_script.py", run_state.training_file)
```

### 6. Diff Application

Use `unidiff` library to parse and apply:
```python
import unidiff

def apply_diff(filepath: Path, diff_text: str):
    patch = unidiff.PatchSet(diff_text)
    
    with open(filepath, 'r') as f:
        original_lines = f.readlines()
    
    # Apply patch logic (iterate hunks, replace lines)
    modified_lines = apply_patch_hunks(original_lines, patch)
    
    # Atomic write
    tmp = filepath.with_suffix('.tmp')
    with open(tmp, 'w') as f:
        f.writelines(modified_lines)
    tmp.rename(filepath)
```

## Demo Flow

User runs: `og run train.py --auto`

1. Training starts, metrics collected
2. Alert fires: "loss not decreasing for 20 steps"
3. Agent triggered automatically:
   - Builds context (metrics + logs + code)
   - LLM diagnoses: "Learning rate too high"
   - LLM suggests: Change `lr=0.01` to `lr=0.001`
4. In `--auto` mode:
   - Creates checkpoint
   - Applies diff to `train.py`
   - Restarts training
5. TUI shows:
   - Chat: Agent explanation + actions taken
   - Graphs: Metrics recovering after fix
   - Logs: Training restarted message

## Files to Create/Modify

1. `ogd/agent.py` - New file with ContextBuilder, ActionPlanner, GuardedExecutor
2. `ogd/socket_handler.py` - Add chat endpoints
3. `tui/src/chat.rs` - New chat tab rendering
4. `tui/src/app.rs` - Wire up chat tab in tab switcher
5. `ogd/alerts.py` - Hook agent trigger on alert detection

## Success Criteria

- Agent auto-triggers on alerts
- Chat shows diagnosis in TUI
- `--auto` flag enables code refactoring
- Checkpoint/rollback works on failure
- Training restarts after successful fix
- All in terminal, no browser needed
```
```

## Quick Questions to know

1. “How is this different from W&B/Trackio/TensorBoard?”
OpenGraphs is SSH-native and terminal-first, with an alert-to-action loop (detect -> explain -> fix/resume) instead of dashboard-only visibility.
2. “Why not just build a plugin on top of existing tools?”
We keep ingest compatibility, but the product value is the local TUI control loop and low-latency operator workflow, which plugins don’t solve cleanly.
3. “What proof do you have that it’s fast?”
Be ready with 3 numbers: time-to-first-metric, median update latency, and CPU overhead during training.
4. “How reliable is this on flaky SSH or process crashes?”
Local-first design, durable writes, resumable checkpoints, and replayable logs keep runs inspectable even after disconnects.
5. “How do you prevent noisy/false alerts?”
Use smoothed trend windows, minimum step horizon, and cooldown/dedup so alerts are sparse and actionable.
6. “Is --auto safe or reckless?”
It is bounded: allowlisted files only, patch-size limits, checkpoint before action, rollback if no improvement.
7. “What if the model gives a bad refactor?”
We gate execution with policy checks, verify post-change metric window, and auto-revert on regression.
8. “How do you prove the agent actually helped?”
Show before/after trend windows from the same run and checkpoint lineage, not just narrative.
9. “How do you handle secrets and privacy with BYOK?”
Keys are user-owned, stored locally, least-privilege access, no mandatory cloud relay for core tracking.
10. “Can this scale beyond one toy run?”
Roadmap: downsampling, indexed queries, run filters/grouping, and bounded-memory rendering for many runs/metrics.
11. “How does this work with distributed training (DDP/multi-node)?”
Aggregate rank metrics into canonical run streams with rank tags and deterministic merge rules.
12. “What is the moat long-term?”
The sticky moat is workflow speed: SSH-native live observability + safe agentic recovery + checkpoint-aware iteration loop.

OG VS OGD 

1. og vs ogd
2. og: user CLI/front door (run, tail, list, resume).
3. ogd: daemon/backend engine (ingest, storage, query, alerts).
4. Keep og thin and ogd authoritative.

**CODEX-RUST based inspiration for agent chat**

What to copy from Codex (not the whole thing)

1. Separate core logic from UI (Codex has core/protocol/tui split).
2. Keep a small protocol/event layer between chat engine and UI.
3. Keep packaging flexible (native binary first, wrappers optional later).

Minimal Rust agent-chat design for OG (ogd)

1. chat_session
    - stores session_id, messages, run_id, timestamps.
2. context_builder
    - pulls last N alerts, recent metric trend summary, last log lines, mentioned files.
3. provider trait
    - generate(request) -> response for BYOK (OpenAI/Claude/etc).
4. tool_executor
    - read/write/patch/restart/checkpoint actions with guardrails.
5. auto_loop
    - rule trigger (reward_ma_slope <= 0 for N steps) -> explain -> patch -> resume -> verify.
6. event_bus
    - tokio::broadcast/mpsc for streaming partial/final responses to TUI.

Hackathon API surface (UDS, simple)

1. POST /chat/send
2. GET /chat/stream?session_id=... (SSE or newline JSON)
3. POST /auto/evaluate
4. POST /auto/apply
5. POST /runs/:id/resume

Guardrails (must-have even in prototype)

1. allowlisted editable paths only
2. max files/lines changed per auto action
3. checkpoint before apply
4. rollback if no improvement after verify window
5. full audit log of prompt/action/diff/result

Why this is enough

- Inference from Codex architecture: modular boundaries matter more than feature count for reliability.
- Don’t build Codex-scale crate graph; keep this inside ogd modules now, split crates later only if needed.

- Put agent chat backend in crates/ogd so one daemon owns run state, alerts, logs, and agent actions.
- Keep apps/tui as UI only: send chat messages, render responses/status.
- Keep crates/og as CLI/orchestration only.

To keep it clean, do this in ogd:

1. chat_api (send/stream messages)
2. agent_runtime (provider calls, context builder)
3. auto_executor (--auto guarded patch/apply/resume)

If it grows, extract agent_runtime into a separate Rust  crate later, but for hackathon speed, starting inside ogd is ideal.

Use this for live:

1. Dataset: openai/gsm8k with train[:64] (or [:128] max)
2. Steps: 120-220 total
3. Log every 2-5 steps
4. Checkpoint every 30-50 steps
5. Trigger alert by step 40-70
6. Auto-fix + resume by step 80-120
7. Show recovery trend by step 150-220

For a 5-minute slot, ideal flow:

1. 30s pain framing
2. 60s run + live graphs
3. 45s alert + agent explain
4. 45s auto patch + resume
5. 30s trend recovery proof
6. 30s close

Use pre-recorded backup regardless, but this live config is realistic for 5 minutes.