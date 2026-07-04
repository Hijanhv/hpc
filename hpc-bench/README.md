# hpc-bench

An async filesystem I/O benchmark suite. It measures the four canonical access
patterns — sequential/random × read/write — over Tokio's async file I/O and
records a full latency distribution per operation with an
[HdrHistogram](https://docs.rs/hdrhistogram), so tail latency (p99, p99.9) is
captured honestly rather than averaged away.

## Two ways to use it

**Programmatically** (this is what `hpc bench run` calls):

```rust
use hpc_bench::{run, BenchOptions};
use hpc_core::types::IoPattern;

let report = run(BenchOptions {
    path: "/mnt/scratch".into(),
    block_size: 4096,
    file_size: 256 * 1024 * 1024,
    patterns: vec![IoPattern::SequentialWrite, IoPattern::RandomRead],
    fsync: false,
}).await?;

for r in &report.results {
    println!("{}: {:.1} MiB/s, p99 {} µs", r.pattern, r.throughput_mib_s, r.latency.p99_us);
}
```

**As Criterion micro-benchmarks:**

```bash
cargo bench -p hpc-bench
```

`benches/io_bench.rs` drives the suite across a couple of block sizes so
Criterion can track throughput and catch regressions.

## What it measures

Each scenario reports throughput (MiB/s), IOPS, and a `LatencyStats` with
min/p50/p90/p99/p99.9/max/mean and the sample count. A single scratch file
(`.hpc-bench.dat`) is created in the target directory, reused across scenarios,
and removed at the end (even on the error path).

## What it does *not* do

I/O is buffered (no `O_DIRECT`), so read scenarios can be served from the page
cache and will overstate a warm-cache workload — this is called out
deliberately. The goal is a portable, dependency-light demonstrator, not a
replacement for `fio`. The write path optionally `fsync`s each operation to
measure durable-write latency (`--fsync`).

## Sample output

```
┌──────────────────┬──────────┬─────────────┬───────┬───────┬───────┬───────┬───────┐
│ PATTERN          ┆ BLOCK    ┆ THROUGHPUT  ┆ IOPS  ┆ p50   ┆ p99   ┆ p99.9 ┆ max   │
╞══════════════════╪══════════╪═════════════╪═══════╪═══════╪═══════╪═══════╪═══════╡
│ sequential_write ┆ 4.00 KiB ┆ 138.4 MiB/s ┆ 35422 ┆ 2 µs  ┆ 5 µs  ┆ 7 µs  ┆ 7 µs  │
│ sequential_read  ┆ 4.00 KiB ┆ 185.4 MiB/s ┆ 47466 ┆ 10 µs ┆ 16 µs ┆ 96 µs ┆ 96 µs │
│ random_read      ┆ 4.00 KiB ┆ 188.4 MiB/s ┆ 48220 ┆ 10 µs ┆ 13 µs ┆ 15 µs ┆ 15 µs │
└──────────────────┴──────────┴─────────────┴───────┴───────┴───────┴───────┴───────┘
```
