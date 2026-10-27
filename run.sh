#!/usr/bin/env bash
# Build and run Moonlight Web.
#
#   ./run.sh            build everything (release) and run the bridge  (same as `run`)
#   ./run.sh build      build web UI + bridge (release) only
#   ./run.sh run        build, then run on $MLW_BIND (default 0.0.0.0:8080)
#   ./run.sh dev        bridge (debug) + Vite dev server with hot reload
#   ./run.sh check      type-check UI, run Rust tests + clippy
#
# If the toolchain isn't on PATH, the script re-executes itself inside the
# Nix dev shell (flake if available, otherwise shell.nix).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null || ! command -v npm >/dev/null || ! command -v cmake >/dev/null; then
  if [[ -n "${MLW_IN_NIX_SHELL:-}" ]]; then
    echo "error: toolchain still missing inside the nix shell" >&2
    exit 1
  fi
  export MLW_IN_NIX_SHELL=1
  if nix flake --help >/dev/null 2>&1 && [[ -f flake.nix ]]; then
    # Git-based flake ref (not path:), so target/ and node_modules/ aren't copied to the store.
    exec nix develop "$ROOT" --command "$0" "$@"
  else
    exec nix-shell "$ROOT/shell.nix" --run "$(printf '%q ' "$0" "$@")"
  fi
fi

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

ensure_submodules() {
  if [[ ! -f moonlight-common-sys/moonlight-common-c/CMakeLists.txt ]]; then
    log "Fetching moonlight-common-c submodule"
    git submodule update --init --recursive
  fi
}

ensure_node_modules() {
  if [[ ! -d web/node_modules || web/package.json -nt web/node_modules ]]; then
    log "Installing web dependencies"
    (cd web && npm install --no-audit --no-fund)
    touch web/node_modules
  fi
}

build() {
  ensure_submodules
  ensure_node_modules
  log "Building web UI"
  (cd web && npm run build)
  log "Building bridge (release)"
  cargo build --release
}

run() {
  build
  log "Starting Moonlight Web on http://${MLW_BIND:-0.0.0.0:8080}"
  exec ./target/release/moonlight-web
}

dev() {
  ensure_submodules
  ensure_node_modules
  log "Building bridge (debug)"
  cargo build
  log "Bridge on :8080, UI dev server on http://localhost:5173 (Ctrl+C stops both)"
  ./target/debug/moonlight-web &
  local bridge=$!
  trap 'kill "$bridge" 2>/dev/null || true' EXIT INT TERM
  (cd web && npm run dev)
}

check() {
  ensure_submodules
  ensure_node_modules
  log "Type-checking web UI"
  (cd web && npm run check)
  log "Running Rust tests"
  cargo test
  log "Running clippy"
  cargo clippy --all-targets -- -D warnings
}

case "${1:-run}" in
  build) build ;;
  run) run ;;
  dev) dev ;;
  check) check ;;
  -h | --help | help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//' ;;
  *)
    echo "unknown command: $1 (try: build, run, dev, check)" >&2
    exit 2
    ;;
esac
