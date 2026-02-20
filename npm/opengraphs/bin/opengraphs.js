#!/usr/bin/env node
"use strict";

const crypto = require("crypto");
const fs = require("fs");
const fsp = fs.promises;
const https = require("https");
const os = require("os");
const path = require("path");
const { spawn, spawnSync } = require("child_process");

const REPO = process.env.OG_REPO || "vyomakesh0728/opengraphs";
const REQUESTED_VERSION = process.env.OG_VERSION || "latest";
const CACHE_ROOT = process.env.XDG_CACHE_HOME
  ? path.join(process.env.XDG_CACHE_HOME, "opengraphs")
  : path.join(os.homedir(), ".cache", "opengraphs");

function detectTarget() {
  const platform = process.platform;
  const arch = process.arch;

  let osPart;
  if (platform === "darwin") {
    osPart = "apple-darwin";
  } else if (platform === "linux") {
    osPart = "unknown-linux-gnu";
  } else {
    throw new Error(`unsupported platform: ${platform}`);
  }

  let archPart;
  if (arch === "x64") {
    archPart = "x86_64";
  } else if (arch === "arm64") {
    archPart = "aarch64";
  } else {
    throw new Error(`unsupported architecture: ${arch}`);
  }

  return `${archPart}-${osPart}`;
}

function requestBuffer(url, headers = {}, redirects = 5) {
  return new Promise((resolve, reject) => {
    const req = https.get(
      url,
      {
        headers: {
          "user-agent": "opengraphs-npx",
          ...headers,
        },
      },
      (res) => {
        const status = res.statusCode || 0;
        const location = res.headers.location;

        if ([301, 302, 303, 307, 308].includes(status) && location && redirects > 0) {
          const nextUrl = location.startsWith("http") ? location : new URL(location, url).toString();
          res.resume();
          resolve(requestBuffer(nextUrl, headers, redirects - 1));
          return;
        }

        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => {
          const body = Buffer.concat(chunks);
          if (status < 200 || status >= 300) {
            reject(new Error(`request failed: ${status} ${url}`));
            return;
          }
          resolve(body);
        });
      },
    );

    req.on("error", reject);
  });
}

async function resolveVersion() {
  if (REQUESTED_VERSION !== "latest") {
    return REQUESTED_VERSION;
  }

  const token = process.env.GITHUB_TOKEN || process.env.OG_GITHUB_TOKEN;
  const headers = token ? { authorization: `Bearer ${token}` } : {};
  const raw = await requestBuffer(`https://api.github.com/repos/${REPO}/releases/latest`, headers);
  const parsed = JSON.parse(raw.toString("utf8"));
  if (!parsed.tag_name) {
    throw new Error(`could not resolve latest release for ${REPO}`);
  }
  return parsed.tag_name;
}

function parseChecksum(shaFileText) {
  for (const line of shaFileText.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }
    const token = trimmed.split(/\s+/)[0];
    if (/^[a-fA-F0-9]{64}$/.test(token)) {
      return token.toLowerCase();
    }
  }
  throw new Error("invalid checksum file");
}

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function ensureTarAvailable() {
  const check = spawnSync("tar", ["--version"], { stdio: "ignore" });
  if (check.status !== 0) {
    throw new Error("tar is required but was not found");
  }
}

async function copyExecutable(src, dst) {
  await fsp.copyFile(src, dst);
  await fsp.chmod(dst, 0o755);
}

async function ensureInstalled(version, target) {
  const installDir = path.join(CACHE_ROOT, version, target);
  const ogtuiPath = path.join(installDir, "ogtui");

  try {
    await fsp.access(ogtuiPath, fs.constants.X_OK);
    return ogtuiPath;
  } catch (_e) {
    // cache miss, continue
  }

  ensureTarAvailable();
  const archive = `opengraphs-${version}-${target}.tar.gz`;
  const baseUrl = `https://github.com/${REPO}/releases/download/${version}`;
  const archiveUrl = `${baseUrl}/${archive}`;
  const checksumUrl = `${archiveUrl}.sha256`;

  const tmpDir = await fsp.mkdtemp(path.join(os.tmpdir(), "opengraphs-npx-"));
  const archivePath = path.join(tmpDir, archive);

  try {
    process.stderr.write(`Downloading ${archiveUrl}\n`);
    const archiveData = await requestBuffer(archiveUrl);
    const checksumData = await requestBuffer(checksumUrl);
    const expected = parseChecksum(checksumData.toString("utf8"));
    const actual = sha256(archiveData);
    if (expected !== actual) {
      throw new Error(`checksum mismatch for ${archive}`);
    }

    await fsp.writeFile(archivePath, archiveData);
    const untar = spawnSync("tar", ["-xzf", archivePath, "-C", tmpDir], { stdio: "inherit" });
    if (untar.status !== 0) {
      throw new Error("failed to extract release archive");
    }

    const pkgDir = path.join(tmpDir, `opengraphs-${version}-${target}`);
    const sourceOgtui = path.join(pkgDir, "ogtui");
    const sourceOgd = path.join(pkgDir, "ogd");

    await fsp.mkdir(installDir, { recursive: true });
    await copyExecutable(sourceOgtui, ogtuiPath);
    await copyExecutable(sourceOgd, path.join(installDir, "ogd"));
  } finally {
    await fsp.rm(tmpDir, { recursive: true, force: true });
  }

  return ogtuiPath;
}

async function main() {
  const target = detectTarget();
  const version = await resolveVersion();
  const ogtuiPath = await ensureInstalled(version, target);
  const args = process.argv.slice(2);

  const child = spawn(ogtuiPath, args, { stdio: "inherit" });
  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    } else {
      process.exit(code || 0);
    }
  });
  child.on("error", (err) => {
    process.stderr.write(`failed to run ogtui: ${err.message}\n`);
    process.exit(1);
  });
}

main().catch((err) => {
  process.stderr.write(`opengraphs npx failed: ${err.message}\n`);
  process.exit(1);
});
