import {
  type CliRenderer,
  BoxRenderable,
  TextRenderable,
  ScrollBoxRenderable,
} from "@opentui/core"
import { type DaemonStatus, type RunStateSnapshot } from "../lib/daemon"

const METRIC_LABELS = [
  "train/loss", "val/loss", "reward", "grad_norm", "throughput", "lr",
  "policy_loss", "value_loss", "episode_len", "kl", "entropy", "accuracy",
  "tokens/sec", "val/accuracy", "advantage", "clip_frac",
]

const SYS_LABELS = ["CPU %", "RAM %", "GPU %", "VRAM %", "Disk IO", "Net IO"]

const BORDER_COLOR = "#6b7280"
const TEXT_DIM = "#9BA3AF"

export interface GraphsTabResult {
  container: BoxRenderable
  updateDaemonStatus: (status: DaemonStatus) => void
  updateRunState: (runState: RunStateSnapshot) => void
  updateLayout: (width: number) => void
}

export function createGraphsTab(renderer: CliRenderer): GraphsTabResult {
  const container = new BoxRenderable(renderer, {
    id: "graphs-tab",
    flexGrow: 1,
    flexDirection: "row",
  })

  // --- Left: metrics window ---
  const metricsScroll = new ScrollBoxRenderable(renderer, {
    id: "metrics-scroll",
    flexGrow: 1,
    border: true,
    borderColor: BORDER_COLOR,
    title: " metrics ",
    scrollY: true,
    verticalScrollbarOptions: { visible: false },
  })
  metricsScroll.verticalScrollBar.visible = false

  const metricsGrid = new BoxRenderable(renderer, {
    id: "metrics-grid",
    flexDirection: "row",
    flexWrap: "wrap",
    width: "100%",
  })

  const metricValues = new Map<string, TextRenderable>()

  for (const label of METRIC_LABELS) {
    const card = new BoxRenderable(renderer, {
      id: `metric-${label}`,
      width: "25%",
      height: 15,
      border: true,
      borderColor: BORDER_COLOR,
      title: ` ${label} `,
      padding: 1,
    })
    const placeholder = new TextRenderable(renderer, {
      id: `metric-val-${label}`,
      content: "--",
      fg: TEXT_DIM,
    })
    card.add(placeholder)
    metricsGrid.add(card)
    metricValues.set(label, placeholder)
  }

  metricsScroll.add(metricsGrid)
  container.add(metricsScroll)

  // --- Right: side column (stats + sys) ---
  const sideColumn = new BoxRenderable(renderer, {
    id: "side-column",
    width: "26%",
    minWidth: 22,
    maxWidth: "30%",
    flexDirection: "column",
  })

  // Stats window
  const statsScroll = new ScrollBoxRenderable(renderer, {
    id: "stats-scroll",
    height: 14,
    minHeight: 12,
    border: true,
    borderColor: BORDER_COLOR,
    title: " stats ",
    scrollY: true,
    verticalScrollbarOptions: { visible: false },
  })
  statsScroll.verticalScrollBar.visible = false

  const statsContent = new BoxRenderable(renderer, {
    id: "stats-content",
    flexDirection: "column",
    padding: 1,
    width: "100%",
  })

  const daemonStatusText = new TextRenderable(renderer, {
    id: "daemon-status",
    content: "ogd: checking...",
    fg: TEXT_DIM,
  })

  const statsBlock = new TextRenderable(renderer, {
    id: "stats-block",
    content: [
      "",
      "run:    --",
      "config: default",
      "env:    local",
      "seed:   42",
      "device: --",
    ].join("\n"),
    fg: TEXT_DIM,
    wrapMode: "word",
  })

  statsContent.add(daemonStatusText)
  statsContent.add(statsBlock)
  statsScroll.add(statsContent)
  sideColumn.add(statsScroll)

  // Sys window
  const sysScroll = new ScrollBoxRenderable(renderer, {
    id: "sys-scroll",
    flexGrow: 1,
    border: true,
    borderColor: BORDER_COLOR,
    title: " sys ",
    scrollY: true,
    verticalScrollbarOptions: { visible: false },
  })
  sysScroll.verticalScrollBar.visible = false

  const sysGrid = new BoxRenderable(renderer, {
    id: "sys-grid",
    flexDirection: "row",
    flexWrap: "wrap",
    width: "100%",
  })

  for (const label of SYS_LABELS) {
    const card = new BoxRenderable(renderer, {
      id: `sys-${label}`,
      width: "50%",
      height: 8,
      border: true,
      borderColor: BORDER_COLOR,
      title: ` ${label} `,
      padding: 1,
    })
    const val = new TextRenderable(renderer, {
      id: `sys-val-${label}`,
      content: "--",
      fg: TEXT_DIM,
    })
    card.add(val)
    sysGrid.add(card)
  }

  sysScroll.add(sysGrid)
  sideColumn.add(sysScroll)
  container.add(sideColumn)

  function updateDaemonStatus(status: DaemonStatus) {
    try {
      if (status.ok) {
        daemonStatusText.content = "ogd: ok"
        daemonStatusText.fg = "#2ecc71"
      } else {
        daemonStatusText.content = `ogd: offline (${status.error}) Start with "cargo run -p ogd" or set OGD_SOCKET.`
        daemonStatusText.fg = "#ef4444"
      }
    } catch {
      // Renderable may be destroyed during shutdown
    }
  }

  function updateRunState(runState: RunStateSnapshot) {
    for (const label of METRIC_LABELS) {
      const target = metricValues.get(label)
      if (!target) continue
      const values = runState.metrics[label]
      if (values && values.length > 0) {
        const latest = values[values.length - 1]
        target.content = formatMetric(latest)
        target.fg = "#d1d5db"
      } else {
        target.content = "--"
        target.fg = TEXT_DIM
      }
    }

    const lastAlert = runState.alerts[runState.alerts.length - 1]
    statsBlock.content = [
      "",
      "run:    --",
      "config: default",
      "env:    local",
      "seed:   42",
      "device: --",
      `step:   ${runState.current_step ?? "--"}`,
      lastAlert ? `alert:  ${lastAlert.message}` : "alert:  --",
    ].join("\n")
  }

  function updateLayout(width: number) {
    const metricChildren = metricsGrid.getChildren() as BoxRenderable[]
    let cols: string
    if (width < 40) cols = "100%"
    else if (width < 60) cols = "50%"
    else if (width < 80) cols = "33%"
    else cols = "25%"

    for (const card of metricChildren) {
      card.width = cols as `${number}%`
    }

    const sideWidth = sideColumn.width as number
    const sysChildren = sysGrid.getChildren() as BoxRenderable[]
    const sysCols = sideWidth < 26 ? "100%" : "50%"
    for (const card of sysChildren) {
      card.width = sysCols as `${number}%`
    }
  }

  return { container, updateDaemonStatus, updateRunState, updateLayout }
}

function formatMetric(value: number): string {
  if (!Number.isFinite(value)) return "--"
  const abs = Math.abs(value)
  if (abs >= 1000) return value.toFixed(0)
  if (abs >= 1) return value.toFixed(4)
  return value.toExponential(2)
}
