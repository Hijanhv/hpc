#!/usr/bin/env bash
#
# deploy-agent.sh — deploy the hpc-agent binary to a remote node over SSH.
#
# Builds a release hpc-agent (unless one is supplied), verifies SSH reachability,
# copies the binary and a config onto the node, installs them under a target
# directory, and optionally starts the agent. Designed to be safe to re-run
# (idempotent copy + restart).
#
# Usage:
#   scripts/deploy-agent.sh --host user@node [options]
#
# Required:
#   --host user@host      SSH destination of the target node.
#
# Options:
#   --endpoint URL        Daemon gRPC endpoint the agent should dial
#                         (default: http://127.0.0.1:7443).
#   --binary PATH         Prebuilt hpc-agent binary (default: build from source).
#   --config PATH         Agent config to ship (default: configs/agent.toml).
#   --remote-dir DIR      Install directory on the node (default: /opt/hpc).
#   --start               Launch the agent after install (nohup, backgrounded).
#   --no-build            Do not build even if --binary is absent (then required).
#   -h, --help            Show this help and exit.
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

HOST=""
ENDPOINT="http://127.0.0.1:7443"
BINARY=""
CONFIG="configs/agent.toml"
REMOTE_DIR="/opt/hpc"
START=false
BUILD=true
usage() { awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) HOST="${2:?--host needs a value}"; shift ;;
    --endpoint) ENDPOINT="${2:?}"; shift ;;
    --binary) BINARY="${2:?}"; shift ;;
    --config) CONFIG="${2:?}"; shift ;;
    --remote-dir) REMOTE_DIR="${2:?}"; shift ;;
    --start) START=true ;;
    --no-build) BUILD=false ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
  shift
done

[[ -n "$HOST" ]] || die "--host is required (try --help)"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# --- 1. Obtain the binary ---------------------------------------------------
if [[ -z "$BINARY" ]]; then
  BINARY="target/release/hpc-agent"
  if [[ ! -x "$BINARY" ]]; then
    if [[ "$BUILD" == true ]]; then
      log "building hpc-agent (release)"
      cargo build --release -p hpc-agent
    else
      die "no binary at ${BINARY} and --no-build was set"
    fi
  fi
fi
[[ -x "$BINARY" ]] || die "binary not found or not executable: ${BINARY}"
[[ -f "$CONFIG" ]] || die "config not found: ${CONFIG}"

# --- 2. Verify SSH reachability --------------------------------------------
log "checking SSH connectivity to ${HOST}"
ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" 'echo ok' >/dev/null 2>&1 \
  || die "cannot SSH to ${HOST} (need key-based auth and reachability)"
ok "SSH to ${HOST} works"

# --- 3. Copy + install ------------------------------------------------------
log "creating ${REMOTE_DIR} on ${HOST}"
ssh "$HOST" "sudo mkdir -p '${REMOTE_DIR}' && sudo chown \"\$(id -un)\" '${REMOTE_DIR}'"

log "copying binary and config"
scp -q "$BINARY" "${HOST}:${REMOTE_DIR}/hpc-agent.new"
scp -q "$CONFIG" "${HOST}:${REMOTE_DIR}/agent.toml"

# Atomic swap of the binary so a running agent is never half-written.
ssh "$HOST" "chmod +x '${REMOTE_DIR}/hpc-agent.new' && mv -f '${REMOTE_DIR}/hpc-agent.new' '${REMOTE_DIR}/hpc-agent'"
ok "installed to ${HOST}:${REMOTE_DIR}/hpc-agent"

# --- 4. Optionally start ----------------------------------------------------
if [[ "$START" == true ]]; then
  log "starting agent (endpoint=${ENDPOINT})"
  ssh "$HOST" "cd '${REMOTE_DIR}' && nohup ./hpc-agent --config agent.toml --endpoint '${ENDPOINT}' \
      > '${REMOTE_DIR}/agent.log' 2>&1 & echo \"started pid \$!\""
  ok "agent started on ${HOST} (logs: ${REMOTE_DIR}/agent.log)"
else
  log "not started; run it on the node with:"
  printf "    ssh %s 'cd %s && ./hpc-agent --config agent.toml --endpoint %s'\n" \
    "$HOST" "$REMOTE_DIR" "$ENDPOINT"
fi

ok "deploy complete"
