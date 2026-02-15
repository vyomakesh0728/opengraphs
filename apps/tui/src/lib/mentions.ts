let cachedFiles: string[] | null = null

async function getFiles(): Promise<string[]> {
  if (cachedFiles) return cachedFiles

  try {
    const tracked = Bun.spawnSync(["git", "ls-files"], {
      stdout: "pipe",
      stderr: "pipe",
    })
    const untracked = Bun.spawnSync(
      ["git", "ls-files", "--others", "--exclude-standard"],
      { stdout: "pipe", stderr: "pipe" },
    )

    const trackedStr = tracked.stdout.toString().trim()
    const untrackedStr = untracked.stdout.toString().trim()
    const all = new Set<string>()
    if (trackedStr) trackedStr.split("\n").forEach((f) => all.add(f))
    if (untrackedStr) untrackedStr.split("\n").forEach((f) => all.add(f))
    cachedFiles = [...all]
  } catch {
    try {
      const rg = Bun.spawnSync(["rg", "--files"], {
        stdout: "pipe",
        stderr: "pipe",
      })
      cachedFiles = rg.stdout.toString().trim().split("\n").filter(Boolean)
    } catch {
      cachedFiles = []
    }
  }

  return cachedFiles
}

export async function getMentionCandidates(
  query: string,
): Promise<string[]> {
  const files = await getFiles()
  if (!query) return files.slice(0, 50)

  const q = query.toLowerCase()
  const results = files.filter((f) => {
    const lower = f.toLowerCase()
    return lower.startsWith(q) || lower.includes(q)
  })

  return results.slice(0, 50)
}

export function invalidateCache() {
  cachedFiles = null
}
