# OpenGraphs Golden Demo Path

## Goal
Show one crisp end-to-end loop:

`run -> live graphs -> alert -> agent explain -> auto refactor -> resume from checkpoint -> trend improves`

## Demo Setup (Recorded + Live)
1. Keep one known-good training project and dataset ready.
2. Keep one pre-recorded run capture ready for safety.
3. Keep one config that intentionally causes flat/no-upward reward trend.
4. Ensure latest checkpoint is present and resumable.
5. Ensure BYOK key is already configured.

## On-Stage Script (3-4 minutes)
1. Open with pain statement:
   - "Remote GPU training over SSH is still painful: browser tabs, port-forwarding, and late failure detection."
2. Start with command:
   - `og run --auto <train_file>`
3. Show TUI tabs:
   - `chat`, `graphs`, `logs` all in one terminal.
4. Show live metrics:
   - reward/loss + system graphs updating.
5. Let alert trigger:
   - "No upward reward trend in the last N steps."
6. Show agent response:
   - brief explanation of likely issue.
   - proposed code/config patch.
7. Show guarded auto action:
   - patch applied to allowlisted files only.
   - resume from latest checkpoint.
8. Show post-action improvement:
   - reward trend starts moving upward.
9. Close:
   - "This is local-first, SSH-native experiment tracking with an actionable agent loop."

## Exact Opening (15s)
Use this line as-is:

"OpenGraphs is observability that lives in your terminal. No browser, no port-forwarding, and now with alert-driven agent actions."

## Required Visual Proof Points
1. Training is live (step counter and graphs moving).
2. Alert condition is explicit and measurable.
3. Agent output is specific (not generic text).
4. Auto action is visible (patch summary + resume event).
5. Result window shows improvement after intervention.

## Command Sequence (Reference)
```bash
# terminal 1
cargo run -p ogd

# terminal 2
bun run tui

# in app or via CLI
og run --auto <train_file>
```

## Judging Soundbites
1. "No browser, no port-forwarding, no context switch."
2. "We catch bad runs early, not 2 GPU-hours later."
3. "Agent suggestions are grounded in live metrics + logs + checkpoint-aware recovery."
