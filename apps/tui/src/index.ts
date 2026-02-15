import { createCliRenderer, TextRenderable } from "@opentui/core"

const renderer = await createCliRenderer()

const title = new TextRenderable(renderer, {
  id: "og-title",
  content: "OpenGraphs TUI (OpenTUI) - press q to quit",
  fg: "#7CE38B",
  position: "absolute",
  left: 2,
  top: 1,
})

const subtitle = new TextRenderable(renderer, {
  id: "og-subtitle",
  content: "Local-first experiment tracking over SSH",
  fg: "#9BA3AF",
  position: "absolute",
  left: 2,
  top: 3,
})

renderer.root.add(title)
renderer.root.add(subtitle)

renderer.keyInput.on("keypress", (key) => {
  if (key.name === "q" || (key.ctrl && key.name === "c")) {
    renderer.destroy()
    process.exit(0)
  }
})

renderer.start()
