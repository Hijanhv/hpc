#!/usr/bin/env bash
#
# ci-local.sh — run the full CI pipeline locally, matching .github/workflows/ci.yml.
#
# Runs, in order: rustfmt check, clippy (warnings-as-errors), the workspace test
# suite, and a release build. Stops at the first failing stage. This is the same
# gate CI enforces, so a green run here means a green run there.
#
# Usage:
#   scripts/ci-local.sh [--fix] [--bench] [-h|--help]
#
# Options:
#   --fix       Apply `cargo fmt` instead of only checking formatting.
#   --bench     Additionally run the Criterion benchmarks after the build.
#   -h, --help  Show this help and exit.
set -euo pipefail

if [[ -t 1 ]]; then
  C_RESET=$'\033[0m'; C_INFO=$'\033[0;34m'; C_OK=$'\033[0;32m'; C_WARN=$'\033[0;33m'; C_ERR=$'\033[0;31m'
else
  C_RESET=""; C_INFO=""; C_OK=""; C_WARN=""; C_ERR=""
fi
log()  { printf '%s[hpc]%s  %s\n'  "$C_INFO" "$C_RESET" "$*"; }
ok()   { printf '%s[ ok ]%s %s\n'  "$C_OK"   "$C_RESET" "$*"; }
warn() { printf '%s[warn]%s %s\n'  "$C_WARN" "$C_RESET" "$*" >&2; }
die()  { printf '%s[fail]%s %s\n'  "$C_ERR"  "$C_RESET" "$*" >&2; exit 1; }

FIX=false
RUN_BENCH=false
usage() { awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --fix) FIX=true ;;
    --bench) RUN_BENCH=true ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
  shift
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Run a named stage, timing it and failing loudly.
stage() {
  local name="$1"; shift
  log "▶ ${name}: $*"
  local start; start=$(date +%s)
  if "$@"; then
    local end; end=$(date +%s)
    ok "${name} passed ($((end - start))s)"
  else
    die "${name} failed"
  fi
}

if [[ "$FIX" == true ]]; then
  stage "fmt (apply)" cargo fmt --all
else
  stage "fmt (check)" cargo fmt --all --check
fi
stage "clippy"  cargo clippy --workspace --all-targets -- -D warnings
stage "test"    cargo test --workspace
stage "build"   cargo build --workspace --release

if [[ "$RUN_BENCH" == true ]]; then
  stage "bench" cargo bench -p hpc-bench
fi

ok "local pipeline green — safe to push"
