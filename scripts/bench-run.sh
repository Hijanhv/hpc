#!/usr/bin/env bash
#
# bench-run.sh — run the hpc I/O benchmark and archive a timestamped result.
#
# Drives the `hpc bench run` CLI against a target directory and writes the JSON
# report into a timestamped folder under the results directory, so runs can be
# compared over time. Optionally also runs the Criterion micro-benchmarks.
#
# Usage:
#   scripts/bench-run.sh [--path DIR] [--file-size BYTES] [--block-size BYTES]
#                        [--fsync] [--out DIR] [--criterion] [-h|--help]
#
# Options:
#   --path DIR         Directory to benchmark (default: ./bench-scratch).
#   --file-size BYTES  Backing file size          (default: 268435456 = 256 MiB).
#   --block-size BYTES I/O block size             (default: 4096).
#   --fsync            fsync after each write (durable-write latency).
#   --out DIR          Results root               (default: ./bench-results).
#   --criterion        Also run `cargo bench -p hpc-bench`.
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

PATH_TARGET="./bench-scratch"
FILE_SIZE=268435456
BLOCK_SIZE=4096
FSYNC=false
OUT_ROOT="./bench-results"
RUN_CRITERION=false
usage() { awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --path) PATH_TARGET="${2:?--path needs a value}"; shift ;;
    --file-size) FILE_SIZE="${2:?}"; shift ;;
    --block-size) BLOCK_SIZE="${2:?}"; shift ;;
    --fsync) FSYNC=true ;;
    --out) OUT_ROOT="${2:?}"; shift ;;
    --criterion) RUN_CRITERION=true ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
  shift
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

hpc_bin="target/release/hpc"
if [[ ! -x "$hpc_bin" ]]; then
  log "building the hpc CLI (release)"
  cargo build --release -p hpc-cli
fi

ts="$(date -u +%Y%m%dT%H%M%SZ)"
out_dir="${OUT_ROOT}/${ts}"
mkdir -p "$out_dir" "$PATH_TARGET"
report="${out_dir}/report.json"

log "benchmarking ${PATH_TARGET} (file=${FILE_SIZE}B block=${BLOCK_SIZE}B fsync=${FSYNC})"
args=(bench run --path "$PATH_TARGET" --file-size "$FILE_SIZE" --block-size "$BLOCK_SIZE" --json "$report")
[[ "$FSYNC" == true ]] && args+=(--fsync)
"./${hpc_bin}" "${args[@]}"

# Record the environment alongside the numbers so results stay interpretable.
{
  echo "timestamp_utc=${ts}"
  echo "host=$(hostname)"
  echo "uname=$(uname -a)"
  echo "file_size=${FILE_SIZE}"
  echo "block_size=${BLOCK_SIZE}"
  echo "fsync=${FSYNC}"
} > "${out_dir}/meta.env"

ok "report written to ${report}"

if [[ "$RUN_CRITERION" == true ]]; then
  log "running Criterion micro-benchmarks"
  cargo bench -p hpc-bench | tee "${out_dir}/criterion.log"
  ok "criterion output saved to ${out_dir}/criterion.log"
fi

log "done — results under ${out_dir}"
