//! Criterion micro-benchmarks for the I/O suite.
//!
//! Run with `cargo bench -p hpc-bench`. These drive [`hpc_bench::run`] on a
//! small backing file across a couple of block sizes so Criterion can track
//! throughput and detect regressions over time. Unwrap/expect is used freely
//! here — this is throwaway benchmark harness code, not library code.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use hpc_bench::BenchOptions;
use hpc_core::types::IoPattern;

fn bench_io(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let dir = tempfile::tempdir().expect("tempdir");
    const FILE_SIZE: u64 = 1 << 20; // 1 MiB keeps the benchmark quick.

    let mut group = c.benchmark_group("io");
    for &block_size in &[4096u64, 65536] {
        group.throughput(Throughput::Bytes(FILE_SIZE));

        for (label, pattern) in [
            ("seq_write", IoPattern::SequentialWrite),
            ("seq_read", IoPattern::SequentialRead),
            ("rand_read", IoPattern::RandomRead),
        ] {
            group.bench_with_input(
                BenchmarkId::new(label, block_size),
                &block_size,
                |b, &block_size| {
                    b.to_async(&rt).iter(|| {
                        let path = dir.path().to_path_buf();
                        async move {
                            let opts = BenchOptions {
                                path,
                                block_size,
                                file_size: FILE_SIZE,
                                patterns: vec![pattern],
                                fsync: false,
                            };
                            hpc_bench::run(opts).await.expect("bench run")
                        }
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, bench_io);
criterion_main!(benches);
