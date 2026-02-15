import {
  type CliRenderer,
  BoxRenderable,
  TextRenderable,
} from "@opentui/core"

const SHORTCUTS = [
  ["Shift+Tab", "Cycle tabs"],
  ["/", "Focus input"],
  ["Esc", "Blur input / close modal"],
  ["q", "Quit (when input not focused)"],
  ["Enter", "Submit input"],
  ["Ctrl/Cmd+Enter", "New line in input"],
  ["?", "Toggle shortcuts (when input not focused)"],
  ["", ""],
  ["Cmd+C", "Copy selection or input"],
  ["Cmd+V / Ctrl+V", "Paste"],
  ["Ctrl+U / Cmd+Backspace", "Delete to line start"],
  ["Ctrl+W / Opt+Backspace", "Delete previous word"],
  ["Ctrl+A / Cmd+Left", "Move to line start"],
  ["Ctrl+E / Cmd+Right", "Move to line end"],
  ["Ctrl+K", "Delete to line end"],
  ["", ""],
  ["@filename", "Mention / attach file"],
  ["og run ...", "Switch to graphs tab"],
  ["og tail ...", "Switch to logs tab"],
  ["quit / exit", "Quit app"],
]

export interface HelpModalResult {
  modal: BoxRenderable
  toggle: () => void
  isVisible: () => boolean
  hide: () => void
}

export function createHelpModal(renderer: CliRenderer): HelpModalResult {
  const modal = new BoxRenderable(renderer, {
    id: "help-modal",
    position: "absolute",
    width: "70%",
    height: "60%",
    left: "15%",
    top: "20%",
    zIndex: 100,
    border: true,
    borderColor: "#6b7280",
    title: " shortcuts ",
    backgroundColor: "#0d1117",
    flexDirection: "column",
    padding: 2,
    visible: false,
    overflow: "hidden",
  })

  for (const [key, desc] of SHORTCUTS) {
    if (!key && !desc) {
      const spacer = new BoxRenderable(renderer, {
        id: `help-spacer-${Math.random().toString(36).slice(2)}`,
        height: 1,
      })
      modal.add(spacer)
      continue
    }

    const row = new BoxRenderable(renderer, {
      id: `help-row-${key.replace(/[^a-zA-Z0-9]/g, "_")}`,
      flexDirection: "row",
      height: 1,
    })

    const keyText = new TextRenderable(renderer, {
      id: `help-key-${key.replace(/[^a-zA-Z0-9]/g, "_")}`,
      content: key.padEnd(24),
      fg: "#2ecc71",
      width: 24,
    })

    const descText = new TextRenderable(renderer, {
      id: `help-desc-${key.replace(/[^a-zA-Z0-9]/g, "_")}`,
      content: desc,
      fg: "#d1d5db",
      flexGrow: 1,
    })

    row.add(keyText)
    row.add(descText)
    modal.add(row)
  }

  let visible = false

  function toggle() {
    visible = !visible
    modal.visible = visible
  }

  function hide() {
    visible = false
    modal.visible = false
  }

  return {
    modal,
    toggle,
    isVisible: () => visible,
    hide,
  }
}
