/*
 * hpc_io.h -- minimal POSIX block-I/O shim.
 *
 * A deliberately tiny C surface that the Rust `hpc-ffi` crate wraps with a
 * safe API. Every function returns a non-negative value on success and a
 * *negated errno* (`-errno`) on failure, so the Rust side can reconstruct a
 * `std::io::Error` without having to read the C `errno` thread-local across the
 * FFI boundary (which is fragile to do portably).
 */
#ifndef HPC_IO_H
#define HPC_IO_H

#include <stddef.h>
#include <stdint.h>

/*
 * Open `path` for reading and writing. When `create` is non-zero the file is
 * created with mode 0644 if it does not already exist.
 *
 * Returns a file descriptor (>= 0) on success, or -errno on failure.
 */
int hpc_open(const char *path, int create);

/*
 * Read up to `len` bytes at absolute byte `offset` into `buf`, retrying on
 * short reads and EINTR. A return value smaller than `len` indicates
 * end-of-file was reached.
 *
 * Returns the number of bytes read (>= 0) on success, or -errno on failure.
 */
long hpc_read_block(int fd, void *buf, size_t len, int64_t offset);

/*
 * Write exactly `len` bytes from `buf` at absolute byte `offset`, retrying on
 * short writes and EINTR.
 *
 * Returns the number of bytes written (== len) on success, or -errno.
 */
long hpc_write_block(int fd, const void *buf, size_t len, int64_t offset);

/*
 * Flush the file's data and metadata to stable storage (fsync).
 *
 * Returns 0 on success, or -errno on failure.
 */
int hpc_sync_fs(int fd);

/*
 * Close a descriptor previously returned by hpc_open.
 *
 * Returns 0 on success, or -errno on failure.
 */
int hpc_close(int fd);

#endif /* HPC_IO_H */
