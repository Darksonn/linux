// SPDX-License-Identifier: GPL-2.0 OR MIT
#![allow(clippy::tabs_in_doc_comments)]

use super::*;

struct SmData<'a, T: DriverGpuVm> {
    gpuvm: &'a mut GpuVmCore<T>,
    user_context: &'a mut T::SmContext,
}

#[repr(C)]
struct SmMapData<'a, T: DriverGpuVm> {
    sm_data: SmData<'a, T>,
    vm_bo: GpuVmBoObtain<T>,
}

/// The argument for [`GpuVmCore::sm_map`].
pub struct OpMapRequest<'a, T: DriverGpuVm> {
    /// Address in GPU virtual address space.
    pub addr: u64,
    /// Length of mapping to create.
    pub range: u64,
    /// Offset in GEM object.
    pub offset: u64,
    /// The GEM object to map.
    pub vm_bo: GpuVmBoObtain<T>,
    /// The user-provided context type.
    pub context: &'a mut T::SmContext,
}

impl<'a, T: DriverGpuVm> OpMapRequest<'a, T> {
    fn raw_request(&self) -> bindings::drm_gpuvm_map_req {
        bindings::drm_gpuvm_map_req {
            map: bindings::drm_gpuva_op_map {
                va: bindings::drm_gpuva_op_map__bindgen_ty_1 {
                    addr: self.addr,
                    range: self.range,
                },
                gem: bindings::drm_gpuva_op_map__bindgen_ty_2 {
                    offset: self.offset,
                    obj: self.vm_bo.obj().as_raw(),
                },
            },
        }
    }
}

/// ```
/// struct drm_gpuva_op_map {
/// 	/**
/// 	 * @va: structure containing address and range of a map
/// 	 * operation
/// 	 */
/// 	struct {
/// 		/**
/// 		 * @va.addr: the base address of the new mapping
/// 		 */
/// 		u64 addr;
///
/// 		/**
/// 		 * @va.range: the range of the new mapping
/// 		 */
/// 		u64 range;
/// 	} va;
///
/// 	/**
/// 	 * @gem: structure containing the &drm_gem_object and it's offset
/// 	 */
/// 	struct {
/// 		/**
/// 		 * @gem.offset: the offset within the &drm_gem_object
/// 		 */
/// 		u64 offset;
///
/// 		/**
/// 		 * @gem.obj: the &drm_gem_object to map
/// 		 */
/// 		struct drm_gem_object *obj;
/// 	} gem;
/// };
/// ```
pub struct OpMap<'op, T: DriverGpuVm> {
    op: &'op bindings::drm_gpuva_op_map,
    // Since these abstractions are designed for immediate mode, the VM BO needs to be
    // pre-allocated, so we always have it available when we reach this point.
    vm_bo: &'op GpuVmBo<T>,
    _invariant: PhantomData<*mut &'op mut T>,
}

impl<'op, T: DriverGpuVm> OpMap<'op, T> {
    /// The base address of the new mapping.
    pub fn addr(&self) -> u64 {
        self.op.va.addr
    }

    /// The length of the new mapping.
    pub fn length(&self) -> u64 {
        self.op.va.range
    }

    /// The offset within the [`drm_gem_object`](crate::gem::Object).
    pub fn gem_offset(&self) -> u64 {
        self.op.gem.offset
    }

    /// The [`drm_gem_object`](crate::gem::Object) to map.
    pub fn obj(&self) -> &T::Object {
        // SAFETY: The `obj` pointer is guaranteed to be valid.
        unsafe { <T::Object as IntoGEMObject>::from_raw(self.op.gem.obj) }
    }

    /// The [`GpuVmBo`] that the new VA will be associated with.
    pub fn vm_bo(&self) -> &GpuVmBo<T> {
        self.vm_bo
    }

    /// Use the pre-allocated VA to carry out this map operation.
    pub fn insert(self, va: GpuVaAlloc<T>, va_data: impl PinInit<T::VaData>) -> OpMapped<'op, T> {
        let va = va.prepare(va_data);
        // SAFETY: By the type invariants we may access the interval tree.
        unsafe { bindings::drm_gpuva_map(self.vm_bo.gpuvm().as_raw(), va, self.op) };
        // SAFETY: The GEM object is valid, so the mutex is properly initialized.
        unsafe { bindings::mutex_lock(&raw mut (*self.op.gem.obj).gpuva.lock) };
        // SAFETY: The va is prepared for insertion, and we hold the GEM lock.
        unsafe { bindings::drm_gpuva_link(va, self.vm_bo.as_raw()) };
        // SAFETY: We took the mutex above, so we may unlock it.
        unsafe { bindings::mutex_unlock(&raw mut (*self.op.gem.obj).gpuva.lock) };
        OpMapped {
            _invariant: self._invariant,
        }
    }
}

/// Represents a completed [`OpMap`] operation.
pub struct OpMapped<'op, T> {
    _invariant: PhantomData<*mut &'op mut T>,
}

/// ```
/// struct drm_gpuva_op_unmap {
/// 	/**
/// 	 * @va: the &drm_gpuva to unmap
/// 	 */
/// 	struct drm_gpuva *va;
///
/// 	/**
/// 	 * @keep:
/// 	 *
/// 	 * Indicates whether this &drm_gpuva is physically contiguous with the
/// 	 * original mapping request.
/// 	 *
/// 	 * Optionally, if &keep is set, drivers may keep the actual page table
/// 	 * mappings for this &drm_gpuva, adding the missing page table entries
/// 	 * only and update the &drm_gpuvm accordingly.
/// 	 */
/// 	bool keep;
/// };
/// ```
pub struct OpUnmap<'op, T: DriverGpuVm> {
    op: &'op bindings::drm_gpuva_op_unmap,
    _invariant: PhantomData<*mut &'op mut T>,
}

impl<'op, T: DriverGpuVm> OpUnmap<'op, T> {
    /// Indicates whether this `drm_gpuva` is physically contiguous with the
    /// original mapping request.
    ///
    /// Optionally, if `keep` is set, drivers may keep the actual page table
    /// mappings for this `drm_gpuva`, adding the missing page table entries
    /// only and update the `drm_gpuvm` accordingly.
    pub fn keep(&self) -> bool {
        self.op.keep
    }

    /// The range being unmapped.
    pub fn va(&self) -> &GpuVa<T> {
        // SAFETY: This is a valid va.
        unsafe { GpuVa::<T>::from_raw(self.op.va) }
    }

    /// Remove the VA.
    pub fn remove(self) -> (OpUnmapped<'op, T>, GpuVaRemoved<T>) {
        // SAFETY: The op references a valid drm_gpuva in the GPUVM.
        unsafe { bindings::drm_gpuva_unmap(self.op) };
        // SAFETY: The va is no longer in the interval tree so we may unlink it.
        unsafe { bindings::drm_gpuva_unlink_defer(self.op.va) };

        // SAFETY: We just removed this va from the `GpuVm<T>`.
        let va = unsafe { GpuVaRemoved::from_raw(self.op.va) };

        (
            OpUnmapped {
                _invariant: self._invariant,
            },
            va,
        )
    }
}

/// Represents a completed [`OpUnmap`] operation.
pub struct OpUnmapped<'op, T> {
    _invariant: PhantomData<*mut &'op mut T>,
}

/// ```
/// struct drm_gpuva_op_remap {
/// 	/**
/// 	 * @prev: the preceding part of a split mapping
/// 	 */
/// 	struct drm_gpuva_op_map *prev;
///
/// 	/**
/// 	 * @next: the subsequent part of a split mapping
/// 	 */
/// 	struct drm_gpuva_op_map *next;
///
/// 	/**
/// 	 * @unmap: the unmap operation for the original existing mapping
/// 	 */
/// 	struct drm_gpuva_op_unmap *unmap;
/// };
/// ```
pub struct OpRemap<'op, T: DriverGpuVm> {
    op: &'op bindings::drm_gpuva_op_remap,
    _invariant: PhantomData<*mut &'op mut T>,
}

impl<'op, T: DriverGpuVm> OpRemap<'op, T> {
    /// The preceding part of a split mapping.
    #[inline]
    pub fn prev(&self) -> Option<&OpRemapMapData> {
        // SAFETY: We checked for null, so the pointer must be valid.
        NonNull::new(self.op.prev).map(|ptr| unsafe { OpRemapMapData::from_raw(ptr) })
    }

    /// The subsequent part of a split mapping.
    #[inline]
    pub fn next(&self) -> Option<&OpRemapMapData> {
        // SAFETY: We checked for null, so the pointer must be valid.
        NonNull::new(self.op.next).map(|ptr| unsafe { OpRemapMapData::from_raw(ptr) })
    }

    /// Indicates whether the `drm_gpuva` being removed is physically contiguous with the original
    /// mapping request.
    ///
    /// Optionally, if `keep` is set, drivers may keep the actual page table mappings for this
    /// `drm_gpuva`, adding the missing page table entries only and update the `drm_gpuvm`
    /// accordingly.
    #[inline]
    pub fn keep(&self) -> bool {
        // SAFETY: The unmap pointer is always valid.
        unsafe { (*self.op.unmap).keep }
    }

    /// The range being unmapped.
    #[inline]
    pub fn va_to_unmap(&self) -> &GpuVa<T> {
        // SAFETY: This is a valid va.
        unsafe { GpuVa::<T>::from_raw((*self.op.unmap).va) }
    }

    /// The [`drm_gem_object`](crate::gem::Object) whose VA is being remapped.
    #[inline]
    pub fn obj(&self) -> &T::Object {
        self.va_to_unmap().obj()
    }

    /// The [`GpuVmBo`] that is being remapped.
    #[inline]
    pub fn vm_bo(&self) -> &GpuVmBo<T> {
        self.va_to_unmap().vm_bo()
    }

    /// Update the GPUVM to perform the remapping.
    pub fn remap(
        self,
        va_alloc: [GpuVaAlloc<T>; 2],
        prev_data: impl PinInit<T::VaData>,
        next_data: impl PinInit<T::VaData>,
    ) -> (OpRemapped<'op, T>, OpRemapRet<T>) {
        let [va1, va2] = va_alloc;

        let mut unused_va = None;
        let mut prev_ptr = ptr::null_mut();
        let mut next_ptr = ptr::null_mut();
        if self.prev().is_some() {
            prev_ptr = va1.prepare(prev_data);
        } else {
            unused_va = Some(va1);
        }
        if self.next().is_some() {
            next_ptr = va2.prepare(next_data);
        } else {
            unused_va = Some(va2);
        }

        // SAFETY: the pointers are non-null when required
        unsafe { bindings::drm_gpuva_remap(prev_ptr, next_ptr, self.op) };

        // SAFETY: The GEM object is valid, so the mutex is properly initialized.
        unsafe { bindings::mutex_lock(&raw mut (*self.obj().as_raw()).gpuva.lock) };
        if !prev_ptr.is_null() {
            // SAFETY: The prev_ptr is a valid drm_gpuva prepared for insertion. The vm_bo is still
            // valid as the not-yet-unlinked gpuva holds a refcount on the vm_bo.
            unsafe { bindings::drm_gpuva_link(prev_ptr, self.vm_bo().as_raw()) };
        }
        if !next_ptr.is_null() {
            // SAFETY: The next_ptr is a valid drm_gpuva prepared for insertion. The vm_bo is still
            // valid as the not-yet-unlinked gpuva holds a refcount on the vm_bo.
            unsafe { bindings::drm_gpuva_link(next_ptr, self.vm_bo().as_raw()) };
        }
        // SAFETY: We took the mutex above, so we may unlock it.
        unsafe { bindings::mutex_unlock(&raw mut (*self.obj().as_raw()).gpuva.lock) };
        // SAFETY: The va is no longer in the interval tree so we may unlink it.
        unsafe { bindings::drm_gpuva_unlink_defer((*self.op.unmap).va) };

        (
            OpRemapped {
                _invariant: self._invariant,
            },
            OpRemapRet {
                // SAFETY: We just removed this va from the `GpuVm<T>`.
                unmapped_va: unsafe { GpuVaRemoved::from_raw((*self.op.unmap).va) },
                unused_va,
            },
        )
    }
}

/// Part of an [`OpRemap`] that represents a new mapping.
#[repr(transparent)]
pub struct OpRemapMapData(bindings::drm_gpuva_op_map);

impl OpRemapMapData {
    /// # Safety
    /// Must reference a valid `drm_gpuva_op_map` for duration of `'a`.
    unsafe fn from_raw<'a>(ptr: NonNull<bindings::drm_gpuva_op_map>) -> &'a Self {
        // SAFETY: ok per safety requirements
        unsafe { ptr.cast().as_ref() }
    }

    /// The base address of the new mapping.
    pub fn addr(&self) -> u64 {
        self.0.va.addr
    }

    /// The length of the new mapping.
    pub fn length(&self) -> u64 {
        self.0.va.range
    }

    /// The offset within the [`drm_gem_object`](crate::gem::Object).
    pub fn gem_offset(&self) -> u64 {
        self.0.gem.offset
    }
}

/// Struct containing objects removed or not used by [`OpRemap::remap`].
pub struct OpRemapRet<T: DriverGpuVm> {
    /// The `drm_gpuva` that was removed.
    pub unmapped_va: GpuVaRemoved<T>,
    /// If the remap did not split the region into two pieces, then the unused `drm_gpuva` is
    /// returned here.
    pub unused_va: Option<GpuVaAlloc<T>>,
}

/// Represents a completed [`OpRemap`] operation.
pub struct OpRemapped<'op, T> {
    _invariant: PhantomData<*mut &'op mut T>,
}

impl<T: DriverGpuVm> GpuVmCore<T> {
    /// Create a mapping, removing or remapping anything that overlaps.
    #[inline]
    pub fn sm_map(&mut self, req: OpMapRequest<'_, T>) -> Result {
        let gpuvm = self.gpuvm().as_raw();
        let raw_req = req.raw_request();
        let mut p = SmMapData {
            sm_data: SmData {
                gpuvm: self,
                user_context: req.context,
            },
            vm_bo: req.vm_bo,
        };
        // SAFETY:
        // * raw_request() creates a valid request.
        // * The private data is valid to be interpreted as both SmData and SmMapData since the
        //   first field of SmMapData is SmData.
        to_result(unsafe {
            bindings::drm_gpuvm_sm_map(gpuvm, (&raw mut p).cast(), &raw const raw_req)
        })
    }

    /// Remove any mappings in the given region.
    #[inline]
    pub fn sm_unmap(&mut self, addr: u64, length: u64, context: &mut T::SmContext) -> Result {
        let gpuvm = self.gpuvm().as_raw();
        let mut p = SmData {
            gpuvm: self,
            user_context: context,
        };
        // SAFETY:
        // * raw_request() creates a valid request.
        // * The private data is valid to be interpreted as only SmData, but drm_gpuvm_sm_unmap()
        //   never calls sm_step_map().
        to_result(unsafe { bindings::drm_gpuvm_sm_unmap(gpuvm, (&raw mut p).cast(), addr, length) })
    }
}

impl<T: DriverGpuVm> GpuVm<T> {
    /// # Safety
    /// Must be called from `sm_map`.
    pub(super) unsafe extern "C" fn sm_step_map(
        op: *mut bindings::drm_gpuva_op,
        p: *mut c_void,
    ) -> c_int {
        // SAFETY: If we reach `sm_step_map` then we were called from `sm_map` which always passes
        // an `SmMapData` as private data.
        let p = unsafe { &mut *p.cast::<SmMapData<'_, T>>() };
        let op = OpMap {
            // SAFETY: sm_step_map is called with a map operation.
            op: unsafe { &(*op).__bindgen_anon_1.map },
            vm_bo: &p.vm_bo,
            _invariant: PhantomData,
        };
        match p.sm_data.gpuvm.sm_step_map(op, p.sm_data.user_context) {
            Ok(OpMapped { .. }) => 0,
            Err(err) => err.to_errno(),
        }
    }
    /// # Safety
    /// Must be called from `sm_map` or `sm_unmap`.
    pub(super) unsafe extern "C" fn sm_step_unmap(
        op: *mut bindings::drm_gpuva_op,
        p: *mut c_void,
    ) -> c_int {
        // SAFETY: If we reach `sm_step_unmap` then we were called from `sm_map` or `sm_unmap` which passes either
        // an `SmMapData` or `SmData` as private data. Both cases can be cast to `SmData`.
        let p = unsafe { &mut *p.cast::<SmData<'_, T>>() };
        let op = OpUnmap {
            // SAFETY: sm_step_unmap is called with an unmap operation.
            op: unsafe { &(*op).__bindgen_anon_1.unmap },
            _invariant: PhantomData,
        };
        match p.gpuvm.sm_step_unmap(op, p.user_context) {
            Ok(OpUnmapped { .. }) => 0,
            Err(err) => err.to_errno(),
        }
    }
    /// # Safety
    /// Must be called from `sm_map` or `sm_unmap`.
    pub(super) unsafe extern "C" fn sm_step_remap(
        op: *mut bindings::drm_gpuva_op,
        p: *mut c_void,
    ) -> c_int {
        // SAFETY: If we reach `sm_step_remap` then we were called from `sm_map` or `sm_unmap` which passes either
        // an `SmMapData` or `SmData` as private data. Both cases can be cast to `SmData`.
        let p = unsafe { &mut *p.cast::<SmData<'_, T>>() };
        let op = OpRemap {
            // SAFETY: sm_step_remap is called with a remap operation.
            op: unsafe { &(*op).__bindgen_anon_1.remap },
            _invariant: PhantomData,
        };
        match p.gpuvm.sm_step_remap(op, p.user_context) {
            Ok(OpRemapped { .. }) => 0,
            Err(err) => err.to_errno(),
        }
    }
}
