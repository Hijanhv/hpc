//! Synchronous raw block-I/O benchmark backed by [`hpc_ffi`].
//!
//! Where [`crate::run`] measures Tokio's *buffered async* file I/O, this path
//! drives the C `pread`/`pwrite`/`fsync` shim in `hpc-ffi` directly and
//! synchronously, reusing this crate's HdrHistogram summarisation so the
//! results are directly comparable. The raw positioned-I/O path is closer to
//! what a storage engine's hot loop actually does, which is why it is worth
//! measuring alongside the async path.

use std::path::PathBuf;
use std::time::Instant;

use hdrhistogram::Histogram;
use hpc_core::error::{HpcError, Result};
use hpc_core::types::{BenchResult, IoPattern};
use hpc_ffi::BlockFile;

use crate::{new_histogram, record, summarise};

/// Parameters for a raw FFI benchmark run.
#[derive(Debug, Clone)]
pub struct RawOptions {
    /// Directory to create the scratch file in.
    pub path: PathBuf,
    /// I/O block size in bytes.
    pub block_size: u64,
    /// Backing file size in bytes.
    pub file_size: u64,
    /// Whether to `fsync` after each write (durable-write latency).
    pub fsync: bool,
}

impl Default for RawOptions {
    fn default() -> Self {
        RawOptions {
            path: PathBuf::from("."),
            block_size: 4096,
            file_size: 16 * 1024 * 1024,
            fsync: false,
        }
    }
}

/// Sequentially write then read back `file_size` bytes through the C shim,
/// returning one [`BenchResult`] per phase (write, then read).
///
/// A single scratch file (`.hpc-bench-ffi.dat`) is created in `path`, driven
/// for both phases, and removed at the end (even on the error path).
pub fn run(opts: &RawOptions) -> Result<Vec<BenchResult>> {
    if opts.block_size == 0 {
        return Err(HpcError::Bench("block_size must be non-zero".into()));
    }
    if opts.file_size < opts.block_size {
        return Err(HpcError::Bench(
            "file_size must be at least block_size".into(),
        ));
    }
    std::fs::create_dir_all(&opts.path).map_err(|e| HpcError::io_at(&opts.path, e))?;
    let file = opts.path.join(".hpc-bench-ffi.dat");

    let outcome = run_inner(&file, opts);
    // Best-effort cleanup regardless of success.
    let _ = std::fs::remove_file(&file);
    outcome
}

fn run_inner(file: &std::path::Path, opts: &RawOptions) -> Result<Vec<BenchResult>> {
    let handle = BlockFile::open(file, true)?;
    let blocks = (opts.file_size / opts.block_size).max(1);
    let block: Vec<u8> = (0..opts.block_size as usize)
        .map(|i| (i % 251) as u8)
        .collect();

    // Write phase.
    let write = phase(
        IoPattern::SequentialWrite,
        opts.block_size,
        blocks,
        |i, hist| {
            let offset = i * opts.block_size;
            let op = Instant::now();
            handle.write_block_at(&block, offset)?;
            if opts.fsync {
                handle.sync()?;
            }
            record(hist, op.elapsed());
            Ok(())
        },
    )?;
    handle.sync()?;

    // Read phase over the freshly written file.
    let mut buf = vec![0u8; opts.block_size as usize];
    let read = phase(
        IoPattern::SequentialRead,
        opts.block_size,
        blocks,
        |i, hist| {
            let offset = i * opts.block_size;
            let op = Instant::now();
            handle.read_block_at(&mut buf, offset)?;
            record(hist, op.elapsed());
            Ok(())
        },
    )?;

    Ok(vec![write, read])
}

/// Run `blocks` iterations of `op`, timing the whole loop and summarising the
/// per-op histogram into a [`BenchResult`].
fn phase(
    pattern: IoPattern,
    block_size: u64,
    blocks: u64,
    mut op: impl FnMut(u64, &mut Histogram<u64>) -> Result<()>,
) -> Result<BenchResult> {
    let mut hist = new_histogram()?;
    let started = Instant::now();
    for i in 0..blocks {
        op(i, &mut hist)?;
    }
    let elapsed = started.elapsed().as_secs_f64();
    Ok(summarise(pattern, block_size, blocks, elapsed, &hist))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_run_produces_write_and_read_results() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opts = RawOptions {
            path: dir.path().to_path_buf(),
            block_size: 4096,
            file_size: 256 * 1024,
            fsync: false,
        };
        let results = run(&opts).expect("raw run");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].pattern, IoPattern::SequentialWrite);
        assert_eq!(results[1].pattern, IoPattern::SequentialRead);
        for r in &results {
            assert!(r.latency.samples > 0);
            assert!(r.total_bytes > 0);
        }
        // Scratch file must be cleaned up.
        assert!(!dir.path().join(".hpc-bench-ffi.dat").exists());
    }

    #[test]
    fn rejects_zero_block_size() {
        let opts = RawOptions {
            block_size: 0,
            ..Default::default()
        };
        assert!(run(&opts).is_err());
    }
}
