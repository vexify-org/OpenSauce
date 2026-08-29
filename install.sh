#!/usr/bin/env bash
#
# OpenSauce installer — Powers By Vexify.
#
# Auto-detects the latest release version, then installs the OpenSauce binary:
#   - prefers a prebuilt release asset if one exists
#   - otherwise builds from source with the release profile (opt-level z)
#
# Usage:
#   ./install.sh                            # latest release
#   VERSION=v1.0.0 ./install.sh             # a specific version
#
set -euo pipefail

REPO="vexify-org/OpenSauce"
BIN="opensauce"

log()  { printf '\033[1;34m[OpenSauce]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[OpenSauce]\033[0m %s\n' "$*" >&2; }
err()  { printf '\033[1;31m[OpenSauce]\033[0m ERROR: %s\n' "$*" >&2; exit 1; }

# ---- Detect version -----------------------------------------------------
detect_version() {
  if [[ -n "${VERSION:-}" ]]; then
    echo "${VERSION#v}"
    return
  fi
  # 1) latest GitHub release
  if command -v gh >/dev/null 2>&1; then
    local tag
    if tag="$(gh release view --repo "$REPO" --json tagName -q .tagName 2>/dev/null)"; then
      echo "${tag#v}"; return
    fi
  fi
  # 2) GitHub API
  local url="https://api.github.com/repos/$REPO/releases/latest"
  if command -v curl >/dev/null 2>&1; then
    local tag
    if tag="$(curl -fsSL "$url" 2>/dev/null | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"; then
      [[ -n "$tag" ]] && { echo "${tag#v}"; return; }
    fi
  fi
  # 3) git tags in a checked-out clone
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    local tag
    if tag="$(git describe --tags --abbrev=0 2>/dev/null)"; then
      echo "${tag#v}"; return
    fi
  fi
  err "无法自动识别版本。请显式指定 VERSION=v1.0.0 ./install.sh"
}

# ---- Detect platform ----------------------------------------------------
platform() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
  esac
  case "$os" in
    darwin) os="darwin" ;;
    linux) os="linux" ;;
    *) err "不支持的系统: $os" ;;
  esac
  echo "${os}-${arch}"
}

# ---- Resolve install directory -----------------------------------------
install_dir() {
  local dir
  dir="${INSTALL_DIR:-}"
  if [[ -z "$dir" ]]; then
    if [[ "$(id -u)" -eq 0 ]] && [[ -w /usr/local/bin ]]; then
      dir="/usr/local/bin"
    elif [[ -d "$HOME/.local/bin" ]] || mkdir -p "$HOME/.local/bin" 2>/dev/null; then
      dir="$HOME/.local/bin"
    elif [[ -d "$CARGO_HOME/bin" ]] || mkdir -p "${CARGO_HOME:-$HOME/.cargo}/bin" 2>/dev/null; then
      dir="${CARGO_HOME:-$HOME/.cargo}/bin"
    else
      err "无法确定安装目录（可用 INSTALL_DIR=/path ./install.sh）"
    fi
  fi
  mkdir -p "$dir"
  echo "$dir"
}

# ---- Try a prebuilt asset ----------------------------------------------
# Asset naming: opensauce-<platform>-<version> (e.g. opensauce-linux-x86_64-1.0.0)
try_prebuilt() {
  local platform="$1" version="$2" dir="$3"
  local base="$BIN-${platform}-${version}"
  local url="https://github.com/$REPO/releases/download/v${version}/${base}"
  local tmp
  tmp="$(mktemp -d)"
  if command -v curl >/dev/null 2>&1 && curl -fsSL -o "$tmp/$BIN" "$url" 2>/dev/null; then
    chmod +x "$tmp/$BIN"
    log "下载预编译二进制 $base"
    mv -f "$tmp/$BIN" "$dir/$BIN"
    rm -rf "$tmp"
    return 0
  fi
  rm -rf "$tmp"
  return 1
}

# ---- Build from source --------------------------------------------------
build_source() {
  local version="$1" dir="$2"
  command -v cargo >/dev/null 2>&1 || err "未找到 cargo。请先安装 Rust: https://rustup.rs"
  local tmp
  tmp="$(mktemp -d)"
  log "从源码构建 OpenSauce v${version}（release, opt-level z）…"
  git clone --depth 1 --branch "v${version}" "https://github.com/$REPO.git" "$tmp/src" 2>/dev/null \
    || git clone --depth 1 "https://github.com/$REPO.git" "$tmp/src" \
    || err "无法克隆仓库"
  ( cd "$tmp/src" && cargo build --release )
  local built="$tmp/src/target/release/$BIN"
  [[ -f "$built" ]] || err "构建完成但找不到 $built"
  cp -f "$built" "$dir/$BIN"
  chmod +x "$dir/$BIN"
  rm -rf "$tmp"
}

# ---- Main --------------------------------------------------------------
version="$(detect_version)"
p="$(platform)"
dir="$(install_dir)"

log "版本: v${version}  ·  平台: ${p}  ·  安装到: ${dir}"

if ! try_prebuilt "$p" "$version" "$dir"; then
  warn "没有预编译产物，改为源码构建。"
  build_source "$version" "$dir"
fi

log "安装完成 → ${dir}/${BIN}（v${version}）"
log "确保 ${dir} 在 PATH 中后，运行: opensauce connect   # 首次配置大模型连接"
log "                        opensauce                # 唤起 TUI（Build/Plan）"