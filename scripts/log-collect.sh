#!/usr/bin/env bash
#
# log-collect.sh — gather logs and diagnostics from cluster nodes into one bundle.
#
# For each node it pulls the remote log directory and (if hpc-diag is installed
# there) a fresh diagnostic bundle, into a per-node folder, then rolls everything
# up into a single timestamped tar.gz with a manifest. A failure on one node is
# logged and does not abort the others.
#
# Usage:
#   scripts/log-collect.sh (--nodes "u@h1 u@h2" | --nodes-file FILE) [options]
#
# Node selection (one required):
#   --nodes "LIST"     Space-separated SSH destinations.
#   --nodes-file FILE  File with one SSH destination per line ('#' comments ok).
#
# Options:
#   --remote-log DIR   Remote log directory (default: /opt/hpc).
#   --out FILE         Output tarball (default: hpc-logs-<timestamp>.tar.gz).
#   -h, --help         Show this help and exit.
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

NODES=""
NODES_FILE=""
REMOTE_LOG="/opt/hpc"
OUT=""
usage() { awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --nodes) NODES="${2:?--nodes needs a value}"; shift ;;
    --nodes-file) NODES_FILE="${2:?}"; shift ;;
    --remote-log) REMOTE_LOG="${2:?}"; shift ;;
    --out) OUT="${2:?}"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
  shift
done

# Build the node list from either source.
declare -a node_list=()
if [[ -n "$NODES" ]]; then
  read -r -a node_list <<< "$NODES"
elif [[ -n "$NODES_FILE" ]]; then
  [[ -f "$NODES_FILE" ]] || die "nodes file not found: ${NODES_FILE}"
  while IFS= read -r line; do
    line="${line%%#*}"; line="$(echo "$line" | xargs || true)"
    [[ -n "$line" ]] && node_list+=("$line")
  done < "$NODES_FILE"
else
  die "provide --nodes or --nodes-file (try --help)"
fi
[[ ${#node_list[@]} -gt 0 ]] || die "no nodes selected"

ts="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${OUT:-hpc-logs-${ts}.tar.gz}"
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
bundle_root="${workdir}/hpc-logs-${ts}"
mkdir -p "$bundle_root"

manifest="${bundle_root}/MANIFEST.txt"
{
  echo "hpc log bundle"
  echo "generated_utc=${ts}"
  echo "collector_host=$(hostname)"
  echo "remote_log_dir=${REMOTE_LOG}"
  echo "nodes=${node_list[*]}"
  echo
} > "$manifest"

failures=0
for node in "${node_list[@]}"; do
  safe="${node//[^A-Za-z0-9._-]/_}"
  dest="${bundle_root}/${safe}"
  mkdir -p "$dest"
  log "collecting from ${node}"

  if ! ssh -o BatchMode=yes -o ConnectTimeout=10 "$node" 'echo ok' >/dev/null 2>&1; then
    warn "unreachable: ${node}"
    echo "${node}: UNREACHABLE" >> "$manifest"
    failures=$((failures + 1))
    continue
  fi

  # Tar the remote log directory on the node and stream it back.
  if ssh "$node" "tar -czf - -C '${REMOTE_LOG}' . 2>/dev/null" > "${dest}/logs.tar.gz"; then
    ok "  logs ← ${REMOTE_LOG}"
  else
    warn "  could not read ${REMOTE_LOG} on ${node}"
  fi

  # Best-effort live diagnostic bundle if hpc-diag is on the node.
  if ssh "$node" "command -v hpc-diag >/dev/null 2>&1 || [ -x '${REMOTE_LOG}/hpc-diag' ]"; then
    ssh "$node" "(command -v hpc-diag >/dev/null 2>&1 && hpc-diag collect --output - --compact) \
        || '${REMOTE_LOG}/hpc-diag' collect --output - --compact" > "${dest}/diag.json" 2>/dev/null \
      && ok "  diagnostics ← hpc-diag" \
      || warn "  hpc-diag present but collect failed"
  fi

  echo "${node}: collected -> ${safe}/" >> "$manifest"
done

log "packaging bundle"
tar -czf "$OUT" -C "$workdir" "hpc-logs-${ts}"
ok "wrote ${OUT}"
[[ $failures -eq 0 ]] || warn "${failures} node(s) failed — see MANIFEST.txt in the bundle"
