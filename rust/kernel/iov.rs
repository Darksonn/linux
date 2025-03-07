// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2025 Google LLC.

//! IO vectors.
//!
//! C headers: [`include/linux/iov_iter.h`](srctree/include/linux/iov_iter.h),
//! [`include/linux/uio.h`](srctree/include/linux/uio.h)

use crate::{bindings, prelude::*, types::Opaque};
use core::{marker::PhantomData, mem::MaybeUninit, slice};

const ITER_SOURCE: bool = bindings::ITER_SOURCE != 0;
const ITER_DEST: bool = bindings::ITER_DEST != 0;

// Compile-time assertion for the above constants.
const _: () = {
    if ITER_SOURCE == ITER_DEST {
        panic!("ITER_DEST and ITER_SOURCE should be different.");
    }
};

/// An IO vector that acts as a source of data.
///
/// # Invariants
///
/// Must hold a valid `struct iov_iter` with `data_source` set to `ITER_SOURCE`. The buffers
/// referenced by the IO vector must be valid for reading for the duration of `'data`.
///
/// Note that if the IO vector is backed by a userspace pointer, it is always considered valid for
/// reading.
#[repr(transparent)]
pub struct IovIterSource<'data> {
    iov: Opaque<bindings::iov_iter>,
    /// Represent to the type system that this value contains a pointer to readable data it does
    /// not own.
    _source: PhantomData<&'data [u8]>,
}

// SAFETY: This struct is essentially just a fancy `std::io::Cursor<&[u8]>`, and that type is safe
// to send across thread boundaries.
unsafe impl<'data> Send for IovIterSource<'data> {}
// SAFETY: This struct is essentially just a fancy `std::io::Cursor<&[u8]>`, and that type is safe
// to share across thread boundaries.
unsafe impl<'data> Sync for IovIterSource<'data> {}

impl<'data> IovIterSource<'data> {
    /// Obtain an `IovIterSource` from a raw pointer.
    ///
    /// # Safety
    ///
    /// * For the duration of `'iov`, the `struct iov_iter` must remain valid and must not be
    ///   accessed except through the returned reference.
    /// * For the duration of `'data`, the buffers backing this IO vector must be valid for
    ///   reading.
    #[track_caller]
    #[inline]
    pub unsafe fn from_raw<'iov>(ptr: *mut bindings::iov_iter) -> &'iov mut IovIterSource<'data> {
        // SAFETY: The caller ensures that `ptr` is valid.
        let data_source = unsafe { (*ptr).data_source };
        assert_eq!(data_source, ITER_SOURCE);

        // SAFETY: The caller ensures the struct invariants for the right durations.
        unsafe { &mut *ptr.cast::<IovIterSource<'data>>() }
    }

    /// Access this as a raw `struct iov_iter`.
    #[inline]
    pub fn as_raw(&mut self) -> *mut bindings::iov_iter {
        self.iov.get()
    }

    /// Returns the number of bytes available in this IO vector.
    ///
    /// Note that this may overestimate the number of bytes. For example, reading from userspace
    /// memory could fail with EFAULT, which will be treated as the end of the IO vector.
    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: It is safe to access the `count` field.
        unsafe {
            (*self.iov.get())
                .__bindgen_anon_1
                .__bindgen_anon_1
                .as_ref()
                .count
        }
    }

    /// Returns whether there are any bytes left in this IO vector.
    ///
    /// This may return `true` even if there are no more bytes available. For example, reading from
    /// userspace memory could fail with EFAULT, which will be treated as the end of the IO vector.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Advance this IO vector by `bytes` bytes.
    ///
    /// If `bytes` is larger than the size of this IO vector, it is advanced to the end.
    #[inline]
    pub fn advance(&mut self, bytes: usize) {
        // SAFETY: `self.iov` is a valid IO vector.
        unsafe { bindings::iov_iter_advance(self.as_raw(), bytes) };
    }

    /// Advance this IO vector backwards by `bytes` bytes.
    ///
    /// # Safety
    ///
    /// The IO vector must not be reverted to before its beginning.
    #[inline]
    pub unsafe fn revert(&mut self, bytes: usize) {
        // SAFETY: `self.iov` is a valid IO vector, and `bytes` is in bounds.
        unsafe { bindings::iov_iter_revert(self.as_raw(), bytes) };
    }

    /// Read data from this IO vector.
    ///
    /// Returns the number of bytes that have been copied.
    #[inline]
    pub fn copy_from_iter(&mut self, out: &mut [u8]) -> usize {
        // SAFETY: We will not write uninitialized bytes to `out`.
        let out = unsafe { &mut *(out as *mut [u8] as *mut [MaybeUninit<u8>]) };

        self.copy_from_iter_raw(out).len()
    }

    /// Read data from this IO vector and append it to a vector.
    ///
    /// Returns the number of bytes that have been copied.
    #[inline]
    pub fn copy_from_iter_vec<A: Allocator>(
        &mut self,
        out: &mut Vec<u8, A>,
        flags: Flags,
    ) -> Result<usize> {
        out.reserve(self.len(), flags)?;
        let len = self.copy_from_iter_raw(out.spare_capacity_mut()).len();
        // SAFETY: The next `len` bytes of the vector have been initialized.
        unsafe { out.set_len(out.len() + len) };
        Ok(len)
    }

    /// Read data from this IO vector into potentially uninitialized memory.
    ///
    /// Returns the sub-slice of the output that has been initialized. If the returned slice is
    /// shorter than the input buffer, then the entire IO vector has been read.
    #[inline]
    pub fn copy_from_iter_raw(&mut self, out: &mut [MaybeUninit<u8>]) -> &mut [u8] {
        // SAFETY: `out` is valid for `out.len()` bytes.
        let len =
            unsafe { bindings::_copy_from_iter(out.as_mut_ptr().cast(), out.len(), self.as_raw()) };

        // SAFETY: We just initialized the first `len` bytes of `out`.
        unsafe { slice::from_raw_parts_mut(out.as_mut_ptr().cast(), len) }
    }
}

impl<'data> Clone for IovIterSource<'data> {
    #[inline]
    fn clone(&self) -> IovIterSource<'data> {
        // SAFETY: This duplicates the bytes inside the `Opaque` value exactly. Since `struct
        // iov_iter` does not have any internal self references, that is okay.
        //
        // Since this IO vector only reads from the backing buffers, having multiple IO vectors to
        // the same source can't lead to data races on the backing buffers.
        unsafe { core::ptr::read(self) }
    }
}
