import { connect, type Socket } from "net"

const OGD_SOCKET =
  process.env.OGD_SOCKET ??
  `${process.env.TMPDIR ?? process.env.TEMP ?? process.env.TMP ?? "/tmp"}/opengraphs-ogd.sock`
const REQUEST_TIMEOUT_MS = 2000
const CHAT_TIMEOUT_MS = 30000

export type DaemonStatus = { ok: true } | { ok: false; error: string }
export type ChatSender = "user" | "agent" | "system"
export type ChatMessage = {
  sender: ChatSender
  content: string
  timestamp: number
}

export type AlertSnapshot = {
  metric: string
  threshold: number
  current: number
  message: string
  timestamp: number
}

export type RunStateSnapshot = {
  metrics: Record<string, number[]>
  logs: string[]
  alerts: AlertSnapshot[]
  current_step: number
  auto_mode?: boolean
}

type DaemonResponse<T> = ({ ok: true } & T) | { ok: false; error: string }

function sendRequest(payload: Record<string, unknown>, timeoutMs = REQUEST_TIMEOUT_MS): Promise<any> {
  return new Promise((resolve, reject) => {
    let socket: Socket | null = null
    let finished = false
    let buffer = ""

    const finish = (fn: () => void) => {
      if (finished) return
      finished = true
      fn()
    }

    const timeout = setTimeout(() => {
      finish(() => {
        socket?.destroy()
        reject(new Error("timeout"))
      })
    }, timeoutMs)

    try {
      socket = connect(OGD_SOCKET, () => {
        socket!.write(`${JSON.stringify(payload)}\n`)
      })

      socket.on("data", (chunk: Buffer) => {
        buffer += chunk.toString()
        const newlineIndex = buffer.indexOf("\n")
        if (newlineIndex >= 0) {
          const line = buffer.slice(0, newlineIndex)
          finish(() => {
            clearTimeout(timeout)
            socket?.end()
            try {
              resolve(JSON.parse(line))
            } catch {
              reject(new Error("invalid response"))
            }
          })
        }
      })

      socket.on("error", (err: Error) => {
        finish(() => {
          clearTimeout(timeout)
          reject(err)
        })
      })

      socket.on("end", () => {
        if (!finished && buffer.trim().length > 0) {
          finish(() => {
            clearTimeout(timeout)
            try {
              resolve(JSON.parse(buffer))
            } catch {
              reject(new Error("invalid response"))
            }
          })
          return
        }
        finish(() => {
          clearTimeout(timeout)
          reject(new Error("empty response"))
        })
      })
    } catch (e: any) {
      finish(() => {
        clearTimeout(timeout)
        reject(e)
      })
    }
  })
}

async function checkHealth(): Promise<DaemonStatus> {
  try {
    const response = await sendRequest({ type: "ping" })
    if (response?.ok) return { ok: true }
    return { ok: false, error: response?.error ?? "bad response" }
  } catch (err: any) {
    return { ok: false, error: err?.message ?? "error" }
  }
}

export async function getChatHistory(): Promise<DaemonResponse<{ chat_history: ChatMessage[] }>> {
  return sendRequest({ type: "get_chat_history" })
}

export async function sendChatMessage(
  content: string,
): Promise<DaemonResponse<{ chat_history: ChatMessage[] }>> {
  return sendRequest({ type: "chat_message", content }, CHAT_TIMEOUT_MS)
}

export async function getRunState(
  logTail = 200,
  metricTail = 1,
): Promise<DaemonResponse<{ run_state: RunStateSnapshot }>> {
  return sendRequest({ type: "get_run_state", log_tail: logTail, metric_tail: metricTail })
}

export function startDaemonPolling(
  onStatus: (status: DaemonStatus) => void,
): () => void {
  let running = true

  const poll = async () => {
    if (!running) return
    const status = await checkHealth()
    if (running) onStatus(status)
  }

  poll()
  const interval = setInterval(poll, 1000)

  return () => {
    running = false
    clearInterval(interval)
  }
}
