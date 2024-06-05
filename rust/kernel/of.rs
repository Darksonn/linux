// SPDX-License-Identifier: GPL-2.0

//! Devicetree and Open Firmware abstractions.
//!
//! C header: [`include/linux/of_*.h`](../../../../include/linux/of_*.h)

use crate::{
    bindings,
    device::Device,
    device_id::RawDeviceId,
    prelude::*,
    str::{BStr, CString},
    types::{ARef, AlwaysRefCounted, Opaque},
};

use core::ptr;

/// An open firmware device id.
#[derive(Clone, Copy)]
pub struct DeviceId(pub &'static BStr);

/// Defines a const open firmware device id table that also carries per-entry data/context/info.
///
/// The name of the const is `OF_DEVICE_ID_TABLE`, which is what buses are expected to name their
/// open firmware tables.
///
/// # Examples
///
/// ```
/// # use kernel::define_of_id_table;
/// use kernel::of;
///
/// define_of_id_table! {u32, [
///     (of::DeviceId::Compatible(b"test-device1,test-device2"), Some(0xff)),
///     (of::DeviceId::Compatible(b"test-device3"), None),
/// ]};
/// ```
#[macro_export]
macro_rules! define_of_id_table {
    ($data_type:ty, $($t:tt)*) => {
        type IdInfo = $data_type;
        const OF_DEVICE_ID_TABLE: $crate::device_id::IdTable<'static, $crate::of::DeviceId, $data_type> = {
            $crate::define_id_array!(ARRAY, $crate::of::DeviceId, $data_type, $($t)* );
            ARRAY.as_table()
        };
    };
}
pub use define_of_id_table;

// SAFETY: `ZERO` is all zeroed-out and `to_rawid` stores `offset` in `of_device_id::data`.
unsafe impl RawDeviceId for DeviceId {
    type RawType = bindings::of_device_id;
    const ZERO: Self::RawType = bindings::of_device_id {
        name: [0; 32],
        type_: [0; 32],
        compatible: [0; 128],
        data: core::ptr::null(),
    };
}

impl DeviceId {
    #[doc(hidden)]
    pub const fn to_rawid(&self, offset: isize) -> <Self as RawDeviceId>::RawType {
        let mut id = Self::ZERO;
        let mut i = 0;
        while i < self.0.len() {
            // If `compatible` does not fit in `id.compatible`, an "index out of bounds" build time
            // error will be triggered.
            id.compatible[i] = self.0.deref_const()[i] as _;
            i += 1;
        }
        id.compatible[i] = b'\0' as _;
        id.data = offset as _;
        id
    }
}

/// OF Device node property.
///
/// # Invariants
///
/// The pointer stored in `Self` is non-null and valid for the lifetime of the `DeviceNode`
/// instance.
#[cfg(CONFIG_OF)]
#[repr(transparent)]
pub struct Property(*mut bindings::property);

#[cfg(CONFIG_OF)]
impl Property {
    /// Creates a reference to a [`Property`] from a pointer.
    fn from_ptr(ptr: *mut bindings::property) -> Result<Self> {
        if ptr.is_null() {
            Err(ENODEV)
        } else {
            // SAFETY: The safety requirements guarantee the validity of the pointer.
            //
            // INVARIANT: The refcount isn't required to be managed for this and the C API guarantees
            // that this property will never be freed.
            Ok(Self(ptr))
        }
    }
}

/// OF Device node.
///
/// # Invariants
///
/// The pointer stored in `Self` is non-null and valid for the lifetime of the ARef instance. In
/// particular, the ARef instance owns an increment on underlying object’s reference count.
#[cfg(CONFIG_OF)]
#[repr(transparent)]
pub struct DeviceNode(Opaque<bindings::device_node>);

// SAFETY: `DeviceNode` only holds a pointer to a C DeviceNode, which is safe to be used from any
// thread.
#[cfg(CONFIG_OF)]
unsafe impl Send for DeviceNode {}

// SAFETY: `DeviceNode` only holds a pointer to a C DeviceNode, references to which are safe to be
// used from any thread.
#[cfg(CONFIG_OF)]
unsafe impl Sync for DeviceNode {}

// SAFETY: The type invariants guarantee that [`DeviceNode`] is always refcounted.
#[cfg(CONFIG_OF)]
unsafe impl AlwaysRefCounted for DeviceNode {
    fn inc_ref(&self) {
        // SAFETY: The existence of a shared reference means that the refcount is nonzero.
        unsafe { bindings::of_node_get(self.0.get()) };
    }

    unsafe fn dec_ref(obj: ptr::NonNull<Self>) {
        // SAFETY: The safety requirements guarantee that the refcount is nonzero.
        unsafe { bindings::of_node_put(obj.cast().as_ptr()) }
    }
}

#[cfg(CONFIG_OF)]
impl DeviceNode {
    /// Creates a reference to a [`DeviceNode`] from a valid pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is valid and remains valid for the lifetime of the
    /// returned [`DeviceNode`] reference.
    pub unsafe fn from_ptr(ptr: *mut bindings::device_node) -> Result<ARef<Self>> {
        // SAFETY: By the safety requirements, ptr is valid.
        let ptr = unsafe { bindings::of_node_get(ptr) };

        let np = ptr::NonNull::new(ptr).ok_or(ENODEV)?;

        // SAFETY: The safety requirements guarantee the validity of the pointer.
        //
        // INVARIANT: The refcount is already incremented by the C API that returned the pointer,
        // and we pass ownership of the refcount to the new `ARef<DeviceNode>`.
        Ok(unsafe { ARef::from_raw(np.cast()) })
    }

    /// Creates a reference to a [`DeviceNode`] from the device structure.
    pub fn from_dev(dev: &Device) -> Result<ARef<Self>> {
        // SAFETY: The raw device pointer is guaranteed to be valid.
        unsafe { Self::from_ptr(bindings::dev_of_node(dev.as_raw())) }
    }

    /// Returns the property associated with the device node.
    pub fn find_property(&self, name: &CString) -> Result<Property> {
        // SAFETY: The OF node is guaranteed by the C code to be valid.
        let pp = unsafe {
            bindings::of_find_property(self.0.get(), name.as_ptr() as *mut _, ptr::null_mut())
        };
        Property::from_ptr(pp)
    }
}
