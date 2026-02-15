import {
  type CliRenderer,
  BoxRenderable,
  TextRenderable,
  TextareaRenderable,
  ScrollBoxRenderable,
  RGBA,
} from "@opentui/core"
import { getMentionCandidates } from "../lib/mentions"

const BORDER_COLOR = "#6b7280"
const GREEN = "#2ecc71"
const TEXT_DIM = "#9BA3AF"
const borderRGBA = RGBA.fromHex(BORDER_COLOR)
const transparentRGBA = RGBA.fromValues(0, 0, 0, 0)

export interface FooterResult {
  container: BoxRenderable
  textarea: TextareaRenderable
  focusInput: () => void
  blurInput: () => void
  isInputFocused: () => boolean
}

export function createFooter(
  renderer: CliRenderer,
  onSubmit: (text: string) => void,
): FooterResult {
  const container = new BoxRenderable(renderer, {
    id: "footer",
    flexDirection: "column",
  })

  // --- Mention panel (hidden by default) ---
  const mentionScroll = new ScrollBoxRenderable(renderer, {
    id: "mention-scroll",
    height: 0,
    visible: false,
    border: true,
    borderColor: BORDER_COLOR,
    maxHeight: 12,
    scrollY: true,
    verticalScrollbarOptions: { visible: false },
  })
  mentionScroll.verticalScrollBar.visible = false

  const mentionList = new BoxRenderable(renderer, {
    id: "mention-list",
    flexDirection: "column",
    width: "100%",
  })
  mentionScroll.add(mentionList)
  container.add(mentionScroll)

  // --- Input frame (border on all 4 sides, we overlay "opengraphs" on bottom) ---
  const inputFrame = new BoxRenderable(renderer, {
    id: "input-frame",
    border: true,
    borderColor: BORDER_COLOR,
    flexDirection: "row",
    minHeight: 3,
    maxHeight: 6,
    paddingLeft: 1,
    alignItems: "flex-start",
    renderAfter(buffer) {
      // Overlay "opengraphs" right-aligned on the bottom border
      const label = "opengraphs"
      const x = this.x + this.width - label.length - 2
      const y = this.y + this.height - 1
      buffer.drawText(label, x, y, borderRGBA, transparentRGBA)
    },
  })

  const prompt = new TextRenderable(renderer, {
    id: "input-prompt",
    content: ">",
    fg: GREEN,
    width: 2,
  })

  const textarea = new TextareaRenderable(renderer, {
    id: "input-textarea",
    flexGrow: 1,
    minHeight: 1,
    maxHeight: 4,
    wrapMode: "word",
    backgroundColor: "transparent",
    textColor: "#d1d5db",
    focusedBackgroundColor: "transparent",
    focusedTextColor: "#ffffff",
    placeholder: "Type a message...",
    placeholderColor: "#6b7280",
    onSubmit: () => {
      const text = textarea.plainText.trim()
      if (!text) return
      closeMentions()
      onSubmit(text)
      textarea.clear()
    },
  })

  inputFrame.add(prompt)
  inputFrame.add(textarea)
  container.add(inputFrame)

  // --- Hint ---
  const hint = new TextRenderable(renderer, {
    id: "shortcuts-hint",
    content: "? for shortcuts",
    fg: TEXT_DIM,
    height: 1,
    alignSelf: "flex-end",
    paddingRight: 1,
  })
  container.add(hint)

  // --- Mention state ---
  let mentionActive = false
  let mentionItems: string[] = []
  let mentionIndex = 0
  let mentionQuery = ""
  let mentionStart = -1

  function closeMentions() {
    mentionActive = false
    mentionScroll.visible = false
    mentionScroll.height = 0
    mentionItems = []
    mentionIndex = 0
    mentionQuery = ""
    mentionStart = -1
  }

  function renderMentionList() {
    // Clear existing
    for (const child of mentionList.getChildren()) {
      mentionList.remove(child.id)
    }

    if (mentionItems.length === 0) {
      closeMentions()
      return
    }

    const visibleCount = Math.min(mentionItems.length, 10)
    mentionScroll.visible = true
    mentionScroll.height = visibleCount + 2 // +2 for border

    for (let i = 0; i < mentionItems.length; i++) {
      const isSelected = i === mentionIndex
      const item = new TextRenderable(renderer, {
        id: `mention-item-${i}`,
        content: `${isSelected ? ">" : " "} ${mentionItems[i]}`,
        fg: isSelected ? GREEN : "#d1d5db",
        bg: isSelected ? "#1a1a2e" : "transparent",
        width: "100%",
      })
      mentionList.add(item)
    }
  }

  async function updateMentions() {
    const text = textarea.plainText
    const offset = textarea.cursorOffset

    // Find @ before cursor
    const before = text.slice(0, offset)
    const atIdx = before.lastIndexOf("@")

    if (atIdx === -1 || (atIdx > 0 && before[atIdx - 1] !== " " && before[atIdx - 1] !== "\n")) {
      if (mentionActive) closeMentions()
      return
    }

    mentionStart = atIdx
    mentionQuery = before.slice(atIdx + 1)

    // Don't show if there's a space in the query (mention was completed)
    if (mentionQuery.includes(" ")) {
      if (mentionActive) closeMentions()
      return
    }

    mentionActive = true
    mentionIndex = 0
    mentionItems = await getMentionCandidates(mentionQuery)
    renderMentionList()
  }

  function insertMention(item: string) {
    const text = textarea.plainText
    const before = text.slice(0, mentionStart)
    const after = text.slice(textarea.cursorOffset)
    const newText = `${before}@${item} ${after}`
    textarea.replaceText(newText)
    textarea.editBuffer.setCursorByOffset(mentionStart + item.length + 2)
    closeMentions()
  }

  // Track content changes for mention detection
  textarea.onContentChange = () => {
    updateMentions()
  }

  // Override keypress for mention navigation
  const origHandleKeyPress = textarea.handleKeyPress.bind(textarea)
  textarea.handleKeyPress = (key) => {
    if (mentionActive) {
      if (key.name === "up") {
        mentionIndex = Math.max(0, mentionIndex - 1)
        renderMentionList()
        return true
      }
      if (key.name === "down") {
        mentionIndex = Math.min(mentionItems.length - 1, mentionIndex + 1)
        renderMentionList()
        return true
      }
      if (key.name === "return" || key.name === "tab") {
        if (mentionItems.length > 0) {
          insertMention(mentionItems[mentionIndex])
          return true
        }
      }
      if (key.name === "escape") {
        closeMentions()
        return true
      }
    }
    return origHandleKeyPress(key)
  }

  return {
    container,
    textarea,
    focusInput: () => textarea.focus(),
    blurInput: () => textarea.blur(),
    isInputFocused: () => textarea.focused,
  }
}
