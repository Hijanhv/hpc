# hpc-agent

The per-node agent. A small, resilient gRPC **client** that runs on every
cluster node: it registers with the daemon, streams resource metrics, and
executes the deploy/filesystem commands the daemon pushes to it.

## Session lifecycle

One connected session multiplexes three concerns over a single HTTP/2 channel
(`src/client.rs`):

1. **metrics** — every `metrics_interval`, sample the node and push a
   `MetricsReport`;
2. **heartbeat** — every `heartbeat_interval`, a cheap liveness ping (which can
   receive directives such as `reregister`);
3. **commands** — a long-lived server stream on which the daemon pushes
   `Command`s; each is executed and its `CommandResult` reported back.

Any transport error tears the session down; the agent then reconnects with
capped exponential backoff.

## Metrics collection (`src/metrics.rs`)

Two sources are blended:

- **`sysinfo`** for portable CPU, memory, load average, disk capacity, and the
  static node description (hostname, kernel, OS, core count, local IP).
- **`/proc`** (Linux) for throughput that `sysinfo` doesn't expose as rates:
  per-device read/write bytes from `/proc/diskstats` and network bytes/errors
  from `/proc/net/dev`. These cumulative counters are diffed against the
  previous sample and divided by elapsed wall-clock time.

On non-Linux hosts (e.g. a macOS dev box) the `/proc`-derived rates are simply
zero; everything else still works, so the agent runs anywhere.

## Command execution (`src/executor.rs`)

Filesystem operations are destructive and require root, so the executor is
conservative:

- `allow_exec = false` (**default**) — every command is validated and the exact
  argv that *would* run is logged and returned as a dry-run. Safe on a laptop or
  in CI.
- `allow_exec = true` — the command is actually spawned (`mount`, `umount`,
  `fsck`, `mkfs.<fs>`) and its exit status / stdout / stderr are captured.

The destructive `format` action additionally requires `force = true`.

## Run it

```bash
hpc-agent --config configs/agent.toml
# override just the daemon endpoint:
hpc-agent --endpoint http://daemon-host:7443
```

The node id defaults to the machine hostname when not set in config. See
[`configs/agent.toml`](../configs/agent.toml) for all options.
