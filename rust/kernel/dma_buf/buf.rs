// SPDX-License-Identifier: GPL-2.0

//! DMA-BUF abstractions.
//!
//! C header: [`include/linux/dma-buf.h`](srctree/include/linux/dma-buf.h)

use crate::{
    bindings,
    device::Device,
    dma,
    error::{from_err_ptr, to_result},
    prelude::*,
    scatterlist::SGTable,
    sync::aref::{ARef, AlwaysRefCounted},
    types::Opaque,
};
use core::ops::Deref;
use core::ptr::NonNull;

/// A wrapper for the kernel's `struct dma_buf`.
///
/// # Invariants
///
/// The pointer is valid and has a non-zero reference count.
#[repr(transparent)]
pub struct DmaBuf {
    opaque: Opaque<bindings::dma_buf>,
}

// SAFETY: `DmaBuf` is reference counted and internally synchronized.
unsafe impl Send for DmaBuf {}
// SAFETY: `DmaBuf` is reference counted and internally synchronized.
unsafe impl Sync for DmaBuf {}

// SAFETY: The reference count is managed by `get_dma_buf` and `dma_buf_put`.
unsafe impl AlwaysRefCounted for DmaBuf {
    fn inc_ref(&self) {
        // SAFETY: `self.opaque.get()` is a valid pointer to `struct dma_buf`.
        unsafe { bindings::get_dma_buf(self.opaque.get()) };
    }

    unsafe fn dec_ref(obj: NonNull<Self>) {
        // SAFETY: The type invariant guarantees that `obj` is valid, and the safety requirement
        // of `dec_ref` guarantees that we own a reference.
        unsafe { bindings::dma_buf_put(obj.as_ptr().cast()) };
    }
}

impl DmaBuf {
    /// Get a `DmaBuf` from a file descriptor.
    pub fn get(fd: i32) -> Result<ARef<DmaBuf>> {
        // SAFETY: `dma_buf_get` is safe to call with any integer.
        // It returns an error pointer if the fd is invalid.
        let ptr = unsafe { bindings::dma_buf_get(fd) };
        let ptr = from_err_ptr(ptr)?;

        let non_null = NonNull::new(ptr).ok_or(EBADF)?;

        // SAFETY: `from_err_ptr` guarantees that `ptr` is not an error pointer.
        // We own the reference count returned by `dma_buf_get` and transfer it to `ARef`.
        Ok(unsafe { ARef::from_raw(non_null.cast()) })
    }

    /// Create a file descriptor for this `DmaBuf`.
    ///
    /// This consumes the `ARef<DmaBuf>` because the file descriptor takes ownership of the
    /// reference.
    pub fn into_fd(this: ARef<Self>, flags: i32) -> Result<i32> {
        let ptr = ARef::into_raw(this);
        // SAFETY: `ptr` is valid as it comes from `ARef`.
        let fd = unsafe { bindings::dma_buf_fd(ptr.as_ptr().cast(), flags) };
        if fd < 0 {
            // SAFETY: `ptr` was returned by `into_raw` and we haven't dropped it yet.
            drop(unsafe { ARef::from_raw(ptr) });
            Err(Error::from_errno(fd))
        } else {
            Ok(fd)
        }
    }

    /// Prepare the buffer for CPU access.
    #[inline]
    pub fn begin_cpu_access(&self, dir: dma::DataDirection) -> Result<CpuAccessGuard<'_>> {
        CpuAccessGuard::new(self, dir)
    }

    /// Attach a device to this `DmaBuf`.
    pub fn attach(&self, dev: &Device) -> Result<DmaBufAttachment> {
        // SAFETY: `self.opaque.get()` is valid. `dev.as_raw()` is valid.
        let ptr = unsafe { bindings::dma_buf_attach(self.opaque.get(), dev.as_raw()) };
        let ptr = from_err_ptr(ptr)?;
        let non_null = NonNull::new(ptr).ok_or(EINVAL)?;

        let dmabuf = ARef::from(self);

        Ok(DmaBufAttachment {
            ptr: non_null,
            dmabuf,
        })
    }

    /// Map the DMA-BUF into kernel virtual address space.
    pub fn vmap(&self) -> Result<DmaBufVmap> {
        let mut map = bindings::iosys_map::default();
        // SAFETY: `self.opaque.get()` is valid. `&mut map` is valid.
        let ret = unsafe { bindings::dma_buf_vmap_unlocked(self.opaque.get(), &mut map) };
        to_result(ret)?;

        let dmabuf = ARef::from(self);

        Ok(DmaBufVmap { dmabuf, map })
    }

    /// Returns the size of the DMA-BUF in bytes.
    #[inline]
    pub fn size(&self) -> usize {
        // SAFETY: `self.opaque.get()` is valid.
        unsafe { (*self.opaque.get()).size }
    }
}

/// A wrapper for the kernel's `struct dma_buf_attachment`.
///
/// # Invariants
///
/// `ptr` is a valid pointer to `struct dma_buf_attachment`.
pub struct DmaBufAttachment {
    ptr: NonNull<bindings::dma_buf_attachment>,
    dmabuf: ARef<DmaBuf>,
}

// SAFETY: `DmaBufAttachment` is safe to send to other threads.
unsafe impl Send for DmaBufAttachment {}
// SAFETY: `DmaBufAttachment` is safe to share between threads.
unsafe impl Sync for DmaBufAttachment {}

impl Drop for DmaBufAttachment {
    fn drop(&mut self) {
        // SAFETY: `self.ptr` is valid.
        unsafe { bindings::dma_buf_detach(self.dmabuf.opaque.get(), self.ptr.as_ptr()) };
    }
}

/// A wrapper for an active DMA-BUF mapping.
///
/// # Invariants
///
/// - `sgt` is a valid pointer to `struct sg_table`.
/// - `sgt` was obtained by mapping `attachment` with direction `dir`.
pub struct DmaBufMapping {
    attachment: DmaBufAttachment,
    sgt: NonNull<bindings::sg_table>,
    dir: dma::DataDirection,
}

// SAFETY: `DmaBufMapping` is safe to send to other threads.
unsafe impl Send for DmaBufMapping {}
// SAFETY: `DmaBufMapping` is safe to share between threads.
unsafe impl Sync for DmaBufMapping {}

impl DmaBufMapping {
    /// Map the attachment.
    pub fn map(attachment: DmaBufAttachment, dir: dma::DataDirection) -> Result<Self> {
        // SAFETY: `attachment.ptr` is valid.
        let sgt = unsafe {
            bindings::dma_buf_map_attachment_unlocked(attachment.ptr.as_ptr(), dir.into())
        };
        let sgt = from_err_ptr(sgt)?;
        let non_null = NonNull::new(sgt).ok_or(EINVAL)?;

        Ok(Self {
            attachment,
            sgt: non_null,
            dir,
        })
    }

    /// Returns the DMA address of the first segment of the mapping.
    ///
    /// This is useful for devices that expect a single contiguous mapping.
    #[inline]
    pub fn iova(&self) -> dma::DmaAddress {
        self.iter()
            .next()
            .map(|entry| entry.dma_address())
            .unwrap_or(0)
    }
}

impl Drop for DmaBufMapping {
    fn drop(&mut self) {
        // SAFETY: `self.attachment.ptr` is valid. `self.sgt` is valid.
        unsafe {
            bindings::dma_buf_unmap_attachment_unlocked(
                self.attachment.ptr.as_ptr(),
                self.sgt.as_ptr(),
                self.dir.into(),
            )
        };
    }
}

impl Deref for DmaBufMapping {
    type Target = SGTable;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: `self.sgt` is valid for the lifetime of `Self`.
        unsafe { SGTable::from_raw(self.sgt.as_ptr()) }
    }
}

/// A wrapper for an active DMA-BUF virtual mapping.
///
/// # Invariants
///
/// `map` is a valid `struct iosys_map` populated by `dma_buf_vmap_unlocked`.
pub struct DmaBufVmap {
    dmabuf: ARef<DmaBuf>,
    map: bindings::iosys_map,
}

// SAFETY: `DmaBufVmap` is safe to send to other threads.
unsafe impl Send for DmaBufVmap {}
// SAFETY: `DmaBufVmap` is safe to share between threads.
unsafe impl Sync for DmaBufVmap {}

impl DmaBufVmap {
    /// Returns the underlying [`DmaBuf`] that this mapping belongs to.
    #[inline]
    pub fn dmabuf(&self) -> &DmaBuf {
        &self.dmabuf
    }

    /// Returns the virtual address of the mapping if it is not in I/O memory.
    #[inline]
    pub fn vaddr(&self) -> Option<NonNull<crate::ffi::c_void>> {
        if self.map.is_iomem {
            None
        } else {
            // SAFETY: `self.map.__bindgen_anon_1.vaddr` is valid if `is_iomem` is false.
            let vaddr = unsafe { self.map.__bindgen_anon_1.vaddr };
            NonNull::new(vaddr)
        }
    }

    /// Copies data from `src` into the buffer at `offset`.
    ///
    /// # Synchronization
    ///
    /// The caller must ensure that CPU access has been properly synchronized by holding
    /// a [`CpuAccessGuard`] for the duration of the copy (obtained via
    /// `self.dmabuf().begin_cpu_access(...)`), otherwise cache coherence issues may occur.
    ///
    /// Returns an error if the source slice does not fit in the buffer at the given offset.
    #[inline]
    pub fn memcpy_to(&self, offset: usize, src: &[u8]) -> Result {
        let end = offset.checked_add(src.len()).ok_or(EINVAL)?;
        if end > self.dmabuf.size() {
            return Err(EINVAL);
        }
        let mut map = self.map;
        // SAFETY: The bounds check ensures we don't write out of bounds.
        // `map` is a valid copy of `self.map`.
        unsafe {
            bindings::iosys_map_memcpy_to(&mut map, offset, src.as_ptr().cast(), src.len());
        }
        Ok(())
    }

    /// Copies data from the buffer at `offset` into `dst`.
    ///
    /// # Synchronization
    ///
    /// The caller must ensure that CPU access has been properly synchronized by holding
    /// a [`CpuAccessGuard`] for the duration of the copy (obtained via
    /// `self.dmabuf().begin_cpu_access(...)`), otherwise cache coherence issues may occur.
    ///
    /// Returns an error if the destination slice does not fit in the buffer at the given offset.
    #[inline]
    pub fn memcpy_from(&self, dst: &mut [u8], offset: usize) -> Result {
        let end = offset.checked_add(dst.len()).ok_or(EINVAL)?;
        if end > self.dmabuf.size() {
            return Err(EINVAL);
        }
        let map = self.map;
        // SAFETY: The bounds check ensures we don't read out of bounds.
        // `map` is a valid copy of `self.map`.
        unsafe {
            bindings::iosys_map_memcpy_from(dst.as_mut_ptr().cast(), &map, offset, dst.len());
        }
        Ok(())
    }
}

impl Drop for DmaBufVmap {
    fn drop(&mut self) {
        // SAFETY: `self.dmabuf.opaque.get()` is valid. `&mut self.map` is valid.
        unsafe { bindings::dma_buf_vunmap_unlocked(self.dmabuf.opaque.get(), &mut self.map) };
    }
}

/// A guard for CPU access to a `DmaBuf`.
///
/// # Invariants
///
/// `dmabuf` is valid and CPU access has been enabled.
pub struct CpuAccessGuard<'a> {
    dmabuf: &'a DmaBuf,
    dir: dma::DataDirection,
}

impl<'a> CpuAccessGuard<'a> {
    fn new(dmabuf: &'a DmaBuf, dir: dma::DataDirection) -> Result<Self> {
        // SAFETY: `dmabuf.opaque.get()` is valid.
        let ret = unsafe { bindings::dma_buf_begin_cpu_access(dmabuf.opaque.get(), dir.into()) };
        to_result(ret)?;
        Ok(Self { dmabuf, dir })
    }

    /// Explicitly end CPU access and check for errors.
    #[inline]
    pub fn end(self) -> Result {
        let this = core::mem::ManuallyDrop::new(self);
        // SAFETY: `this.dmabuf.opaque.get()` is valid.
        let ret =
            unsafe { bindings::dma_buf_end_cpu_access(this.dmabuf.opaque.get(), this.dir.into()) };
        to_result(ret)
    }
}

impl Drop for CpuAccessGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: `self.dmabuf.opaque.get()` is valid.
        unsafe {
            bindings::dma_buf_end_cpu_access(self.dmabuf.opaque.get(), self.dir.into());
        }
    }
}
