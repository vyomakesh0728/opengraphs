import {
  type CliRenderer,
  BoxRenderable,
  TextRenderable,
  ASCIIFontRenderable,
  ScrollBoxRenderable,
} from "@opentui/core"
import type { ChatMessage } from "../lib/daemon"

const SENDER_COLORS: Record<string, string> = {
  user: "#9ca3af",
  agent: "#ffffff",
  system: "#6b7280",
}

export interface ChatTabResult {
  container: BoxRenderable
  setMessages: (messages: ChatMessage[]) => void
  setThinking: (thinking: boolean) => void
  setAutoMode: (enabled: boolean) => void
}

export function createChatTab(
  renderer: CliRenderer,
  version: string,
): ChatTabResult {
  const container = new BoxRenderable(renderer, {
    id: "chat-tab",
    flexGrow: 1,
    flexDirection: "column",
  })

  const chatScroll = new ScrollBoxRenderable(renderer, {
    id: "chat-scroll",
    flexGrow: 1,
    scrollY: true,
    stickyScroll: true,
    stickyStart: "bottom",
    verticalScrollbarOptions: { visible: false },
  })
  chatScroll.verticalScrollBar.visible = false

  const chatContent = new BoxRenderable(renderer, {
    id: "chat-content",
    flexDirection: "column",
    width: "100%",
    paddingLeft: 2,
    paddingTop: 0,
  })
  chatScroll.add(chatContent)
  container.add(chatScroll)

  let currentMessages: ChatMessage[] = []
  let isThinking = false
  let autoMode = false

  function createBrandingBlock() {
    const branding = new BoxRenderable(renderer, {
      id: "chat-branding",
      flexDirection: "column",
      paddingLeft: 2,
      paddingBottom: 1,
    })

    const logo = new ASCIIFontRenderable(renderer, {
      id: "chat-logo",
      text: "opengraphs",
      font: "block",
      color: "#2ecc71",
    })

    const versionText = new TextRenderable(renderer, {
      id: "chat-version",
      content: `(v${version})`,
      fg: "#9BA3AF",
    })

    branding.add(logo)
    branding.add(versionText)
    return branding
  }

  function renderMessages() {
    for (const child of chatContent.getChildren()) {
      chatContent.remove(child.id)
    }

    chatContent.add(createBrandingBlock())

    if (autoMode) {
      const auto = new TextRenderable(renderer, {
        id: "chat-auto-mode",
        content: "⚡ Auto mode: Agent will apply fixes",
        fg: "#2ecc71",
      })
      chatContent.add(auto)
    }

    if (currentMessages.length === 0) {
      const empty = new TextRenderable(renderer, {
        id: "chat-empty",
        content: "No messages yet.",
        fg: "#6b7280",
      })
      chatContent.add(empty)
    } else {
      currentMessages.forEach((message, idx) => {
        const sender = message.sender ?? "system"
        const prefix = sender === "user" ? "you" : sender
        const line = new TextRenderable(renderer, {
          id: `chat-line-${idx}`,
          content: `${prefix}: ${message.content}`,
          fg: SENDER_COLORS[sender] ?? "#d1d5db",
          wrapMode: "word",
        })
        chatContent.add(line)
      })
    }

    if (isThinking) {
      const thinking = new TextRenderable(renderer, {
        id: "chat-thinking",
        content: "agent is thinking...",
        fg: "#9BA3AF",
      })
      chatContent.add(thinking)
    }
  }

  function setMessages(messages: ChatMessage[]) {
    currentMessages = messages
    renderMessages()
  }

  function setThinking(thinking: boolean) {
    isThinking = thinking
    renderMessages()
  }

  function setAutoMode(enabled: boolean) {
    autoMode = enabled
    renderMessages()
  }

  renderMessages()

  return { container, setMessages, setThinking, setAutoMode }
}
