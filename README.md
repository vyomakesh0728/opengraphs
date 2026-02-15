<p align="center">
    <img src="screenshots/Opengraphs%20logo%20with%20terminal%20icon.png" alt="OpenGraphs logo" width="900" />
</p>

# opengraphs (og)

Local-first, TUI-native experiment tracking for AI runs over SSH.

[![GitHub Stars](https://img.shields.io/github/stars/vyomakesh0728/opengraphs?style=flat-square)](https://github.com/vyomakesh0728/opengraphs/stargazers)
[![GitHub Downloads](https://img.shields.io/github/downloads/vyomakesh0728/opengraphs/total?style=flat-square)](https://github.com/vyomakesh0728/opengraphs/releases)
[![Rust](https://img.shields.io/badge/rust-first-orange?style=flat-square)](https://www.rust-lang.org/)
[![OpenTUI](https://img.shields.io/badge/tui-opentui-black?style=flat-square)](https://github.com/anomalyco/opentui)

## Why this exists

Browser dashboards and port forwarding are painful on remote GPU boxes. `opengraphs` is built for terminal-native workflows:

- fast experiment views in SSH sessions
- lightweight local-first tracking
- simple run comparison and filtering
- Rust-first core, with isolated Python only for agent chat workflows

## Current workspace

- `crates/og`: CLI entrypoint
- `crates/ogd`: daemon/backend + Trackio Rust client integration point
- `apps/tui`: Bun + OpenTUI app
- `python/agent-chat`: isolated Python env for agent features

## Quickstart (developer)

```bash
# from repo root
cargo check
cargo run -p og
bun run tui
```

Python agent-chat environment:

```bash
cd python/agent-chat
uv sync
```

## Stars graph

[![Star History Chart](https://api.star-history.com/svg?repos=vyomakesh0728/opengraphs&type=Date)](https://star-history.com/#vyomakesh0728/opengraphs&Date)

## Contributing

Contributions are welcome. Open an issue or PR with the problem you are solving, the proposed approach, and any tradeoffs.

---

Made with love 💚 from india
