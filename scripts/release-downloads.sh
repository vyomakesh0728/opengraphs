#!/usr/bin/env bash
set -euo pipefail

REPO="${OG_REPO:-vyomakesh0728/opengraphs}"
LIMIT=100
OUTPUT="table"

usage() {
  cat <<'EOF'
Report GitHub release download counts for OpenGraphs.

This breaks out binary archive downloads from checksum downloads so the
"real" install pull count is easier to read.

Usage:
  release-downloads.sh [--repo <owner/name>] [--limit <n>] [--json]

Options:
  --repo <name>   GitHub repo in owner/name format. Default: vyomakesh0728/opengraphs
  --limit <n>     Number of recent releases to fetch (max 100). Default: 100
  --json          Print machine-readable JSON
  -h, --help      Show this help
EOF
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command '$1' not found" >&2
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      REPO="$2"
      shift 2
      ;;
    --limit)
      LIMIT="$2"
      shift 2
      ;;
    --json)
      OUTPUT="json"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument '$1'" >&2
      usage
      exit 1
      ;;
  esac
done

if ! [[ "${LIMIT}" =~ ^[0-9]+$ ]] || [[ "${LIMIT}" -lt 1 ]] || [[ "${LIMIT}" -gt 100 ]]; then
  echo "error: --limit must be an integer between 1 and 100" >&2
  exit 1
fi

need_cmd curl
need_cmd jq

API_URL="https://api.github.com/repos/${REPO}/releases?per_page=${LIMIT}"
RELEASES_JSON="$(curl -fsSL "${API_URL}")"

if [[ "$(jq -r 'type' <<<"${RELEASES_JSON}")" != "array" ]]; then
  echo "error: unexpected GitHub API response for ${REPO}" >&2
  jq -r '.' <<<"${RELEASES_JSON}" >&2
  exit 1
fi

if [[ "$(jq 'length' <<<"${RELEASES_JSON}")" -eq 0 ]]; then
  if [[ "${OUTPUT}" == "json" ]]; then
    jq -n --arg repo "${REPO}" '{repo: $repo, totals: {binaries: 0, checksums: 0, all_assets: 0}, releases: []}'
  else
    echo "No releases found for ${REPO}."
  fi
  exit 0
fi

if [[ "${OUTPUT}" == "json" ]]; then
  jq --arg repo "${REPO}" '
  {
    repo: $repo,
    generated_at: (now | todate),
    totals: {
      binaries: (map(.assets[]? | select(.name | endswith(".tar.gz")) | .download_count) | add // 0),
      checksums: (map(.assets[]? | select(.name | endswith(".sha256")) | .download_count) | add // 0),
      all_assets: (map(.assets[]?.download_count) | add // 0)
    },
    releases: map({
      tag: .tag_name,
      published_at,
      binaries: ([.assets[]? | select(.name | endswith(".tar.gz")) | .download_count] | add // 0),
      checksums: ([.assets[]? | select(.name | endswith(".sha256")) | .download_count] | add // 0),
      all_assets: ([.assets[]?.download_count] | add // 0)
    })
  }' <<<"${RELEASES_JSON}"
  exit 0
fi

read -r TOTAL_BIN TOTAL_SHA TOTAL_ALL < <(
  jq -r '
  [
    (map(.assets[]? | select(.name | endswith(".tar.gz")) | .download_count) | add // 0),
    (map(.assets[]? | select(.name | endswith(".sha256")) | .download_count) | add // 0),
    (map(.assets[]?.download_count) | add // 0)
  ] | @tsv' <<<"${RELEASES_JSON}"
)

echo "Repo: ${REPO}"
echo
printf "%-12s %10s %10s %10s\n" "Release" "Binaries" "Checksums" "AllAssets"
printf "%-12s %10s %10s %10s\n" "-------" "--------" "---------" "---------"

jq -r '
map({
  tag: .tag_name,
  binaries: ([.assets[]? | select(.name | endswith(".tar.gz")) | .download_count] | add // 0),
  checksums: ([.assets[]? | select(.name | endswith(".sha256")) | .download_count] | add // 0),
  all_assets: ([.assets[]?.download_count] | add // 0)
})[] | [.tag, .binaries, .checksums, .all_assets] | @tsv' <<<"${RELEASES_JSON}" |
while IFS=$'\t' read -r TAG BIN SHA ALL; do
  printf "%-12s %10s %10s %10s\n" "${TAG}" "${BIN}" "${SHA}" "${ALL}"
done

echo
echo "Totals:"
echo "  Binary-only (.tar.gz): ${TOTAL_BIN}"
echo "  Checksum-only (.sha256): ${TOTAL_SHA}"
echo "  All release assets: ${TOTAL_ALL}"
echo
echo "Note: download_count is not unique users (retries, CI, and repeat pulls are counted)."
