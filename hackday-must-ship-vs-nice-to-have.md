# Hack Day Scope: Must Ship vs Nice To Have

## Must Ship (P0)
1. **Core CLI + daemon path works**
   - `ogd` boots cleanly.
   - `og run <file>` starts the run loop.
2. **TUI with three tabs**
   - `chat`, `graphs`, `logs` with clickable/switchable tabs.
3. **Live graphing in terminal**
   - metrics + system graph boxes render and update during run.
4. **Logs tab tail**
   - active run logs stream in-app.
5. **Alert trigger**
   - clear rule for bad trend (example: reward moving average stagnation window).
6. **Python agent sidecar integrated (required)**
   - receives alert context (trend + log tail + selected files).
   - returns short explanation + suggested change.
7. **`--auto` path (bounded)**
   - allowlisted file edits only.
   - apply patch + resume from latest checkpoint.
   - show what changed to user before/after apply.
8. **Checkpoint resume works**
   - run resumes from saved checkpoint after auto action.
9. **UDS internal transport**
   - local components communicate via `OGD_SOCKET`.
10. **Golden demo path recorded and reproducible**
   - same sequence can be replayed reliably during judging.

## Nice To Have (P1)
1. Multi-run overlay compare in graphs.
2. Metric search + tag/config filters in TUI.
3. Checkpoint family tree visualization in stats pane.
4. Saved graph presets (`--graphs` selections).
5. Rich in-plot legends/raw-vs-smooth toggles.
6. Better agent memory across tab/context switches.
7. Resume subcommand polish (`og resume` UX improvements).
8. Export/report command (`og export`) for quick artifacts.

## Explicit De-scope For Hack Day
1. Cloud sync and team collaboration.
2. Artifact/model registry workflows.
3. Unbounded autonomous code edits.
4. Full production-grade policy engine.

## Time Budget Guardrails (7h)
1. If behind schedule, cut all P1 first.
2. If still behind, keep `--auto` to one file target and one patch pattern.
3. Protect the golden loop at all costs: run -> alert -> explain -> action -> resume -> improve.

## Team Execution Rule
1. Every hour, ask: "Did we improve the golden loop?"
2. If the answer is no, stop and rescope immediately.
