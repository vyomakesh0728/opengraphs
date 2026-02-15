import { connect, type Socket } from "net"

const OGD_SOCKET =
  process.env.OGD_SOCKET ??
  `${process.env.TMPDIR ?? process.env.TEMP ?? process.env.TMP ?? "/tmp"}/opengraphs-ogd.sock`

export type DaemonStatus = { ok: true } | { ok: false; error: string }

function checkHealth(): Promise<DaemonStatus> {
  return new Promise((resolve) => {
    let socket: Socket | null = null
    const timeout = setTimeout(() => {
      socket?.destroy()
      resolve({ ok: false, error: "timeout" })
    }, 2000)

    try {
      socket = connect(OGD_SOCKET, () => {
        socket!.write("GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
      })
      let data = ""
      socket.on("data", (chunk) => {
        data += chunk.toString()
      })
      socket.on("end", () => {
        clearTimeout(timeout)
        if (data.includes("200")) {
          resolve({ ok: true })
        } else {
          resolve({ ok: false, error: data.trim() || "bad response" })
        }
      })
      socket.on("error", (err) => {
        clearTimeout(timeout)
        resolve({ ok: false, error: err.message })
      })
    } catch (e: any) {
      clearTimeout(timeout)
      resolve({ ok: false, error: e.message })
    }
  })
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
