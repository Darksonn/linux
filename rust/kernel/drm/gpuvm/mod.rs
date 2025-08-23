// SPDX-License-Identifier: GPL-2.0 OR MIT

//! DRM GPUVM in immediate mode
//!
//! Rust abstractions for using GPUVM in immediate mode. This is when the GPUVM state is updated
//! during `run_job()`, i.e., in the DMA fence signalling critical path, to ensure that the GPUVM
//! and the GPU's virtual address space has the same state at all times.
//!
//! C header: [`include/drm/drm_gpuvm.h`](srctree/include/drm/drm_gpuvm.h)

use kernel::{
    alloc::{AllocError, Flags as AllocFlags},
    bindings, drm,
    drm::gem::IntoGEMObject,
    error::to_result,
    prelude::*,
    sync::aref::{ARef, AlwaysRefCounted},
    types::Opaque,
};

use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::{ManuallyDrop, MaybeUninit},
    ops::{Deref, DerefMut, Range},
    ptr::{self, NonNull},
};

mod sm_ops;
pub use self::sm_ops::*;

mod vm_bo;
pub use self::vm_bo::*;

mod va;
pub use self::va::*;

/// A DRM GPU VA manager.
///
/// This object is refcounted, but the "core" is only accessible using a special unique handle. The
/// core consists of the `core` field and the GPUVM's interval tree.
#[repr(C)]
#[pin_data]
pub struct GpuVm<T: DriverGpuVm> {
    #[pin]
    vm: Opaque<bindings::drm_gpuvm>,
    /// Accessed only through the [`GpuVmCore`] reference.
    core: UnsafeCell<T>,
    /// Shared data not protected by any lock.
    #[pin]
    shared_data: T::SharedData,
}

// SAFETY: dox
unsafe impl<T: DriverGpuVm> AlwaysRefCounted for GpuVm<T> {
    fn inc_ref(&self) {
        // SAFETY: dox
        unsafe { bindings::drm_gpuvm_get(self.vm.get()) };
    }

    unsafe fn dec_ref(obj: NonNull<Self>) {
        // SAFETY: dox
        unsafe { bindings::drm_gpuvm_put((*obj.as_ptr()).vm.get()) };
    }
}

impl<T: DriverGpuVm> GpuVm<T> {
    const fn vtable() -> &'static bindings::drm_gpuvm_ops {
        &bindings::drm_gpuvm_ops {
            vm_free: Some(Self::vm_free),
            op_alloc: None,
            op_free: None,
            vm_bo_alloc: GpuVmBo::<T>::ALLOC_FN,
            vm_bo_free: GpuVmBo::<T>::FREE_FN,
            vm_bo_validate: None,
            sm_step_map: Some(Self::sm_step_map),
            sm_step_unmap: Some(Self::sm_step_unmap),
            sm_step_remap: Some(Self::sm_step_remap),
        }
    }

    /// Creates a GPUVM instance.
    #[expect(clippy::new_ret_no_self)]
    pub fn new<E>(
        name: &'static CStr,
        dev: &drm::Device<T::Driver>,
        r_obj: &T::Object,
        range: Range<u64>,
        reserve_range: Range<u64>,
        core: T,
        shared: impl PinInit<T::SharedData, E>,
    ) -> Result<GpuVmCore<T>, E>
    where
        E: From<AllocError>,
        E: From<core::convert::Infallible>,
    {
        let obj = KBox::try_pin_init::<E>(
            try_pin_init!(Self {
                core <- UnsafeCell::new(core),
                shared_data <- shared,
                vm <- Opaque::ffi_init(|vm| {
                    // SAFETY: These arguments are valid. `vm` is valid until refcount drops to
                    // zero.
                    unsafe {
                        bindings::drm_gpuvm_init(
                            vm,
                            name.as_char_ptr(),
                            bindings::drm_gpuvm_flags_DRM_GPUVM_IMMEDIATE_MODE
                                | bindings::drm_gpuvm_flags_DRM_GPUVM_RESV_PROTECTED,
                            dev.as_raw(),
                            r_obj.as_raw(),
                            range.start,
                            range.end - range.start,
                            reserve_range.start,
                            reserve_range.end - reserve_range.start,
                            const { Self::vtable() },
                        )
                    }
                }),
            }? E),
            GFP_KERNEL,
        )?;
        // SAFETY: This transfers the initial refcount to the ARef.
        Ok(GpuVmCore(unsafe {
            ARef::from_raw(NonNull::new_unchecked(KBox::into_raw(
                Pin::into_inner_unchecked(obj),
            )))
        }))
    }

    /// Access this [`GpuVm`] from a raw pointer.
    ///
    /// # Safety
    ///
    /// For the duration of `'a`, the pointer must reference a valid [`GpuVm<T>`].
    #[inline]
    pub unsafe fn from_raw<'a>(ptr: *mut bindings::drm_gpuvm) -> &'a Self {
        // SAFETY: `drm_gpuvm` is first field and `repr(C)`.
        unsafe { &*ptr.cast() }
    }

    /// Get a raw pointer.
    #[inline]
    pub fn as_raw(&self) -> *mut bindings::drm_gpuvm {
        self.vm.get()
    }

    /// Access the shared data.
    #[inline]
    pub fn shared(&self) -> &T::SharedData {
        &self.shared_data
    }

    /// The start of the VA space.
    #[inline]
    pub fn va_start(&self) -> u64 {
        // SAFETY: Safe by the type invariant of `GpuVm<T>`.
        unsafe { (*self.as_raw()).mm_start }
    }

    /// The length of the address space
    #[inline]
    pub fn va_length(&self) -> u64 {
        // SAFETY: Safe by the type invariant of `GpuVm<T>`.
        unsafe { (*self.as_raw()).mm_range }
    }

    /// Returns the range of the GPU virtual address space.
    #[inline]
    pub fn va_range(&self) -> Range<u64> {
        let start = self.va_start();
        let end = start + self.va_length();
        Range { start, end }
    }

    /// Returns a [`GpuVmBoObtain`] for the provided GEM object.
    #[inline]
    pub fn obtain(
        &self,
        obj: &T::Object,
        data: impl PinInit<T::VmBoData>,
    ) -> Result<GpuVmBoObtain<T>, AllocError> {
        Ok(GpuVmBoAlloc::new(self, obj, data)?.obtain())
    }

    /// Prepare this GPUVM.
    #[inline]
    pub fn prepare(&self, num_fences: u32) -> impl PinInit<GpuVmExec<'_, T>, Error> {
        try_pin_init!(GpuVmExec {
            exec <- Opaque::try_ffi_init(|exec: *mut bindings::drm_gpuvm_exec| {
                // SAFETY: exec is valid but unused memory, so we can write.
                unsafe {
                    ptr::write_bytes(exec, 0u8, 1usize);
                    ptr::write(&raw mut (*exec).vm, self.as_raw());
                    ptr::write(&raw mut (*exec).flags, bindings::DRM_EXEC_INTERRUPTIBLE_WAIT);
                    ptr::write(&raw mut (*exec).num_fences, num_fences);
                }

                // SAFETY: We can prepare the GPUVM.
                to_result(unsafe { bindings::drm_gpuvm_exec_lock(exec) })
            }),
            _gpuvm: PhantomData,
        })
    }

    /// Clean up buffer objects that are no longer used.
    #[inline]
    pub fn deferred_cleanup(&self) {
        // SAFETY: Always safe to perform deferred cleanup.
        unsafe { bindings::drm_gpuvm_bo_deferred_cleanup(self.as_raw()) }
    }

    /// Check if this GEM object is an external object for this GPUVM.
    #[inline]
    pub fn is_extobj(&self, obj: &T::Object) -> bool {
        // SAFETY: We may call this with any GPUVM and GEM object.
        unsafe { bindings::drm_gpuvm_is_extobj(self.as_raw(), obj.as_raw()) }
    }

    /// Free this GPUVM.
    ///
    /// # Safety
    ///
    /// Called when refcount hits zero.
    unsafe extern "C" fn vm_free(me: *mut bindings::drm_gpuvm) {
        // SAFETY: GPUVM was allocated with KBox and can now be freed.
        drop(unsafe { KBox::<Self>::from_raw(me.cast()) })
    }
}

/// The manager for a GPUVM.
pub trait DriverGpuVm: Sized {
    /// Parent `Driver` for this object.
    type Driver: drm::Driver;

    /// The kind of GEM object stored in this GPUVM.
    type Object: IntoGEMObject;

    /// Data stored in the [`GpuVm`] that is fully shared.
    type SharedData;

    /// Data stored with each `struct drm_gpuvm_bo`.
    type VmBoData;

    /// Data stored with each `struct drm_gpuva`.
    type VaData;

    /// The private data passed to callbacks.
    type SmContext;

    /// Indicates that a new mapping should be created.
    fn sm_step_map<'op>(
        &mut self,
        op: OpMap<'op, Self>,
        context: &mut Self::SmContext,
    ) -> Result<OpMapped<'op, Self>, Error>;

    /// Indicates that an existing mapping should be removed.
    fn sm_step_unmap<'op>(
        &mut self,
        op: OpUnmap<'op, Self>,
        context: &mut Self::SmContext,
    ) -> Result<OpUnmapped<'op, Self>, Error>;

    /// Indicates that an existing mapping should be split up.
    fn sm_step_remap<'op>(
        &mut self,
        op: OpRemap<'op, Self>,
        context: &mut Self::SmContext,
    ) -> Result<OpRemapped<'op, Self>, Error>;
}

/// The core of the DRM GPU VA manager.
///
/// This object is the reference to the GPUVM that
///
/// # Invariants
///
/// This object owns the core.
pub struct GpuVmCore<T: DriverGpuVm>(ARef<GpuVm<T>>);

impl<T: DriverGpuVm> GpuVmCore<T> {
    /// Get a reference without access to `core`.
    #[inline]
    pub fn gpuvm(&self) -> &GpuVm<T> {
        &self.0
    }
}

impl<T: DriverGpuVm> Deref for GpuVmCore<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: By the type invariants we may access `core`.
        unsafe { &*self.0.core.get() }
    }
}

impl<T: DriverGpuVm> DerefMut for GpuVmCore<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: By the type invariants we may access `core`.
        unsafe { &mut *self.0.core.get() }
    }
}

/// The exec token for preparing the objects.
#[pin_data(PinnedDrop)]
pub struct GpuVmExec<'a, T: DriverGpuVm> {
    #[pin]
    exec: Opaque<bindings::drm_gpuvm_exec>,
    _gpuvm: PhantomData<&'a mut GpuVm<T>>,
}

impl<'a, T: DriverGpuVm> GpuVmExec<'a, T> {
    /// Add a fence.
    ///
    /// # Safety
    ///
    /// `fence` arg must be valid.
    pub unsafe fn resv_add_fence(
        &self,
        // TODO: use a safe fence abstraction
        fence: *mut bindings::dma_fence,
        private_usage: DmaResvUsage,
        extobj_usage: DmaResvUsage,
    ) {
        // SAFETY: Caller ensures fence is ok.
        unsafe {
            bindings::drm_gpuvm_resv_add_fence(
                (*self.exec.get()).vm,
                &raw mut (*self.exec.get()).exec,
                fence,
                private_usage as u32,
                extobj_usage as u32,
            )
        }
    }
}

#[pinned_drop]
impl<'a, T: DriverGpuVm> PinnedDrop for GpuVmExec<'a, T> {
    fn drop(self: Pin<&mut Self>) {
        // SAFETY: We hold the lock, so it's safe to unlock.
        unsafe { bindings::drm_gpuvm_exec_unlock(self.exec.get()) };
    }
}

/// How the fence will be used.
#[repr(u32)]
pub enum DmaResvUsage {
    /// For in kernel memory management only (e.g. copying, clearing memory).
    Kernel = bindings::dma_resv_usage_DMA_RESV_USAGE_KERNEL,
    /// Implicit write synchronization for userspace submissions.
    Write = bindings::dma_resv_usage_DMA_RESV_USAGE_WRITE,
    /// Implicit read synchronization for userspace submissions.
    Read = bindings::dma_resv_usage_DMA_RESV_USAGE_READ,
    /// No implicit sync (e.g. preemption fences, page table updates, TLB flushes).
    Bookkeep = bindings::dma_resv_usage_DMA_RESV_USAGE_BOOKKEEP,
}

/// A lock guard for the GPUVM's resv lock.
///
/// This guard provides access to the extobj and evicted lists.
///
/// # Invariants
///
/// Holds the GPUVM resv lock.
pub struct GpuvmResvLockGuard<'a, T: DriverGpuVm>(&'a GpuVm<T>);

impl<T: DriverGpuVm> GpuVm<T> {
    /// Lock the VM's resv lock.
    #[inline]
    pub fn resv_lock(&self) -> GpuvmResvLockGuard<'_, T> {
        // SAFETY: It's always ok to lock the resv lock.
        unsafe { bindings::dma_resv_lock(self.raw_resv_lock(), ptr::null_mut()) };
        // INVARIANTS: We took the lock.
        GpuvmResvLockGuard(self)
    }

    #[inline]
    fn raw_resv_lock(&self) -> *mut bindings::dma_resv {
        // SAFETY: `r_obj` is immutable and valid for duration of GPUVM.
        unsafe { (*(*self.as_raw()).r_obj).resv }
    }
}

impl<'a, T: DriverGpuVm> Drop for GpuvmResvLockGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: We hold the lock so we can release it.
        unsafe { bindings::dma_resv_unlock(self.0.raw_resv_lock()) };
    }
}
