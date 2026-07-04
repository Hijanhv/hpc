//! # hpc-bench
//!
//! An async filesystem I/O benchmark suite. It measures four canonical access
//! patterns — sequential/random × read/write — over Tokio's async file I/O,
//! recording a full latency distribution per operation with an
//! [HdrHistogram](hdrhistogram) so tail latency (p99, p99.9) is captured
//! honestly rather than averaged away.
//!
//! The suite is used two ways:
//! * programmatically via [`run`], which the `hpc` CLI calls for `bench run`, and
//! * through Criterion micro-benchmarks in `benches/io_bench.rs`.
//!
//! ## What it does *not* do
//! I/O is buffered (no `O_DIRECT`), so read scenarios can be served from the
//! page cache and will overstate a warm-cache workload. This is called out
//! deliberately: the goal is a portable, dependency-light demonstrator, not a
//! substitute for `fio`. The write path optionally `fsync`s to measure durable
//! write latency.
#![forbid(unsafe_code)]

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::time::Instant;

use hdrhistogram::Histogram;
use hpc_core::error::{HpcError, Result};
use hpc_core::types::{now_unix, BenchReport, BenchResult, IoPattern, LatencyStats};
use rand::Rng;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// Largest latency the histogram tracks: 60 s in microseconds.
const MAX_LATENCY_US: u64 = 60_000_000;
/// Chunk size used when preparing the backing file (unmeasured).
const PREP_CHUNK: usize = 1 << 20; // 1 MiB

/// Parameters for a benchmark run.
#[derive(Debug, Clone)]
pub struct BenchOptions {
    /// Directory to create the scratch file in.
    pub path: PathBuf,
    /// I/O block size in bytes.
    pub block_size: u64,
    /// Backing file size in bytes.
    pub file_size: u64,
    /// Which access patterns to run.
    pub patterns: Vec<IoPattern>,
    /// Whether to `fsync` after each write op (durable-write latency).
    pub fsync: bool,
}

impl Default for BenchOptions {
    fn default() -> Self {
        BenchOptions {
            path: PathBuf::from("."),
            block_size: 4096,
            file_size: 64 * 1024 * 1024,
            patterns: vec![
                IoPattern::SequentialWrite,
                IoPattern::SequentialRead,
                IoPattern::RandomWrite,
                IoPattern::RandomRead,
            ],
            fsync: false,
        }
    }
}

impl BenchOptions {
    fn validate(&self) -> Result<()> {
        if self.block_size == 0 {
            return Err(HpcError::Bench("block_size must be non-zero".into()));
        }
        if self.file_size < self.block_size {
            return Err(HpcError::Bench(
                "file_size must be at least block_size".into(),
            ));
        }
        Ok(())
    }

    /// Number of whole blocks that fit in the backing file.
    fn blocks(&self) -> u64 {
        (self.file_size / self.block_size).max(1)
    }
}

/// Run every requested scenario and return a consolidated report.
///
/// A single scratch file (`.hpc-bench.dat`) is created in `path`, reused across
/// scenarios, and removed at the end (even on the error path).
pub async fn run(opts: BenchOptions) -> Result<BenchReport> {
    opts.validate()?;
    tokio::fs::create_dir_all(&opts.path)
        .await
        .map_err(|e| HpcError::io_at(&opts.path, e))?;
    let file_path = opts.path.join(".hpc-bench.dat");

    // Prepare the backing file so reads have data and random writes have room.
    prepare_file(&file_path, opts.file_size).await?;

    let mut results = Vec::with_capacity(opts.patterns.len());
    for pattern in &opts.patterns {
        tracing::info!(?pattern, "running scenario");
        let result = run_pattern(&file_path, *pattern, &opts).await;
        match result {
            Ok(r) => results.push(r),
            Err(e) => {
                // Best-effort cleanup before surfacing the failure.
                let _ = tokio::fs::remove_file(&file_path).await;
                return Err(e);
            }
        }
    }

    let _ = tokio::fs::remove_file(&file_path).await;

    Ok(BenchReport {
        target_path: opts.path.display().to_string(),
        started_at_unix: now_unix(),
        results,
    })
}

/// Write a zero-filled file of exactly `size` bytes (throughput not measured).
async fn prepare_file(path: &Path, size: u64) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .await
        .map_err(|e| HpcError::io_at(path, e))?;
    let chunk = vec![0u8; PREP_CHUNK];
    let mut remaining = size;
    while remaining > 0 {
        let n = remaining.min(PREP_CHUNK as u64) as usize;
        file.write_all(&chunk[..n])
            .await
            .map_err(|e| HpcError::io_at(path, e))?;
        remaining -= n as u64;
    }
    file.flush().await.map_err(|e| HpcError::io_at(path, e))?;
    file.sync_all()
        .await
        .map_err(|e| HpcError::io_at(path, e))?;
    Ok(())
}

async fn run_pattern(path: &Path, pattern: IoPattern, opts: &BenchOptions) -> Result<BenchResult> {
    match pattern {
        IoPattern::SequentialWrite => write_scenario(path, opts, false).await,
        IoPattern::RandomWrite => write_scenario(path, opts, true).await,
        IoPattern::SequentialRead => read_scenario(path, opts, false).await,
        IoPattern::RandomRead => read_scenario(path, opts, true).await,
    }
}

/// Shared write path for sequential and random writes.
async fn write_scenario(path: &Path, opts: &BenchOptions, random: bool) -> Result<BenchResult> {
    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .open(path)
        .await
        .map_err(|e| HpcError::io_at(path, e))?;

    let block = fill_block(opts.block_size as usize);
    let blocks = opts.blocks();
    let mut hist = new_histogram()?;
    let mut rng = rand::rng();

    let started = Instant::now();
    for i in 0..blocks {
        let offset = block_offset(i, blocks, random, &mut rng, opts.block_size);
        seek(&mut file, offset, path).await?;

        let op = Instant::now();
        file.write_all(&block)
            .await
            .map_err(|e| HpcError::io_at(path, e))?;
        if opts.fsync {
            file.sync_data()
                .await
                .map_err(|e| HpcError::io_at(path, e))?;
        }
        record(&mut hist, op.elapsed());
    }
    file.flush().await.map_err(|e| HpcError::io_at(path, e))?;
    let elapsed = started.elapsed().as_secs_f64();

    Ok(summarise(
        if random {
            IoPattern::RandomWrite
        } else {
            IoPattern::SequentialWrite
        },
        opts.block_size,
        blocks,
        elapsed,
        &hist,
    ))
}

/// Shared read path for sequential and random reads.
async fn read_scenario(path: &Path, opts: &BenchOptions, random: bool) -> Result<BenchResult> {
    let mut file = File::open(path)
        .await
        .map_err(|e| HpcError::io_at(path, e))?;

    let mut buf = vec![0u8; opts.block_size as usize];
    let blocks = opts.blocks();
    let mut hist = new_histogram()?;
    let mut rng = rand::rng();

    let started = Instant::now();
    for i in 0..blocks {
        let offset = block_offset(i, blocks, random, &mut rng, opts.block_size);
        seek(&mut file, offset, path).await?;

        let op = Instant::now();
        file.read_exact(&mut buf)
            .await
            .map_err(|e| HpcError::io_at(path, e))?;
        record(&mut hist, op.elapsed());
    }
    let elapsed = started.elapsed().as_secs_f64();

    Ok(summarise(
        if random {
            IoPattern::RandomRead
        } else {
            IoPattern::SequentialRead
        },
        opts.block_size,
        blocks,
        elapsed,
        &hist,
    ))
}

fn block_offset(i: u64, blocks: u64, random: bool, rng: &mut impl Rng, block_size: u64) -> u64 {
    let index = if random {
        rng.random_range(0..blocks)
    } else {
        i
    };
    index * block_size
}

async fn seek(file: &mut File, offset: u64, path: &Path) -> Result<()> {
    file.seek(SeekFrom::Start(offset))
        .await
        .map(|_| ())
        .map_err(|e| HpcError::io_at(path, e))
}

fn fill_block(size: usize) -> Vec<u8> {
    // A fixed non-zero pattern; avoids any sparse-file optimisation on writes.
    (0..size).map(|i| (i % 251) as u8).collect()
}

fn new_histogram() -> Result<Histogram<u64>> {
    Histogram::new_with_bounds(1, MAX_LATENCY_US, 3)
        .map_err(|e| HpcError::Bench(format!("histogram init: {e}")))
}

fn record(hist: &mut Histogram<u64>, elapsed: std::time::Duration) {
    let us = elapsed.as_micros().min(MAX_LATENCY_US as u128) as u64;
    // saturating_record never errors and clamps to the tracked range.
    hist.saturating_record(us.max(1));
}

fn summarise(
    pattern: IoPattern,
    block_size: u64,
    blocks: u64,
    elapsed: f64,
    hist: &Histogram<u64>,
) -> BenchResult {
    let total_bytes = blocks * block_size;
    let throughput_mib_s = if elapsed > 0.0 {
        (total_bytes as f64 / (1024.0 * 1024.0)) / elapsed
    } else {
        0.0
    };
    let iops = if elapsed > 0.0 {
        blocks as f64 / elapsed
    } else {
        0.0
    };
    BenchResult {
        pattern,
        block_size_bytes: block_size,
        total_bytes,
        duration_secs: elapsed,
        throughput_mib_s,
        iops,
        latency: LatencyStats {
            min_us: hist.min(),
            p50_us: hist.value_at_quantile(0.50),
            p90_us: hist.value_at_quantile(0.90),
            p99_us: hist.value_at_quantile(0.99),
            p999_us: hist.value_at_quantile(0.999),
            max_us: hist.max(),
            mean_us: hist.mean() as u64,
            samples: hist.len(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn small_run_produces_all_results() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opts = BenchOptions {
            path: dir.path().to_path_buf(),
            block_size: 4096,
            file_size: 256 * 1024,
            patterns: vec![
                IoPattern::SequentialWrite,
                IoPattern::SequentialRead,
                IoPattern::RandomWrite,
                IoPattern::RandomRead,
            ],
            fsync: false,
        };
        let report = run(opts).await.expect("bench run");
        assert_eq!(report.results.len(), 4);
        for r in &report.results {
            assert!(r.latency.samples > 0);
            assert!(r.total_bytes > 0);
        }
        // Scratch file must be cleaned up.
        assert!(!dir.path().join(".hpc-bench.dat").exists());
    }

    #[test]
    fn rejects_zero_block_size() {
        let opts = BenchOptions {
            block_size: 0,
            ..Default::default()
        };
        assert!(opts.validate().is_err());
    }
}
