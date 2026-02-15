# OpenGraphs Exact UI/UX Recreation Prompt

Use this prompt with a coding agent to ecreate the current OpenGraphs TUI exactly.

---

You are a senior TypeScript + OpenTUI engineer.

Rebuild the OpenGraphs TUI UI/UX exactly as specified below.
Do not add new features, new panels, or visual redesigns.
Match behavior, shortcuts, layout, and interactions precisely.

## Stack and runtime
1. Use Bun + TypeScript + `@opentui/core`.
2. Full-screen terminal app.
3. Use mouse support and keyboard handling.
4. Keep the app responsive on split-screen terminal sizes.

## High-level structure
1. Root app uses column layout with:
   - top content region
   - footer region
2. Content region has:
   - header row
   - tab body
3. Footer has:
   - input frame
   - mention panel
   - `? for shortcuts` hint
4. Include help modal overlay.

## Header (tabs + step progress)
1. Three clickable tabs:
   - `chat`
   - `graphs`
   - `logs`
2. Tab buttons:
   - equal width (`8`)
   - fixed height (`3`)
   - bordered
   - center-aligned labels
3. Active tab style:
   - border/text color `#2ecc71`
4. Inactive tab style:
   - border `#6b7280`
   - text `#d1d5db`
5. To the right of tabs, a bordered box titled:
   - ` step progress `
6. Step progress box fills remaining header width.
7. On Apple Terminal, keep one-line top offset to avoid clipping.

## Tab behavior
1. Default tab:
   - `chat` by default
   - `graphs` if a run target was passed
2. Tabs switch by:
   - mouse click on tab box or tab label
   - `Shift+Tab` cycle through tabs
3. Commands submitted in input can switch tabs:
   - `og run ...` or `!og run ...` or `og --run ...` -> `graphs`
   - `og tail ...`, `og -t ...`, `og log ...`, `og logs ...` -> `logs`

## Chat tab
1. Show ASCII logo text `opengraphs` (block font, green).
2. Under logo, show:
   - `(v<package version>)`
   - current working directory
3. Keep this content anchored in lower-left visual area of the chat body.

## Graphs tab layout
1. Main row split:
   - left `metrics` window
   - right side column with `stats` and `sys`
2. Side column sizing:
   - width about `26%`
   - min width `22`
   - max width `30%`

### Metrics window
1. Border + title ` metrics `.
2. Scroll functionality enabled, but scrollbars hidden.
3. Metric cards must be uniform height (`15`) and grid-aligned.
4. Responsive columns:
   - default `4`
   - width < 80 -> `3`
   - width < 60 -> `2`
   - width < 40 -> `1`
5. Keep metric cards titled in box-title style.
6. Include these metric labels in order:
   - `train/loss`, `val/loss`, `reward`, `grad_norm`, `throughput`, `lr`
   - `policy_loss`, `value_loss`, `episode_len`, `kl`, `entropy`, `accuracy`, `tokens/sec`, `val/accuracy`, `advantage`, `clip_frac`

### Stats window
1. Border + title ` stats `.
2. Fixed-ish top panel height (`14`), min height (`12`).
3. Scroll enabled, scrollbars hidden.
4. Show daemon status line near top (health text).
5. Show static run/config/environment text block.

### Sys window
1. Border + title ` sys `.
2. Scroll enabled, scrollbars hidden.
3. Grid cards with equal card height (`8`).
4. Two-column default layout, one-column on narrow width:
   - width < 26 -> `1`
   - otherwise `2`
5. Labels:
   - `CPU %`, `RAM %`, `GPU %`, `VRAM %`, `Disk IO`, `Net IO`
6. Keep CPU/RAM and GPU/VRAM spacing visually consistent.

## Logs tab
1. Full-width scrollable log panel.
2. Scroll functionality exists, but scrollbars hidden.
3. Include starter line:
   - `-- tail of the running script --`
4. Follow with realistic timestamped training log lines.

## Footer input area
1. Input prompt line:
   - prompt marker `>`
   - text area beside prompt
2. Input container border:
   - top + left + right border on the input box
   - custom bottom border line with right-aligned `opengraphs` text
3. Keep `? for shortcuts` below the input frame (not inside the input box).
4. Input supports multiline expansion:
   - min 1 line
   - max 4 lines
   - box height tracks input lines

## Mention UX (`@`)
1. Typing `@` opens mention suggestions.
2. Source mention candidates from:
   - `git ls-files`
   - plus untracked files (`git ls-files --others --exclude-standard`)
   - fallback to `rg --files`
3. Mention suggestions:
   - case-insensitive prefix + contains match
   - limit to 50
4. Mention controls:
   - `Up/Down` navigate
   - `Enter`/`Tab` insert selected mention
   - `Esc` closes mention panel
5. Insert mention into input with trailing space when needed.

## Help modal
1. Hidden by default.
2. Opens with `?` only when input is not focused.
3. Closes with `?` or `Esc`.
4. Style:
   - border true
   - title ` shortcuts `
   - ~70% width, ~60% height
   - centered overlay
   - dark background
5. Include shortcut help content matching the controls below.

## Keyboard shortcuts and controls (must match)
1. `Shift+Tab` cycles tabs.
2. `/` focuses input when not focused.
3. `Esc` blurs input when focused.
4. `q` quits app only when input is not focused.
5. Input submit:
   - `Enter` / `Return`
6. Newline in input:
   - `Shift+Enter` (support terminal variants)
7. Quit command by text submit:
   - `quit` / `exit`
   - and `og quit` / `!og quit` / `og exit` / `!og exit`
8. Clipboard/edit shortcuts:
   - `Cmd+C` copy selection or entire input
   - `Cmd+V` / `Ctrl+V` / `^V` paste
   - `Ctrl+U` or `Cmd+Backspace` delete to line start
   - `Ctrl+W` or `Opt+Backspace` delete previous word
   - `Ctrl+A` or `Cmd+Left` move to line start
   - `Ctrl+E` or `Cmd+Right` move to line end
   - `Ctrl+K` delete to line end
9. Paste event should focus input if not focused.

## Daemon status behavior
1. Read `OGD_SOCKET` env var.
2. Default socket path:
   - `${TMPDIR|TEMP|TMP|/tmp}/opengraphs-ogd.sock`
3. Poll `/health` over Unix Domain Socket every second.
4. Status line examples:
   - healthy: `ogd: ok`
   - error: `ogd: offline (...) Start with "cargo run -p ogd" or set OGD_SOCKET.`

## Visual consistency requirements
1. Keep scroll behavior implicit (functional, but no visible scrollbars).
2. Keep tab/footer alignment stable across `chat`, `graphs`, `logs`.
3. Do not collapse the header/footer spacing when switching tabs.
4. Preserve clickable tabs and terminal-first interaction model.

## Acceptance checklist
1. Tabs are clickable and keyboard-switchable with `Shift+Tab`.
2. Help modal opens/closes exactly as specified.
3. `Shift+Enter` inserts newline reliably.
4. `? for shortcuts` stays below input area in all tabs.
5. Graph windows match `metrics` + `stats` + `sys` composition.
6. Hidden-scroll behavior exists in metrics/stats/sys/logs.
7. `og run ...` switches to graphs and `og tail ...` switches to logs.
8. UDS health status updates every second.

Only return production-ready TypeScript implementation (no extra commentary).
