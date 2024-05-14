// SPDX-License-Identifier: GPL-2.0

//! Devicetree and Open Firmware abstractions.
//!
//! C header: [`include/linux/of_*.h`](../../../../include/linux/of_*.h)

use crate::{
    bindings,
    device_id::RawDeviceId,
    str::BStr
};

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
