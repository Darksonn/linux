// SPDX-License-Identifier: GPL-2.0

//! User pointers.
//!
//! C header: [`include/linux/uaccess.h`](../../../../include/linux/uaccess.h)

// Comparison with MAX_USER_OP_LEN triggers this lint on platforms
// where `c_ulong == usize`.
#![allow(clippy::absurd_extreme_comparisons)]

use crate::{bindings, error::code::*, error::Result};
use alloc::vec::Vec;
use core::ffi::{c_ulong, c_void};

/// The maximum length of a operation using `copy_[from|to]_user`.
///
/// If a usize is not greater than this constant, then casting it to `c_ulong`
/// is guaranteed to be lossless.
const MAX_USER_OP_LEN: usize = c_ulong::MAX as usize;

/// A pointer to an area in userspace memory, which can be either read-only or
/// read-write.
///
/// All methods on this struct are safe: invalid pointers return `EFAULT`.
/// Concurrent access, *including data races to/from userspace memory*, is
/// permitted, because fundamentally another userspace thread/process could
/// always be modifying memory at the same time (in the same way that userspace
/// Rust's [`std::io`] permits data races with the contents of files on disk).
/// In the presence of a race, the exact byte values read/written are
/// unspecified but the operation is well-defined. Kernelspace code should
/// validate its copy of data after completing a read, and not expect that
/// multiple reads of the same address will return the same value.
///
/// These APIs are designed to make it difficult to accidentally write TOCTOU
/// bugs. Every time you read from a memory location, the pointer is advanced by
/// the length so that you cannot use that reader to read the same memory
/// location twice. Preventing double-fetches avoids TOCTOU bugs. This is
/// accomplished by taking `self` by value to prevent obtaining multiple readers
/// on a given [`UserSlicePtr`], and the readers only permitting forward reads.
/// If double-fetching a memory location is necessary for some reason, then that
/// is done by creating multiple readers to the same memory location, e.g. using
/// [`clone_reader`].
///
/// Constructing a [`UserSlicePtr`] performs no checks on the provided address
/// and length, it can safely be constructed inside a kernel thread with no
/// current userspace process. Reads and writes wrap the kernel APIs
/// `copy_from_user` and `copy_to_user`, which check the memory map of the
/// current process and enforce that the address range is within the user range
/// (no additional calls to `access_ok` are needed).
///
/// [`std::io`]: https://doc.rust-lang.org/std/io/index.html
/// [`clone_reader`]: UserSlicePtrReader::clone_reader
pub struct UserSlicePtr(*mut c_void, usize);

impl UserSlicePtr {
    /// Constructs a user slice from a raw pointer and a length in bytes.
    ///
    /// Callers must be careful to avoid time-of-check-time-of-use
    /// (TOCTOU) issues. The simplest way is to create a single instance of
    /// [`UserSlicePtr`] per user memory block as it reads each byte at
    /// most once.
    pub fn new(ptr: *mut c_void, length: usize) -> Self {
        UserSlicePtr(ptr, length)
    }

    /// Reads the entirety of the user slice.
    ///
    /// Returns `EFAULT` if the address does not currently point to
    /// mapped, readable memory.
    pub fn read_all(self) -> Result<Vec<u8>> {
        self.reader().read_all()
    }

    /// Constructs a [`UserSlicePtrReader`].
    pub fn reader(self) -> UserSlicePtrReader {
        UserSlicePtrReader(self.0, self.1)
    }

    /// Constructs a [`UserSlicePtrWriter`].
    pub fn writer(self) -> UserSlicePtrWriter {
        UserSlicePtrWriter(self.0, self.1)
    }

    /// Constructs both a [`UserSlicePtrReader`] and a [`UserSlicePtrWriter`].
    pub fn reader_writer(self) -> (UserSlicePtrReader, UserSlicePtrWriter) {
        (
            UserSlicePtrReader(self.0, self.1),
            UserSlicePtrWriter(self.0, self.1),
        )
    }
}

/// A reader for [`UserSlicePtr`].
///
/// Used to incrementally read from the user slice.
pub struct UserSlicePtrReader(*mut c_void, usize);

impl UserSlicePtrReader {
    /// Skip the provided number of bytes.
    ///
    /// Returns an error if skipping more than the length of the buffer.
    pub fn skip(&mut self, num_skip: usize) -> Result {
        // Update `self.1` first since that's the fallible one.
        self.1 = self.1.checked_sub(num_skip).ok_or(EFAULT)?;
        self.0 = self.0.wrapping_add(num_skip);
        Ok(())
    }

    /// Create a reader that can access the same range of data.
    ///
    /// Reading from the clone does not advance the current reader.
    ///
    /// The caller should take care to not introduce TOCTOU issues.
    pub fn clone_reader(&self) -> UserSlicePtrReader {
        UserSlicePtrReader(self.0, self.1)
    }

    /// Returns the number of bytes left to be read from this.
    ///
    /// Note that even reading less than this number of bytes may fail.
    pub fn len(&self) -> usize {
        self.1
    }

    /// Returns `true` if no data is available in the io buffer.
    pub fn is_empty(&self) -> bool {
        self.1 == 0
    }

    /// Reads raw data from the user slice into a raw kernel buffer.
    ///
    /// Fails with `EFAULT` if the read encounters a page fault.
    ///
    /// # Safety
    ///
    /// The `out` pointer must be valid for writing `len` bytes.
    pub unsafe fn read_raw(&mut self, out: *mut u8, len: usize) -> Result {
        if len > self.1 || len > MAX_USER_OP_LEN {
            return Err(EFAULT);
        }
        // SAFETY: The caller promises that `out` is valid for writing `len` bytes.
        let res = unsafe { bindings::copy_from_user(out.cast::<c_void>(), self.0, len as c_ulong) };
        if res != 0 {
            return Err(EFAULT);
        }
        // Since this is not a pointer to a valid object in our program,
        // we cannot use `add`, which has C-style rules for defined
        // behavior.
        self.0 = self.0.wrapping_add(len);
        self.1 -= len;
        Ok(())
    }

    /// Reads all remaining data in the buffer into a vector.
    ///
    /// Fails with `EFAULT` if the read encounters a page fault.
    pub fn read_all(&mut self) -> Result<Vec<u8>> {
        let len = self.len();
        let mut data = Vec::<u8>::try_with_capacity(len)?;

        // SAFETY: The output buffer is valid for `len` bytes as we just allocated that much space.
        unsafe { self.read_raw(data.as_mut_ptr(), len)? };

        // SAFETY: Since the call to `read_raw` was successful, the first `len` bytes of the vector
        // have been initialized.
        unsafe { data.set_len(len) };
        Ok(data)
    }
}

/// A writer for [`UserSlicePtr`].
///
/// Used to incrementally write into the user slice.
pub struct UserSlicePtrWriter(*mut c_void, usize);

impl UserSlicePtrWriter {
    /// Returns the amount of space remaining in this buffer.
    ///
    /// Note that even writing less than this number of bytes may fail.
    pub fn len(&self) -> usize {
        self.1
    }

    /// Returns `true` if no more data can be written to this buffer.
    pub fn is_empty(&self) -> bool {
        self.1 == 0
    }

    /// Writes raw data to this user pointer from a raw kernel buffer.
    ///
    /// Fails with `EFAULT` if the write encounters a page fault.
    ///
    /// # Safety
    ///
    /// The `data` pointer must be valid for reading `len` bytes.
    pub unsafe fn write_raw(&mut self, data: *const u8, len: usize) -> Result {
        if len > self.1 || len > MAX_USER_OP_LEN {
            return Err(EFAULT);
        }
        let res = unsafe { bindings::copy_to_user(self.0, data.cast::<c_void>(), len as c_ulong) };
        if res != 0 {
            return Err(EFAULT);
        }
        // Since this is not a pointer to a valid object in our program,
        // we cannot use `add`, which has C-style rules for defined
        // behavior.
        self.0 = self.0.wrapping_add(len);
        self.1 -= len;
        Ok(())
    }

    /// Writes the provided slice to this user pointer.
    ///
    /// Fails with `EFAULT` if the write encounters a page fault.
    pub fn write_slice(&mut self, data: &[u8]) -> Result {
        let len = data.len();
        let ptr = data.as_ptr();
        // SAFETY: The pointer originates from a reference to a slice of length
        // `len`, so the pointer is valid for reading `len` bytes.
        unsafe { self.write_raw(ptr, len) }
    }
}
