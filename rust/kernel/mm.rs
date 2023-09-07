// SPDX-License-Identifier: GPL-2.0

//! Memory management.
//!
//! C header: [`include/linux/mm.h`](../../../../include/linux/mm.h)

use crate::{
    bindings,
    error::{to_result, Result},
    pages,
    types::Opaque,
};

use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
    ptr::{self, NonNull},
};

/// A reference to a `struct mm` that holds a reference using `mmgrab`.
pub struct MmGrab {
    mm: NonNull<bindings::mm_struct>,
}

impl MmGrab {
    /// Call `mmgrab` on `current.mm`.
    pub fn mmgrab_current() -> Option<Self> {
        // SAFETY: It's safe to get the `mm` field from current.
        let mm = unsafe {
            let current = bindings::get_current();
            (*current).mm
        };

        let mm = NonNull::new(mm)?;

        // SAFETY: We just checked that `mm` is not null.
        unsafe { bindings::mmgrab(mm.as_ptr()) };

        Some(Self { mm })
    }

    /// Check whether this vma is associated with this mm.
    pub fn is_same_mm(&self, area: &virt::Area) -> bool {
        let area = area as *const virt::Area as *const bindings::vm_area_struct;
        // SAFETY: Reading the `mm` field of the area is okay.
        let mm_ptr = unsafe { ptr::addr_of!((*area).vm_mm).read() };

        mm_ptr == self.mm.as_ptr()
    }

    /// Calls `mmget_not_zero` and returns a handle if it succeeds.
    pub fn mmget_not_zero(&self) -> Option<MmGet> {
        // SAFETY: We know that `mm` is still valid since we hold an `mmgrab` refcount.
        let res = unsafe { bindings::mmget_not_zero(self.mm.as_ptr()) };

        if res {
            Some(MmGet { mm: self.mm })
        } else {
            None
        }
    }
}

unsafe impl Send for MmGrab {}
unsafe impl Sync for MmGrab {}

impl Drop for MmGrab {
    fn drop(&mut self) {
        // SAFETY: We acquired a refcount when creating this object.
        unsafe { bindings::mmdrop(self.mm.as_ptr()) };
    }
}

/// A reference to a `struct mm` that holds a reference using `mmget`.
pub struct MmGet {
    mm: NonNull<bindings::mm_struct>,
}

impl MmGet {
    /// Lock the mmap read lock.
    pub fn mmap_write_lock(&self) -> MmapWriteLock<'_> {
        // SAFETY: The pointer is valid since we hold a refcount.
        unsafe { bindings::mmap_write_lock(self.mm.as_ptr()) };

        MmapWriteLock {
            mm: self.mm,
            _lifetime: PhantomData,
        }
    }

    /// When dropping the `MmGet`, use `mmput_async` instead of `mmput`.
    pub fn use_async_put(self) -> MmGetAsync {
        // Disable destructor of `self`.
        let me = ManuallyDrop::new(self);

        MmGetAsync { mm: me.mm }
    }
}

impl Drop for MmGet {
    fn drop(&mut self) {
        // SAFETY: We acquired a refcount when creating this object.
        unsafe { bindings::mmput(self.mm.as_ptr()) };
    }
}

/// A reference to a `struct mm` that holds a reference using `mmget` and will drop it using
/// `mmput_async`.
pub struct MmGetAsync {
    mm: NonNull<bindings::mm_struct>,
}

impl MmGetAsync {
    /// Lock the mmap read lock.
    pub fn mmap_write_lock(&self) -> MmapWriteLock<'_> {
        // SAFETY: The pointer is valid since we hold a refcount.
        unsafe { bindings::mmap_write_lock(self.mm.as_ptr()) };

        MmapWriteLock {
            mm: self.mm,
            _lifetime: PhantomData,
        }
    }

    /// Try to lock the mmap read lock.
    pub fn mmap_read_trylock(&self) -> Option<MmapReadLock<'_>> {
        // SAFETY: The pointer is valid since we hold a refcount.
        let success = unsafe { bindings::mmap_read_trylock(self.mm.as_ptr()) };

        if success {
            Some(MmapReadLock {
                mm: self.mm,
                _lifetime: PhantomData,
            })
        } else {
            None
        }
    }
}

impl Drop for MmGetAsync {
    fn drop(&mut self) {
        // SAFETY: We acquired a refcount when creating this object.
        unsafe { bindings::mmput_async(self.mm.as_ptr()) };
    }
}

/// A guard for the mmap read lock.
pub struct MmapReadLock<'a> {
    mm: NonNull<bindings::mm_struct>,
    _lifetime: PhantomData<&'a bindings::mm_struct>,
}

impl<'a> MmapReadLock<'a> {
    /// Look up a vma at the given address.
    pub fn vma_lookup(&self, vma_addr: usize) -> Option<&virt::Area> {
        // SAFETY: The `mm` pointer is known to be valid while this read lock is held.
        let vma = unsafe { bindings::vma_lookup(self.mm.as_ptr(), vma_addr as u64) };

        if vma.is_null() {
            None
        } else {
            // SAFETY: We just checked that a vma was found, so the pointer is valid. Furthermore,
            // the returned area will borrow from this read lock guard, so it can only be used
            // while the read lock is still held.
            unsafe { Some(virt::Area::from_ptr(vma)) }
        }
    }
}

impl Drop for MmapReadLock<'_> {
    fn drop(&mut self) {
        // SAFETY: We acquired the lock when creating this object.
        unsafe { bindings::mmap_read_unlock(self.mm.as_ptr()) };
    }
}

/// A guard for the mmap write lock.
pub struct MmapWriteLock<'a> {
    mm: NonNull<bindings::mm_struct>,
    _lifetime: PhantomData<&'a mut bindings::mm_struct>,
}

impl<'a> MmapWriteLock<'a> {
    /// Look up a vma at the given address.
    pub fn vma_lookup(&mut self, vma_addr: usize) -> Option<&mut virt::Area> {
        // SAFETY: The `mm` pointer is known to be valid while this read lock is held.
        let vma = unsafe { bindings::vma_lookup(self.mm.as_ptr(), vma_addr as u64) };

        if vma.is_null() {
            None
        } else {
            // SAFETY: We just checked that a vma was found, so the pointer is valid. Furthermore,
            // the returned area will borrow from this read lock guard, so it can only be used
            // while the read lock is still held.
            unsafe { Some(virt::Area::from_ptr_mut(vma)) }
        }
    }
}

impl Drop for MmapWriteLock<'_> {
    fn drop(&mut self) {
        // SAFETY: We acquired the lock when creating this object.
        unsafe { bindings::mmap_write_unlock(self.mm.as_ptr()) };
    }
}

/// Virtual memory.
pub mod virt {
    use super::*;

    /// A wrapper for the kernel's `struct vm_area_struct`.
    ///
    /// It represents an area of virtual memory.
    #[repr(transparent)]
    pub struct Area {
        vma: Opaque<bindings::vm_area_struct>,
    }

    impl Area {
        /// Access a virtual memory area given a raw pointer.
        ///
        /// # Safety
        ///
        /// Callers must ensure that `vma` is non-null and valid for the duration of the new area's
        /// lifetime, with shared access.
        pub unsafe fn from_ptr<'a>(vma: *const bindings::vm_area_struct) -> &'a Self {
            // SAFETY: The caller ensures that the pointer is valid.
            unsafe { &*vma.cast() }
        }

        /// Access a virtual memory area given a raw pointer.
        ///
        /// # Safety
        ///
        /// Callers must ensure that `vma` is non-null and valid for the duration of the new area's
        /// lifetime, with exclusive access.
        pub unsafe fn from_ptr_mut<'a>(vma: *mut bindings::vm_area_struct) -> &'a mut Self {
            // SAFETY: The caller ensures that the pointer is valid.
            unsafe { &mut *vma.cast() }
        }

        /// Returns the flags associated with the virtual memory area.
        ///
        /// The possible flags are a combination of the constants in [`flags`].
        pub fn flags(&self) -> usize {
            // SAFETY: `self.vma` is valid by the type invariants.
            unsafe { (*self.vma.get()).__bindgen_anon_2.vm_flags as _ }
        }

        /// Sets the flags associated with the virtual memory area.
        ///
        /// The possible flags are a combination of the constants in [`flags`].
        pub fn set_flags(&mut self, flags: usize) {
            // SAFETY: `self.vma` is valid by the type invariants.
            unsafe { (*self.vma.get()).__bindgen_anon_2.vm_flags = flags as _ };
        }

        /// Returns the start address of the virtual memory area.
        pub fn start(&self) -> usize {
            // SAFETY: `self.vma` is valid by the type invariants.
            unsafe { (*self.vma.get()).__bindgen_anon_1.__bindgen_anon_1.vm_start as _ }
        }

        /// Returns the end address of the virtual memory area.
        pub fn end(&self) -> usize {
            // SAFETY: `self.vma` is valid by the type invariants.
            unsafe { (*self.vma.get()).__bindgen_anon_1.__bindgen_anon_1.vm_end as _ }
        }

        /// Maps a single page at the given address within the virtual memory area.
        pub fn insert_page(&mut self, address: usize, page: &pages::Pages<0>) -> Result {
            // SAFETY: The page is guaranteed to be order 0 by the type system. The range of
            // `address` is already checked by `vm_insert_page`. `self.vma` and `page.pages` are
            // guaranteed by their respective type invariants to be valid.
            to_result(unsafe {
                bindings::vm_insert_page(self.vma.get(), address as _, page.as_ptr())
            })
        }

        /// Unmap pages in the given page range.
        pub fn zap_page_range(&self, address: usize, size: usize) {
            // SAFETY: The `vma` pointer is valid.
            unsafe {
                bindings::zap_page_range_single(
                    self.vma.get(),
                    address as _,
                    size as _,
                    ptr::null_mut(),
                )
            };
        }
    }

    /// Container for [`Area`] flags.
    pub mod flags {
        use crate::bindings;

        /// No flags are set.
        pub const NONE: usize = bindings::VM_NONE as _;

        /// Mapping allows reads.
        pub const READ: usize = bindings::VM_READ as _;

        /// Mapping allows writes.
        pub const WRITE: usize = bindings::VM_WRITE as _;

        /// Mapping allows execution.
        pub const EXEC: usize = bindings::VM_EXEC as _;

        /// Mapping is shared.
        pub const SHARED: usize = bindings::VM_SHARED as _;

        /// Mapping may be updated to allow reads.
        pub const MAYREAD: usize = bindings::VM_MAYREAD as _;

        /// Mapping may be updated to allow writes.
        pub const MAYWRITE: usize = bindings::VM_MAYWRITE as _;

        /// Mapping may be updated to allow execution.
        pub const MAYEXEC: usize = bindings::VM_MAYEXEC as _;

        /// Mapping may be updated to be shared.
        pub const MAYSHARE: usize = bindings::VM_MAYSHARE as _;

        /// Do not copy this vma on fork.
        pub const DONTCOPY: usize = bindings::VM_DONTCOPY as _;

        /// Cannot expand with mremap().
        pub const DONTEXPAND: usize = bindings::VM_DONTEXPAND as _;

        /// Lock the pages covered when they are faulted in.
        pub const LOCKONFAULT: usize = bindings::VM_LOCKONFAULT as _;

        /// Is a VM accounted object.
        pub const ACCOUNT: usize = bindings::VM_ACCOUNT as _;

        /// should the VM suppress accounting.
        pub const NORESERVE: usize = bindings::VM_NORESERVE as _;

        /// Huge TLB Page VM.
        pub const HUGETLB: usize = bindings::VM_HUGETLB as _;

        /// Synchronous page faults.
        pub const SYNC: usize = bindings::VM_SYNC as _;

        /// Architecture-specific flag.
        pub const ARCH_1: usize = bindings::VM_ARCH_1 as _;

        /// Wipe VMA contents in child..
        pub const WIPEONFORK: usize = bindings::VM_WIPEONFORK as _;

        /// Do not include in the core dump.
        pub const DONTDUMP: usize = bindings::VM_DONTDUMP as _;

        /// Not soft dirty clean area.
        pub const SOFTDIRTY: usize = bindings::VM_SOFTDIRTY as _;

        /// Can contain "struct page" and pure PFN pages.
        pub const MIXEDMAP: usize = bindings::VM_MIXEDMAP as _;

        /// MADV_HUGEPAGE marked this vma.
        pub const HUGEPAGE: usize = bindings::VM_HUGEPAGE as _;

        /// MADV_NOHUGEPAGE marked this vma.
        pub const NOHUGEPAGE: usize = bindings::VM_NOHUGEPAGE as _;

        /// KSM may merge identical pages.
        pub const MERGEABLE: usize = bindings::VM_MERGEABLE as _;
    }
}
