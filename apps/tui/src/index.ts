import { createCliRenderer, BoxRenderable, type CliRenderer } from "@opentui/core"
import { createHeader, type TabName } from "./ui/header"
import { createChatTab } from "./ui/chat"
import { createGraphsTab } from "./ui/graphs"
import { createLogsTab } from "./ui/logs"
import { createFooter } from "./ui/footer"
import { createHelpModal } from "./ui/help"
import {
  getChatHistory,
  getRunState,
  sendChatMessage,
  startDaemonPolling,
} from "./lib/daemon"
import type { ChatMessage } from "./lib/daemon"

// --- Read version ---
let version = "0.1.0"
try {
  const pkg = await Bun.file(
    new URL("../../package.json", import.meta.url),
  ).json()
  if (pkg.version) version = pkg.version
} catch {}

// --- Determine initial tab ---
const args = process.argv.slice(2)
const initialTab = determineInitialTab(args)

// --- Create renderer ---
const renderer = await createCliRenderer({
  useMouse: true,
  exitOnCtrlC: true,
})

// --- Root layout: column ---
const root = renderer.root
root.flexDirection = "column"

// --- Content region ---
const content = new BoxRenderable(renderer, {
  id: "content",
  flexGrow: 1,
  flexDirection: "column",
  overflow: "hidden",
})

// --- Header ---
const header = createHeader(renderer, switchTab, initialTab)
content.add(header.container)

// --- Tab bodies ---
const tabBody = new BoxRenderable(renderer, {
  id: "tab-body",
  flexGrow: 1,
  overflow: "hidden",
})

const chatTab = createChatTab(renderer, version)
const graphsResult = createGraphsTab(renderer)
const graphsTab = graphsResult.container
const logsResult = createLogsTab(renderer)
const logsTab = logsResult.container
const autoModeFromEnv = process.env.OG_AGENT_AUTO === "1"
chatTab.setAutoMode(autoModeFromEnv)

// Only show active tab
chatTab.container.visible = initialTab === "chat"
graphsTab.visible = initialTab === "graphs"
logsTab.visible = initialTab === "logs"

tabBody.add(chatTab.container)
tabBody.add(graphsTab)
tabBody.add(logsTab)
content.add(tabBody)

root.add(content)

// --- Footer ---
const footer = createFooter(renderer, handleInputSubmit)
root.add(footer.container)

// --- Help modal ---
const helpModal = createHelpModal(renderer)
root.add(helpModal.modal)

// --- Daemon polling ---
const stopPolling = startDaemonPolling((status) => {
  graphsResult.updateDaemonStatus(status)
})

let chatSignature = ""
let latestChatHistory: ChatMessage[] = []

async function refreshChatHistory() {
  try {
    const response = await getChatHistory()
    if (!response.ok) return
    const signature = response.chat_history
      .map((msg) => `${msg.timestamp}:${msg.sender}:${msg.content}`)
      .join("|")
    if (signature === chatSignature) return
    chatSignature = signature
    latestChatHistory = response.chat_history
    chatTab.setMessages(response.chat_history)
  } catch {
    // ignore polling errors
  }
}

async function refreshRunState() {
  try {
    const response = await getRunState(200, 1)
    if (!response.ok) return
    graphsResult.updateRunState(response.run_state)
    logsResult.setLogs(response.run_state.logs)
    chatTab.setAutoMode(response.run_state.auto_mode ?? autoModeFromEnv)
  } catch {
    // ignore polling errors
  }
}

const dataInterval = setInterval(() => {
  void refreshChatHistory()
  void refreshRunState()
}, 500)

void refreshChatHistory()
void refreshRunState()

// --- Tab switching ---
function switchTab(tab: TabName) {
  chatTab.container.visible = tab === "chat"
  graphsTab.visible = tab === "graphs"
  logsTab.visible = tab === "logs"
}

// --- Input submit handler ---
function handleInputSubmit(text: string) {
  const lower = text.toLowerCase().trim()

  // Quit commands
  if (
    lower === "quit" || lower === "exit" ||
    lower === "og quit" || lower === "!og quit" ||
    lower === "og exit" || lower === "!og exit"
  ) {
    cleanup()
    return
  }

  // Tab-switching commands
  if (
    lower.startsWith("og run ") ||
    lower.startsWith("!og run ") ||
    lower.startsWith("og --run ")
  ) {
    header.setActiveTab("graphs")
    return
  }

  if (
    lower.startsWith("og tail ") ||
    lower.startsWith("og -t ") ||
    lower.startsWith("og log ") ||
    lower.startsWith("og logs ")
  ) {
    header.setActiveTab("logs")
    return
  }

  void sendUserMessage(text)
}

async function sendUserMessage(text: string) {
  chatTab.setThinking(true)
  try {
    const response = await sendChatMessage(text)
    if (response.ok) {
      latestChatHistory = response.chat_history
      chatTab.setMessages(response.chat_history)
    } else {
      const fallback = latestChatHistory.length > 0 ? latestChatHistory : []
      chatTab.setMessages([
        ...fallback,
        {
          sender: "system",
          content: `Failed to send message: ${response.error}`,
          timestamp: Date.now() / 1000,
        },
      ])
    }
  } catch (err: any) {
    const fallback = latestChatHistory.length > 0 ? latestChatHistory : []
    chatTab.setMessages([
      ...fallback,
      {
        sender: "system",
        content: `Failed to send message: ${err?.message ?? "unknown error"}`,
        timestamp: Date.now() / 1000,
      },
    ])
  } finally {
    chatTab.setThinking(false)
  }
}

// --- Keyboard shortcuts ---
const keyInput = renderer.keyInput as any
keyInput.on("keypress", (key: any) => {
  const inputFocused = footer.isInputFocused()

  // Help modal toggle with ?
  if ((key.name === "?" || key.sequence === "?" || (key.name === "/" && key.shift)) && !inputFocused) {
    helpModal.toggle()
    key.preventDefault()
    return
  }

  // Close help modal with Esc
  if (key.name === "escape" && helpModal.isVisible()) {
    helpModal.hide()
    key.preventDefault()
    return
  }

  // If help modal is visible, block other shortcuts
  if (helpModal.isVisible()) return

  // Shift+Tab cycles tabs
  if (key.name === "tab" && key.shift) {
    header.cycleTab()
    key.preventDefault()
    return
  }

  // / focuses input when not focused
  if (key.name === "/" && !inputFocused) {
    footer.focusInput()
    key.preventDefault()
    return
  }

  // Esc blurs input when focused
  if (key.name === "escape" && inputFocused) {
    footer.blurInput()
    key.preventDefault()
    return
  }

  // q quits when input is not focused
  if (key.name === "q" && !inputFocused) {
    cleanup()
    return
  }

  // Paste event focuses input
  // (handled via paste event below)
})

// Paste focuses input if not focused
keyInput.on("paste", (_event: any) => {
  if (!footer.isInputFocused()) {
    footer.focusInput()
  }
})

// --- Responsive layout ---
;(renderer as any).on("resize", (width: number, _height: number) => {
  graphsResult.updateLayout(width)
})

function determineInitialTab(argv: string[]): TabName {
  const hasLogsTarget = argv.some((arg) =>
    arg === "--logs" || arg === "--log" || arg === "--tail" || arg.startsWith("--logs="),
  )
  if (hasLogsTarget) return "logs"

  const hasRunTarget = argv.some((arg) =>
    arg === "--run" || arg === "-r" || arg.startsWith("--run="),
  )
  if (hasRunTarget) return "graphs"

  return "chat"
}

// Initial layout update
graphsResult.updateLayout(renderer.width)

// --- Cleanup ---
function cleanup() {
  stopPolling()
  clearInterval(dataInterval)
  renderer.destroy()
  process.exit(0)
}

// --- Start ---
renderer.start()
