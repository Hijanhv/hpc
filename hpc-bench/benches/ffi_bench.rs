//! Criterion micro-benchmark for the raw FFI I/O path.
//!
//! Mirrors `io_bench.rs` but drives `hpc-ffi`'s synchronous C
//! `pread`/`pwrite`/`fsync` shim via [`hpc_bench::ffi_raw`], so the two paths
//! can be compared side by side with `cargo bench -p hpc-bench`. Unwrap/expect
//! is used freely here — this is throwaway benchmark harness code.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use hpc_bench::ffi_raw::{self, RawOptions};

fn bench_ffi(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    const FILE_SIZE: u64 = 1 << 20; // 1 MiB keeps the benchmark quick.

    let mut group = c.benchmark_group("ffi_raw");
    for &block_size in &[4096u64, 65536] {
        group.throughput(Throughput::Bytes(FILE_SIZE));
        group.bench_with_input(
            BenchmarkId::new("write_read", block_size),
            &block_size,
            |b, &block_size| {
                b.iter(|| {
                    let opts = RawOptions {
                        path: dir.path().to_path_buf(),
                        block_size,
                        file_size: FILE_SIZE,
                        fsync: false,
                    };
                    ffi_raw::run(&opts).expect("raw bench run")
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_ffi);
criterion_main!(benches);
