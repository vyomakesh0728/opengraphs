<p align="center">
    <img src="screenshots/Opengraphs%20logo%20with%20terminal%20icon.png" alt="OpenGraphs logo" width="900" />
</p>

# opengraphs (og)

Local-first, TUI-native experiment tracking for AI runs over SSH.

[![GitHub Stars](https://img.shields.io/github/stars/vyomakesh0728/opengraphs?style=flat-square)](https://github.com/vyomakesh0728/opengraphs/stargazers)
[![GitHub Downloads](https://img.shields.io/github/downloads/vyomakesh0728/opengraphs/total?style=flat-square)](https://github.com/vyomakesh0728/opengraphs/releases)
[![Rust](https://img.shields.io/badge/rust-first-orange?style=flat-square)](https://www.rust-lang.org/)

## Demo

<p align="center">
    <img src="screenshots/Kapture%202026-02-16%20at%2011.41.29.gif" alt="OpenGraphs demo GIF" width="900" />
</p>

<p align="center">
    <img src="screenshots/demo-agent-chat-screenshot.png" alt="OpenGraphs agent chat screenshot" width="900" />
</p>

- Video (MP4): [Kapture 2026-02-16 at 11.41.29](screenshots/Kapture%202026-02-16%20at%2011.41.29.mp4)

## Why this exists

Browser dashboards and port forwarding are painful on remote GPU boxes. `opengraphs` is built for terminal-native workflows:

- fast experiment views in SSH sessions
- lightweight local-first tracking
- simple run comparison and filtering
- Rust-first core (ratatui TUI), with isolated Python for DSPy/RLM agent chat
- agent auto-diagnoses training issues and can refactor code with `--auto`

## Current workspace

- `crates/ogtui`: Rust ratatui TUI (graphs, logs, agent chat tabs)
- `crates/ogd`: daemon/backend + Trackio Rust client integration point
- `python/agent-chat`: Python agent daemon (DSPy RLM + ReAct, alerts, code patching)

## Install (users)

One-line installer (always latest release):

```bash
curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash
```

Re-run the same command anytime to update to latest.

Install a pinned version:

```bash
curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash -s -- --version v0.1.0
```

Homebrew:

```bash
brew tap vyomakesh0728/opengraphs
brew install vyomakesh0728/opengraphs/opengraphs
brew upgrade vyomakesh0728/opengraphs/opengraphs
```

ZeroBrew (fast, experimental):

```bash
# requires zb >= 0.1.2
zb install vyomakesh0728/opengraphs/opengraphs
# zb has no dedicated upgrade command yet; rerun install to refresh
zb install vyomakesh0728/opengraphs/opengraphs
```

Note: `brew install opengraphs` (without tap prefix) works only after a Homebrew core merge.
Note: the `curl` installer resolves published GitHub Releases (`vX.Y.Z` tags). If no release is published yet, use the developer quickstart below.

## Quickstart (developer)

```bash
# Build + run TUI
cargo run -p ogtui -- --path runs/

# Install agent dependencies
uv pip install -e python/agent-chat/

# Start agent daemon (separate terminal)
export OPENAI_API_KEY="your-key"
python3 -m og_agent_chat.server --training-file train.py --codebase-root .

# Or with auto-refactor mode
python3 -m og_agent_chat.server --training-file train.py --codebase-root . --auto
```

## Agent chat

The TUI has a built-in **chat** tab that connects to the Python agent daemon via Unix socket.

- **DSPy ReAct** for tool-calling (reads metrics, logs, codebase)
- **DSPy RLM** for codebase exploration and code editing
- **`--auto` mode**: agent applies unified diffs to your training script and restarts training
- **Checkpoint/rollback**: every refactor is checkpointed; failures auto-rollback
## Live training metrics (single terminal)

```bash
cd /home/ubuntu/opengraphs
RUN_DIR="/tmp/og-live-$(date +%s)"
rm -f /tmp/opengraphs-ogd.sock
PATH="$PWD/.venv/bin:$HOME/.local/bin:$PATH" \
PYTHONPATH="$PWD/python/agent-chat" \
./target/debug/ogtui \
  --path "$RUN_DIR" \
  --refresh-ms 100 \
  --training-file /home/ubuntu/modded-nanogpt/train_gpt_demo50.py \
  --codebase-root /home/ubuntu/modded-nanogpt \
  --training-cmd "torchrun --standalone --nproc_per_node=8 train_gpt_demo50.py" \
  --start-training \
  --fresh-run \
  --auto
```

Optional env vars for TensorBoard logging:

```bash
TB_LOG_DIR=runs/       # default: runs/
TB_LOG_EVERY=10        # cheap metrics interval
TB_LOG_HEAVY_EVERY=50  # expensive metrics interval
```

If you only want the current run (and not older eval/event files), pass that run directory directly:

```bash
cargo run -p ogtui -- --path runs/<current-run-id>
```

## Stars graph

[![Star History Chart](https://api.star-history.com/svg?repos=vyomakesh0728/opengraphs&type=Date)](https://star-history.com/#vyomakesh0728/opengraphs&Date)

## Contributing

Contributions are welcome. Open an issue or PR with the problem you are solving, the proposed approach, and any tradeoffs.

---

Made with love 💚 from india
