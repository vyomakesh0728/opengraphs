import {
  type CliRenderer,
  BoxRenderable,
  TextRenderable,
  ScrollBoxRenderable,
} from "@opentui/core"

const SAMPLE_LOGS = [
  "-- tail of the running script --",
  "",
  "[2025-06-14 10:32:01] INFO  Starting training run (seed=42)",
  "[2025-06-14 10:32:01] INFO  Model: transformer-7b | Device: cuda:0",
  "[2025-06-14 10:32:02] INFO  Loading dataset: openwebtext-10k",
  "[2025-06-14 10:32:05] INFO  Dataset loaded: 10,240 samples",
  "[2025-06-14 10:32:05] INFO  Optimizer: AdamW (lr=3e-4, wd=0.1)",
  "[2025-06-14 10:32:06] INFO  Step 1/5000 | loss=11.234 | lr=3.0e-4 | tok/s=0",
  "[2025-06-14 10:32:08] INFO  Step 10/5000 | loss=9.871 | lr=3.0e-4 | tok/s=12,450",
  "[2025-06-14 10:32:12] INFO  Step 50/5000 | loss=7.342 | lr=3.0e-4 | tok/s=14,200",
  "[2025-06-14 10:32:18] INFO  Step 100/5000 | loss=5.891 | lr=2.9e-4 | tok/s=15,100",
  "[2025-06-14 10:32:30] INFO  Step 200/5000 | loss=4.567 | lr=2.8e-4 | tok/s=15,800",
  "[2025-06-14 10:32:45] INFO  Step 300/5000 | loss=3.921 | lr=2.6e-4 | tok/s=15,950",
  "[2025-06-14 10:33:01] INFO  Checkpoint saved: ckpt-300.pt",
  "[2025-06-14 10:33:05] INFO  Step 400/5000 | loss=3.456 | lr=2.4e-4 | tok/s=16,020",
  "[2025-06-14 10:33:20] INFO  Step 500/5000 | loss=3.102 | lr=2.2e-4 | tok/s=16,100",
  "[2025-06-14 10:33:22] INFO  val/loss=3.245 | val/accuracy=0.412",
  "[2025-06-14 10:33:35] INFO  Step 600/5000 | loss=2.891 | lr=2.0e-4 | tok/s=16,150",
  "[2025-06-14 10:33:50] INFO  Step 700/5000 | loss=2.734 | lr=1.8e-4 | tok/s=16,180",
  "[2025-06-14 10:34:05] INFO  Step 800/5000 | loss=2.612 | lr=1.6e-4 | tok/s=16,200",
  "[2025-06-14 10:34:20] INFO  Step 900/5000 | loss=2.501 | lr=1.4e-4 | tok/s=16,220",
  "[2025-06-14 10:34:35] INFO  Step 1000/5000 | loss=2.398 | lr=1.2e-4 | tok/s=16,240",
  "[2025-06-14 10:34:37] INFO  Checkpoint saved: ckpt-1000.pt",
  "[2025-06-14 10:34:38] INFO  val/loss=2.512 | val/accuracy=0.534",
]

export function createLogsTab(renderer: CliRenderer): BoxRenderable {
  const scroll = new ScrollBoxRenderable(renderer, {
    id: "logs-scroll",
    flexGrow: 1,
    scrollY: true,
    stickyScroll: true,
    stickyStart: "bottom",
    verticalScrollbarOptions: { visible: false },
  })
  scroll.verticalScrollBar.visible = false

  const logsContent = new BoxRenderable(renderer, {
    id: "logs-content",
    flexDirection: "column",
    width: "100%",
    paddingLeft: 1,
    paddingTop: 1,
  })

  for (let i = 0; i < SAMPLE_LOGS.length; i++) {
    const line = SAMPLE_LOGS[i]
    const isHeader = i === 0
    const text = new TextRenderable(renderer, {
      id: `log-line-${i}`,
      content: line,
      fg: isHeader ? "#6b7280" : "#d1d5db",
    })
    logsContent.add(text)
  }

  scroll.add(logsContent)
  return scroll
}
