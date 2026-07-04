/*
 * hpc_io.c -- POSIX block-I/O shim implementation.
 *
 * Positioned I/O (pread/pwrite) is used throughout so a descriptor can be
 * shared without a mutable kernel file offset getting in the way, and so the
 * caller controls exactly where each block lands. Short transfers and EINTR
 * are handled here rather than pushed onto the Rust side.
 */
#include "hpc_io.h"

#include <errno.h>
#include <fcntl.h>
#include <unistd.h>

int hpc_open(const char *path, int create) {
    int flags = O_RDWR;
    if (create) {
        flags |= O_CREAT;
    }
    int fd = open(path, flags, 0644);
    if (fd < 0) {
        return -errno;
    }
    return fd;
}

long hpc_read_block(int fd, void *buf, size_t len, int64_t offset) {
    size_t done = 0;
    char *out = (char *)buf;
    while (done < len) {
        ssize_t n = pread(fd, out + done, len - done, (off_t)(offset + (int64_t)done));
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -errno;
        }
        if (n == 0) {
            break; /* end of file */
        }
        done += (size_t)n;
    }
    return (long)done;
}

long hpc_write_block(int fd, const void *buf, size_t len, int64_t offset) {
    size_t done = 0;
    const char *in = (const char *)buf;
    while (done < len) {
        ssize_t n = pwrite(fd, in + done, len - done, (off_t)(offset + (int64_t)done));
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -errno;
        }
        done += (size_t)n;
    }
    return (long)done;
}

int hpc_sync_fs(int fd) {
    if (fsync(fd) < 0) {
        return -errno;
    }
    return 0;
}

int hpc_close(int fd) {
    if (close(fd) < 0) {
        return -errno;
    }
    return 0;
}
