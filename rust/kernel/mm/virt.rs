// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2024 Google LLC.

//! Virtual memory.

use crate::{
    bindings,
    error::{to_result, Result},
    page::Page,
    types::Opaque,
};

use core::ops::Deref;

/// A wrapper for the kernel's `struct vm_area_struct` with read access.
///
/// It represents an area of virtual memory.
///
/// # Invariants
///
/// The caller must hold the mmap read lock or the vma read lock.
#[repr(transparent)]
pub struct VmAreaRef {
    vma: Opaque<bindings::vm_area_struct>,
}

// Methods you can call when holding the mmap or vma read lock (or strong). They must be usable no
// matter what the vma flags are.
impl VmAreaRef {
    /// Access a virtual memory area given a raw pointer.
    ///
    /// # Safety
    ///
    /// Callers must ensure that `vma` is valid for the duration of 'a, and that the mmap or vma
    /// read lock (or stronger) is held for at least the duration of 'a.
    #[inline]
    pub unsafe fn from_raw<'a>(vma: *const bindings::vm_area_struct) -> &'a Self {
        // SAFETY: The caller ensures that the invariants are satisfied for the duration of 'a.
        unsafe { &*vma.cast() }
    }

    /// Returns a raw pointer to this area.
    #[inline]
    pub fn as_ptr(&self) -> *mut bindings::vm_area_struct {
        self.vma.get()
    }

    /// Returns the flags associated with the virtual memory area.
    ///
    /// The possible flags are a combination of the constants in [`flags`].
    #[inline]
    pub fn flags(&self) -> vm_flags_t {
        // SAFETY: By the type invariants, the caller holds at least the mmap read lock, so this
        // access is not a data race.
        unsafe { (*self.as_ptr()).__bindgen_anon_2.vm_flags as _ }
    }

    /// Returns the (inclusive) start address of the virtual memory area.
    #[inline]
    pub fn start(&self) -> usize {
        // SAFETY: By the type invariants, the caller holds at least the mmap read lock, so this
        // access is not a data race.
        unsafe { (*self.as_ptr()).__bindgen_anon_1.__bindgen_anon_1.vm_start as _ }
    }

    /// Returns the (exclusive) end address of the virtual memory area.
    #[inline]
    pub fn end(&self) -> usize {
        // SAFETY: By the type invariants, the caller holds at least the mmap read lock, so this
        // access is not a data race.
        unsafe { (*self.as_ptr()).__bindgen_anon_1.__bindgen_anon_1.vm_end as _ }
    }

    /// Zap pages in the given page range.
    ///
    /// This clears page table mappings for the range at the leaf level, leaving all other page
    /// tables intact, and freeing any memory referenced by the VMA in this range. That is,
    /// anonymous memory is completely freed, file-backed memory has its reference count on page
    /// cache folio's dropped, any dirty data will still be written back to disk as usual.
    #[inline]
    pub fn zap_page_range_single(&self, address: usize, size: usize) {
        // SAFETY: By the type invariants, the caller has read access to this VMA, which is
        // sufficient for this method call. This method has no requirements on the vma flags. Any
        // value of `address` and `size` is allowed.
        unsafe {
            bindings::zap_page_range_single(
                self.as_ptr(),
                address as _,
                size as _,
                core::ptr::null_mut(),
            )
        };
    }

    /// Check whether the `VM_MIXEDMAP` flag is set.
    ///
    /// This can be used to access methods that require `VM_MIXEDMAP` to be set.
    #[inline]
    pub fn as_mixedmap_vma(&self) -> Option<&VmAreaMixedMap> {
        if self.flags() & flags::MIXEDMAP != 0 {
            // SAFETY: We just checked that `VM_MIXEDMAP` is set. All other requirements are
            // satisfied by the type invariants of `VmAreaRef`.
            Some(unsafe { VmAreaMixedMap::from_raw(self.as_ptr()) })
        } else {
            None
        }
    }
}

/// A wrapper for the kernel's `struct vm_area_struct` with read access and `VM_MIXEDMAP` set.
///
/// It represents an area of virtual memory.
///
/// # Invariants
///
/// The caller must hold the mmap read lock or the vma read lock. The `VM_MIXEDMAP` flag must be
/// set.
#[repr(transparent)]
pub struct VmAreaMixedMap {
    vma: VmAreaRef,
}

// Make all `VmAreaRef` methods available on `VmAreaMixedMap`.
impl Deref for VmAreaMixedMap {
    type Target = VmAreaRef;

    #[inline]
    fn deref(&self) -> &VmAreaRef {
        &self.vma
    }
}

impl VmAreaMixedMap {
    /// Access a virtual memory area given a raw pointer.
    ///
    /// # Safety
    ///
    /// Callers must ensure that `vma` is valid for the duration of 'a, and that the mmap read lock
    /// (or stronger) is held for at least the duration of 'a. The `VM_MIXEDMAP` flag must be set.
    #[inline]
    pub unsafe fn from_raw<'a>(vma: *const bindings::vm_area_struct) -> &'a Self {
        // SAFETY: The caller ensures that the invariants are satisfied for the duration of 'a.
        unsafe { &*vma.cast() }
    }

    /// Maps a single page at the given address within the virtual memory area.
    ///
    /// This operation does not take ownership of the page.
    #[inline]
    pub fn vm_insert_page(&self, address: usize, page: &Page) -> Result {
        // SAFETY: The caller has read access and has verified that `VM_MIXEDMAP` is set. The page
        // is order 0. The address is checked on the C side so it can take any value.
        to_result(unsafe { bindings::vm_insert_page(self.as_ptr(), address as _, page.as_ptr()) })
    }
}

/// The integer type used for vma flags.
#[doc(inline)]
pub use bindings::vm_flags_t;

/// All possible flags for [`VmAreaRef`].
pub mod flags {
    use super::vm_flags_t;
    use crate::bindings;

    /// No flags are set.
    pub const NONE: vm_flags_t = bindings::VM_NONE as _;

    /// Mapping allows reads.
    pub const READ: vm_flags_t = bindings::VM_READ as _;

    /// Mapping allows writes.
    pub const WRITE: vm_flags_t = bindings::VM_WRITE as _;

    /// Mapping allows execution.
    pub const EXEC: vm_flags_t = bindings::VM_EXEC as _;

    /// Mapping is shared.
    pub const SHARED: vm_flags_t = bindings::VM_SHARED as _;

    /// Mapping may be updated to allow reads.
    pub const MAYREAD: vm_flags_t = bindings::VM_MAYREAD as _;

    /// Mapping may be updated to allow writes.
    pub const MAYWRITE: vm_flags_t = bindings::VM_MAYWRITE as _;

    /// Mapping may be updated to allow execution.
    pub const MAYEXEC: vm_flags_t = bindings::VM_MAYEXEC as _;

    /// Mapping may be updated to be shared.
    pub const MAYSHARE: vm_flags_t = bindings::VM_MAYSHARE as _;

    /// Page-ranges managed without `struct page`, just pure PFN.
    pub const PFNMAP: vm_flags_t = bindings::VM_PFNMAP as _;

    /// Memory mapped I/O or similar.
    pub const IO: vm_flags_t = bindings::VM_IO as _;

    /// Do not copy this vma on fork.
    pub const DONTCOPY: vm_flags_t = bindings::VM_DONTCOPY as _;

    /// Cannot expand with mremap().
    pub const DONTEXPAND: vm_flags_t = bindings::VM_DONTEXPAND as _;

    /// Lock the pages covered when they are faulted in.
    pub const LOCKONFAULT: vm_flags_t = bindings::VM_LOCKONFAULT as _;

    /// Is a VM accounted object.
    pub const ACCOUNT: vm_flags_t = bindings::VM_ACCOUNT as _;

    /// Should the VM suppress accounting.
    pub const NORESERVE: vm_flags_t = bindings::VM_NORESERVE as _;

    /// Huge TLB Page VM.
    pub const HUGETLB: vm_flags_t = bindings::VM_HUGETLB as _;

    /// Synchronous page faults. (DAX-specific)
    pub const SYNC: vm_flags_t = bindings::VM_SYNC as _;

    /// Architecture-specific flag.
    pub const ARCH_1: vm_flags_t = bindings::VM_ARCH_1 as _;

    /// Wipe VMA contents in child on fork.
    pub const WIPEONFORK: vm_flags_t = bindings::VM_WIPEONFORK as _;

    /// Do not include in the core dump.
    pub const DONTDUMP: vm_flags_t = bindings::VM_DONTDUMP as _;

    /// Not soft dirty clean area.
    pub const SOFTDIRTY: vm_flags_t = bindings::VM_SOFTDIRTY as _;

    /// Can contain `struct page` and pure PFN pages.
    pub const MIXEDMAP: vm_flags_t = bindings::VM_MIXEDMAP as _;

    /// MADV_HUGEPAGE marked this vma.
    pub const HUGEPAGE: vm_flags_t = bindings::VM_HUGEPAGE as _;

    /// MADV_NOHUGEPAGE marked this vma.
    pub const NOHUGEPAGE: vm_flags_t = bindings::VM_NOHUGEPAGE as _;

    /// KSM may merge identical pages.
    pub const MERGEABLE: vm_flags_t = bindings::VM_MERGEABLE as _;
}
