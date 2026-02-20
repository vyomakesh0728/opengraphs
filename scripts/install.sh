#!/usr/bin/env bash
set -euo pipefail

REPO="${OG_REPO:-vyomakesh0728/opengraphs}"
VERSION="${OG_VERSION:-latest}"
PREFIX="${OG_PREFIX:-$HOME/.local}"
BIN_DIR=""

usage() {
  cat <<'EOF'
Install OpenGraphs binaries (ogtui + ogd + og alias) from GitHub releases.

Usage:
  install.sh [--version <tag>] [--prefix <dir>] [--bin-dir <dir>] [--repo <owner/name>]

Options:
  --version <tag>   Release tag to install (example: v0.1.0). Default: latest
  --prefix <dir>    Install prefix. Default: ~/.local
  --bin-dir <dir>   Install bin directory. Overrides --prefix/bin
  --repo <name>     GitHub repo in owner/name format. Default: vyomakesh0728/opengraphs
  -h, --help        Show this help

Examples:
  curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash -s -- --version v0.1.0
EOF
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command '$1' not found" >&2
    exit 1
  fi
}

resolve_latest_version() {
  local api_url
  local tag
  api_url="https://api.github.com/repos/${REPO}/releases/latest"
  tag="$(curl -fsSL "${api_url}" | awk -F '"' '/"tag_name":/ { print $4; exit }')"
  if [[ -z "${tag}" ]]; then
    echo "error: could not resolve latest release tag for ${REPO}" >&2
    echo "hint: create a tagged release first, e.g. v0.1.0" >&2
    exit 1
  fi
  VERSION="${tag}"
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-gnu" ;;
    *)
      echo "error: unsupported OS '${os}'" >&2
      exit 1
      ;;
  esac

  case "${arch}" in
    x86_64 | amd64) arch="x86_64" ;;
    arm64 | aarch64) arch="aarch64" ;;
    *)
      echo "error: unsupported architecture '${arch}'" >&2
      exit 1
      ;;
  esac

  TARGET="${arch}-${os}"
}

verify_checksum() {
  local archive_path checksum_path
  archive_path="$1"
  checksum_path="$2"
  local expected actual

  expected="$(awk 'NF { print $1; exit }' "${checksum_path}")"
  if [[ -z "${expected}" ]]; then
    echo "error: checksum file is empty or invalid: ${checksum_path}" >&2
    exit 1
  fi

  if command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "${archive_path}" | awk '{print $1}')"
    if [[ "${actual}" != "${expected}" ]]; then
      echo "error: checksum mismatch for $(basename "${archive_path}")" >&2
      exit 1
    fi
    return 0
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${archive_path}" | awk '{print $1}')"
    if [[ "${actual}" != "${expected}" ]]; then
      echo "error: checksum mismatch for $(basename "${archive_path}")" >&2
      exit 1
    fi
    return 0
  fi

  echo "warning: no sha256 tool found (shasum/sha256sum); skipping checksum verification" >&2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="$2"
      shift 2
      ;;
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    --bin-dir)
      BIN_DIR="$2"
      shift 2
      ;;
    --repo)
      REPO="$2"
      shift 2
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

if [[ -z "${BIN_DIR}" ]]; then
  BIN_DIR="${PREFIX}/bin"
fi

need_cmd curl
need_cmd tar
need_cmd mktemp
need_cmd install

detect_target

if [[ "${VERSION}" == "latest" ]]; then
  resolve_latest_version
fi

ARCHIVE="opengraphs-${VERSION}-${TARGET}.tar.gz"
CHECKSUM="${ARCHIVE}.sha256"
RELEASE_BASE="https://github.com/${REPO}/releases/download/${VERSION}"
ARCHIVE_URL="${RELEASE_BASE}/${ARCHIVE}"
CHECKSUM_URL="${RELEASE_BASE}/${CHECKSUM}"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

echo "Installing OpenGraphs ${VERSION} for ${TARGET}..."
echo "Downloading ${ARCHIVE_URL}"
curl -fL "${ARCHIVE_URL}" -o "${tmp_dir}/${ARCHIVE}"

if curl -fsSL "${CHECKSUM_URL}" -o "${tmp_dir}/${CHECKSUM}"; then
  verify_checksum "${tmp_dir}/${ARCHIVE}" "${tmp_dir}/${CHECKSUM}"
else
  echo "warning: checksum file not found, continuing without verification" >&2
fi

tar -xzf "${tmp_dir}/${ARCHIVE}" -C "${tmp_dir}"

pkg_dir="${tmp_dir}/opengraphs-${VERSION}-${TARGET}"
if [[ ! -f "${pkg_dir}/ogtui" || ! -f "${pkg_dir}/ogd" ]]; then
  echo "error: archive did not contain expected ogtui/ogd binaries" >&2
  exit 1
fi

mkdir -p "${BIN_DIR}"
install -m 0755 "${pkg_dir}/ogtui" "${BIN_DIR}/ogtui"
install -m 0755 "${pkg_dir}/ogd" "${BIN_DIR}/ogd"
install -m 0755 "${pkg_dir}/ogtui" "${BIN_DIR}/og"

echo
echo "Installed:"
echo "  ${BIN_DIR}/ogtui"
echo "  ${BIN_DIR}/ogd"
echo "  ${BIN_DIR}/og"

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
  echo
  echo "Add this to your shell profile to use the binaries globally:"
  echo "  export PATH=\"${BIN_DIR}:\$PATH\""
fi

echo
echo "Done. Run: og --help"
