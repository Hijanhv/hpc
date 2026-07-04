//! # hpc-ffi
//!
//! The workspace's C-interop layer. It wraps a tiny native shim
//! (`src/hpc_io.c`) that performs raw POSIX block I/O — `open`, `pread`,
//! `pwrite`, `fsync` — behind a safe, idiomatic Rust API. The C declarations
//! are turned into Rust `extern "C"` signatures by **bindgen** at build time
//! and the shim is compiled and statically linked by the **cc** crate (see
//! [`build.rs`](../build.rs)).
//!
//! This is deliberately the *one* crate in the workspace that contains
//! `unsafe`. It exists to demonstrate a real FFI bridge and to give
//! [`hpc-bench`](../hpc-bench) a synchronous, positioned raw-I/O path to
//! contrast with Tokio's buffered async I/O. Every `unsafe` block is small,
//! justified with a `// SAFETY:` comment, and wrapped so no `unsafe` escapes
//! the crate boundary; the rest of the workspace keeps `#![forbid(unsafe_code)]`.
//!
//! ## Error model
//!
//! The C shim returns a negated errno (`-errno`) on failure. The wrappers turn
//! that back into a [`std::io::Error`] via
//! [`std::io::Error::from_raw_os_error`] and surface it as
//! [`HpcError::Ffi`](hpc_core::error::HpcError::Ffi), so callers get the same
//! `hpc_core::Result` they get everywhere else.
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

use std::ffi::CString;
use std::os::raw::c_int;
use std::path::Path;

use hpc_core::error::{HpcError, Result};

/// The bindgen-generated `extern "C"` declarations for `hpc_io.h`. Kept in a
/// private module so the raw, `unsafe` surface never leaks to callers.
mod sys {
    #![allow(
        non_upper_case_globals,
        non_camel_case_types,
        non_snake_case,
        dead_code,
        missing_docs
    )]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

/// Convert a negated-errno return value from the C shim into an [`HpcError`].
///
/// Generic over the return width so callers can pass the raw `c_int` or
/// `c_long` result directly without a cast (which keeps it portable and free of
/// spurious `useless_conversion`/`unnecessary_cast` lints).
fn errno_error(context: &str, ret: impl Into<i64>) -> HpcError {
    let os = std::io::Error::from_raw_os_error((-ret.into()) as i32);
    HpcError::Ffi(format!("{context}: {os}"))
}

/// An open file handle backed by the native POSIX I/O shim.
///
/// All I/O is *positioned* (`pread`/`pwrite`): calls take an explicit byte
/// offset and never touch a shared kernel file offset, which is why the read
/// and write methods only need `&self`. The descriptor is closed on drop.
#[derive(Debug)]
pub struct BlockFile {
    fd: c_int,
}

impl BlockFile {
    /// Open `path` for read + write, creating it (mode 0644) when `create` is
    /// set and the file is absent.
    pub fn open(path: impl AsRef<Path>, create: bool) -> Result<Self> {
        let path = path.as_ref();
        let c_path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| HpcError::Ffi(format!("path contains NUL byte: {}", path.display())))?;
        // SAFETY: `c_path` is a valid, NUL-terminated C string that outlives
        // the call; the shim only reads it and returns an fd or a negated
        // errno. No Rust invariants are involved.
        let ret = unsafe { sys::hpc_open(c_path.as_ptr(), c_int::from(create)) };
        if ret < 0 {
            return Err(errno_error(&format!("open {}", path.display()), ret));
        }
        Ok(BlockFile { fd: ret })
    }

    /// Read up to `buf.len()` bytes starting at `offset`. The returned count is
    /// smaller than `buf.len()` only when end-of-file is reached.
    pub fn read_block_at(&self, buf: &mut [u8], offset: u64) -> Result<usize> {
        // SAFETY: `buf` is valid for writes of exactly `buf.len()` bytes and the
        // shim never writes past `len`; `self.fd` is an open descriptor for the
        // lifetime of `self`.
        let ret = unsafe {
            sys::hpc_read_block(self.fd, buf.as_mut_ptr().cast(), buf.len(), offset as i64)
        };
        if ret < 0 {
            return Err(errno_error("read_block", ret));
        }
        Ok(ret as usize)
    }

    /// Write all of `buf` starting at `offset`, returning the number of bytes
    /// written (always `buf.len()` on success).
    pub fn write_block_at(&self, buf: &[u8], offset: u64) -> Result<usize> {
        // SAFETY: `buf` is valid for reads of exactly `buf.len()` bytes and the
        // shim only reads from it; `self.fd` is an open descriptor for the
        // lifetime of `self`.
        let ret =
            unsafe { sys::hpc_write_block(self.fd, buf.as_ptr().cast(), buf.len(), offset as i64) };
        if ret < 0 {
            return Err(errno_error("write_block", ret));
        }
        Ok(ret as usize)
    }

    /// Flush data and metadata to stable storage (`fsync`).
    pub fn sync(&self) -> Result<()> {
        // SAFETY: `self.fd` is an open descriptor; the shim only fsyncs it.
        let ret = unsafe { sys::hpc_sync_fs(self.fd) };
        if ret < 0 {
            return Err(errno_error("sync_fs", ret));
        }
        Ok(())
    }
}

impl Drop for BlockFile {
    fn drop(&mut self) {
        // SAFETY: `self.fd` was returned by `hpc_open` and has not been closed
        // elsewhere (there is no other code path that closes it). The result is
        // ignored on the drop path, matching std's `File` behaviour.
        unsafe {
            let _ = sys::hpc_close(self.fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("blk.dat");
        let f = BlockFile::open(&path, true).expect("open");

        let data = b"hpc-ffi positioned block roundtrip";
        let written = f.write_block_at(data, 0).expect("write");
        assert_eq!(written, data.len());
        f.sync().expect("sync");

        let mut buf = vec![0u8; data.len()];
        let read = f.read_block_at(&mut buf, 0).expect("read");
        assert_eq!(read, data.len());
        assert_eq!(&buf, data);
    }

    #[test]
    fn positioned_offset_is_honoured() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("blk2.dat");
        let f = BlockFile::open(&path, true).expect("open");

        f.write_block_at(&[0xAB; 8], 4096).expect("write at offset");
        let mut buf = [0u8; 8];
        let read = f.read_block_at(&mut buf, 4096).expect("read at offset");
        assert_eq!(read, 8);
        assert_eq!(buf, [0xAB; 8]);

        // The gap before the offset must read back as zeroes, not our pattern.
        let mut head = [0xFFu8; 8];
        f.read_block_at(&mut head, 0).expect("read head");
        assert_eq!(head, [0u8; 8]);
    }

    #[test]
    fn opening_absent_file_without_create_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.dat");
        let err = BlockFile::open(&path, false).expect_err("must fail");
        assert!(matches!(err, HpcError::Ffi(_)));
    }
}
