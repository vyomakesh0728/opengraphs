import {
  type CliRenderer,
  BoxRenderable,
  TextRenderable,
  ASCIIFontRenderable,
} from "@opentui/core"

export function createChatTab(
  renderer: CliRenderer,
  version: string,
): BoxRenderable {
  const container = new BoxRenderable(renderer, {
    id: "chat-tab",
    flexGrow: 1,
    flexDirection: "column",
    justifyContent: "flex-end",
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

  const cwdText = new TextRenderable(renderer, {
    id: "chat-cwd",
    content: process.cwd(),
    fg: "#6b7280",
  })

  container.add(logo)
  container.add(versionText)
  container.add(cwdText)

  return container
}
