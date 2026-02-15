import {
  type CliRenderer,
  BoxRenderable,
  TextRenderable,
} from "@opentui/core"

export type TabName = "chat" | "graphs" | "logs"

const TAB_ACTIVE_COLOR = "#2ecc71"
const TAB_INACTIVE_BORDER = "#6b7280"
const TAB_INACTIVE_TEXT = "#d1d5db"

export interface HeaderResult {
  container: BoxRenderable
  setActiveTab: (tab: TabName) => void
  getActiveTab: () => TabName
  cycleTab: () => void
}

export function createHeader(
  renderer: CliRenderer,
  onTabChange: (tab: TabName) => void,
  initialTab: TabName = "chat",
): HeaderResult {
  const isAppleTerminal = process.env.TERM_PROGRAM === "Apple_Terminal"

  const container = new BoxRenderable(renderer, {
    id: "header",
    flexDirection: "row",
    height: isAppleTerminal ? 4 : 3,
    paddingTop: isAppleTerminal ? 1 : 0,
  })

  const tabs: TabName[] = ["chat", "graphs", "logs"]
  let activeTab: TabName = initialTab
  const tabBoxes: Map<TabName, BoxRenderable> = new Map()
  const tabLabels: Map<TabName, TextRenderable> = new Map()

  for (const tab of tabs) {
    const isActive = tab === activeTab
    const box = new BoxRenderable(renderer, {
      id: `tab-${tab}`,
      width: 8,
      height: 3,
      border: true,
      borderColor: isActive ? TAB_ACTIVE_COLOR : TAB_INACTIVE_BORDER,
      justifyContent: "center",
      alignItems: "center",
      onMouseDown() {
        setActiveTab(tab)
      },
    })

    const label = new TextRenderable(renderer, {
      id: `tab-label-${tab}`,
      content: tab,
      fg: isActive ? TAB_ACTIVE_COLOR : TAB_INACTIVE_TEXT,
    })

    box.add(label)
    container.add(box)
    tabBoxes.set(tab, box)
    tabLabels.set(tab, label)
  }

  const stepProgress = new BoxRenderable(renderer, {
    id: "step-progress",
    flexGrow: 1,
    height: 3,
    border: true,
    borderColor: TAB_INACTIVE_BORDER,
    title: " step progress ",
    paddingLeft: 1,
    paddingRight: 1,
    alignItems: "center",
  })

  container.add(stepProgress)

  function setActiveTab(tab: TabName) {
    if (activeTab === tab) return
    activeTab = tab

    for (const t of tabs) {
      const isActive = t === tab
      const box = tabBoxes.get(t)!
      const label = tabLabels.get(t)!
      box.borderColor = isActive ? TAB_ACTIVE_COLOR : TAB_INACTIVE_BORDER
      label.fg = isActive ? TAB_ACTIVE_COLOR : TAB_INACTIVE_TEXT
    }

    onTabChange(tab)
  }

  function cycleTab() {
    const idx = tabs.indexOf(activeTab)
    const next = tabs[(idx + 1) % tabs.length]
    setActiveTab(next)
  }

  return {
    container,
    setActiveTab,
    getActiveTab: () => activeTab,
    cycleTab,
  }
}
