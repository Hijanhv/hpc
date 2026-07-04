# hpc-cli

The operator command-line interface (installed binary name: **`hpc`**). It talks
to a running `hpc-daemon` over its REST API for `node` and `fs` commands, and
drives the `hpc-bench` suite directly for `bench`.

Built with `clap` (derive). Output is rendered as clean UTF-8 tables via
`comfy-table`.

## Subcommands

```
hpc [--api URL] <command>

node list                 List all nodes with summarised health.
node status <id>          Detailed node view + latest metrics + disks.
node deploy <id> --component X [--version V] [--action install|upgrade|rollback|remove]
                          [--target-path P] [--opt k=v ...]

fs mount <id> --device D --mount-point M [--fs-type T] [--opt OPT ...]
fs unmount <id> --mount-point M
fs status <id>            Show the filesystems/disks a node reports.

bench run [--path P] [--block-size B] [--file-size S]
          [--pattern seq-write,seq-read,rand-write,rand-read] [--fsync] [--json OUT]
bench report <file.json>  Pretty-print a saved report.
```

`--api` (or the `HPC_API` env var) points at the daemon REST endpoint and
defaults to `http://127.0.0.1:8080`.

## Examples

```bash
export HPC_API=http://127.0.0.1:8080

hpc node list
hpc node status storage-01
hpc node deploy storage-01 --component lustre-ost --version 2.15 --opt stripe=4

hpc fs mount storage-01 --device /dev/sdb1 --mount-point /mnt/scratch --fs-type xfs --opt noatime
hpc fs status storage-01

hpc bench run --path /mnt/scratch --file-size 268435456 --json report.json
hpc bench report report.json
```

Command dispatch is asynchronous: `deploy`/`mount`/`unmount` return a command id
immediately; the outcome is executed by the agent and recorded in the daemon's
audit log (`GET /api/v1/outcomes`).

## Notes

- Non-2xx responses are surfaced with the daemon's structured `{ "error": … }`
  message, not a bare status code.
- The CLI initialises `tracing` at `warn` by default (raise with `RUST_LOG`) so
  normal output stays clean.
