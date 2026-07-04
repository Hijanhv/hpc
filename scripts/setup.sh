#!/usr/bin/env bash
#
# setup.sh — prepare a machine to build and run the hpc workspace.
#
# Verifies the Rust toolchain, ensures the clippy/rustfmt components are
# present, installs the native build dependencies (protoc for the gRPC build
# scripts, libclang for hpc-ffi's bindgen step), and primes the cargo cache.
#
# Usage:
#   scripts/setup.sh [--yes] [--skip-system] [-h|--help]
#
# Options:
#   --yes           Actually install system packages (otherwise they are only
#                   reported — the script never sudo-installs without consent).
#   --skip-system   Skip the system-dependency step entirely.
#   -h, --help      Show this help and exit.
set -euo pipefail

# --- pretty logging ---------------------------------------------------------
if [[ -t 1 ]]; then
  C_RESET=$'\033[0m'; C_INFO=$'\033[0;34m'; C_OK=$'\033[0;32m'; C_WARN=$'\033[0;33m'; C_ERR=$'\033[0;31m'
else
  C_RESET=""; C_INFO=""; C_OK=""; C_WARN=""; C_ERR=""
fi
log()  { printf '%s[hpc]%s  %s\n'  "$C_INFO" "$C_RESET" "$*"; }
ok()   { printf '%s[ ok ]%s %s\n'  "$C_OK"   "$C_RESET" "$*"; }
warn() { printf '%s[warn]%s %s\n'  "$C_WARN" "$C_RESET" "$*" >&2; }
die()  { printf '%s[fail]%s %s\n'  "$C_ERR"  "$C_RESET" "$*" >&2; exit 1; }

MIN_RUST="1.82"
DO_INSTALL=false
SKIP_SYSTEM=false

usage() { awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --yes) DO_INSTALL=true ;;
    --skip-system) SKIP_SYSTEM=true ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
  shift
done

# --- 1. Rust toolchain ------------------------------------------------------
log "checking Rust toolchain (need >= ${MIN_RUST})"
command -v cargo >/dev/null 2>&1 || die "cargo not found — install Rust from https://rustup.rs"
rustc_version="$(rustc --version | awk '{print $2}')"
# Sort-based semver comparison: the smaller of (min, found) must be min.
if [[ "$(printf '%s\n%s\n' "$MIN_RUST" "$rustc_version" | sort -V | head -n1)" != "$MIN_RUST" ]]; then
  die "rustc ${rustc_version} is older than the required ${MIN_RUST}"
fi
ok "rustc ${rustc_version}"

if command -v rustup >/dev/null 2>&1; then
  log "ensuring clippy + rustfmt components"
  rustup component add clippy rustfmt >/dev/null 2>&1 || warn "could not add components via rustup"
  ok "clippy + rustfmt present"
else
  warn "rustup not found; assuming clippy/rustfmt are installed by other means"
fi

# --- 2. System dependencies -------------------------------------------------
if [[ "$SKIP_SYSTEM" == true ]]; then
  warn "skipping system dependencies (--skip-system)"
else
  log "checking native build dependencies (protoc, libclang)"
  missing=()
  command -v protoc >/dev/null 2>&1 || missing+=("protobuf-compiler")
  # libclang backs hpc-ffi's bindgen build step.
  if ! ls /usr/lib/llvm-*/lib/libclang.so* /usr/lib/*/libclang.so* \
        /Library/Developer/CommandLineTools/usr/lib/libclang.dylib >/dev/null 2>&1; then
    missing+=("libclang-dev")
  fi

  if [[ ${#missing[@]} -eq 0 ]]; then
    ok "all native dependencies present"
  elif command -v apt-get >/dev/null 2>&1; then
    pkgs=(protobuf-compiler llvm-dev libclang-dev clang)
    if [[ "$DO_INSTALL" == true ]]; then
      log "installing: ${pkgs[*]}"
      sudo apt-get update -y && sudo apt-get install -y "${pkgs[@]}"
      ok "system dependencies installed"
    else
      warn "missing: ${missing[*]}"
      warn "re-run with --yes to install, or manually: sudo apt-get install -y ${pkgs[*]}"
    fi
  elif command -v brew >/dev/null 2>&1; then
    warn "missing: ${missing[*]}"
    warn "install on macOS with: brew install protobuf llvm"
  else
    warn "missing: ${missing[*]} — install protobuf-compiler and libclang for your distro"
  fi
fi

# --- 3. Prime the cargo cache ----------------------------------------------
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
log "fetching cargo dependencies"
( cd "$repo_root" && cargo fetch ) && ok "dependencies fetched"

log "setup complete — next steps:"
printf '    cargo build --release\n'
printf '    ./target/release/hpc-daemon --config configs/daemon.toml\n'
printf '    scripts/ci-local.sh   # run the full local pipeline\n'
