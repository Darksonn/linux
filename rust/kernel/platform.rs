// SPDX-License-Identifier: GPL-2.0

//! Platform devices and drivers.
//!
//! Also called `platdev`, `pdev`.
//!
//! C header: [`include/linux/platform_device.h`](../../../../include/linux/platform_device.h)

use crate::{
    bindings, container_of, device, device_id, driver,
    error::{from_result, to_result, Result},
    of,
    str::CStr,
    types::{ARef, ForeignOwnable},
    ThisModule,
};

/// A registration of a platform driver.
pub type Registration<T> = driver::Registration<Adapter<T>>;

/// An adapter for the registration of platform drivers.
pub struct Adapter<T: Driver>(T);

impl<T: Driver> driver::DriverOps for Adapter<T> {
    type RegType = bindings::platform_driver;

    fn register(
        reg: &mut bindings::platform_driver,
        name: &'static CStr,
        module: &'static ThisModule,
    ) -> Result {
        // SAFETY: By the safety requirements of this function (defined in the trait definition),
        // `reg` is non-null and valid.
        let pdrv = &mut *reg;

        pdrv.driver.name = name.as_char_ptr();
        pdrv.probe = Some(Self::probe_callback);
        pdrv.remove = Some(Self::remove_callback);
        pdrv.driver.of_match_table = T::OF_DEVICE_ID_TABLE.as_ref();
        // SAFETY:
        //   - `pdrv` lives at least until the call to `platform_driver_unregister()` returns.
        //   - `name` pointer has static lifetime.
        //   - `module.0` lives at least as long as the module.
        //   - `probe()` and `remove()` are static functions.
        //   - `of_match_table` is either a raw pointer with static lifetime,
        //      as guaranteed by the [`device_id::IdTable`] type, or null.
        to_result(unsafe { bindings::__platform_driver_register(reg, module.0) })
    }

    fn unregister(reg: &mut bindings::platform_driver) {
        // SAFETY: By the safety requirements of this function (defined in the trait definition),
        // `reg` was passed (and updated) by a previous successful call to
        // `platform_driver_register`.
        unsafe { bindings::platform_driver_unregister(reg) };
    }
}

impl<T: Driver> Adapter<T> {
    fn get_id_info(dev: &Device) -> Option<&'static T::IdInfo> {
        let table = T::OF_DEVICE_ID_TABLE;

        // SAFETY: `table` has static lifetime, so it is valid for read. `dev` is guaranteed to be
        // valid while it's alive, so is the raw device returned by it.
        let id = unsafe { bindings::of_match_device(table.as_ref(), &((*(dev.as_raw())).dev)) };
        if id.is_null() {
            return None;
        }

        // SAFETY: `id` is a pointer within the static table, so it's always valid.
        let offset = unsafe { (*id).data };
        if offset.is_null() {
            return None;
        }

        // SAFETY: The offset comes from a previous call to `offset_from` in `IdArray::new`, which
        // guarantees that the resulting pointer is within the table.
        let ptr = unsafe {
            id.cast::<u8>()
                .offset(offset as _)
                .cast::<Option<T::IdInfo>>()
        };

        // SAFETY: The id table has a static lifetime, so `ptr` is guaranteed to be valid for read.
        #[allow(clippy::needless_borrow)]
        unsafe {
            (&*ptr).as_ref()
        }
    }

    extern "C" fn probe_callback(pdev: *mut bindings::platform_device) -> core::ffi::c_int {
        from_result(|| {
            // SAFETY: `pdev` is valid by the contract with the C code. `dev` is alive only for the
            // duration of this call, so it is guaranteed to remain alive for the lifetime of
            // `pdev`.
            // let mut dev = unsafe { Device::from_ptr(pdev) };
            let dev = unsafe { device::Device::from_raw(&mut (*pdev).dev) };
            let mut pdev = unsafe { Device::from_dev(dev) };

            let info = Self::get_id_info(&pdev);
            let data = T::probe(&mut pdev, info)?;
            // SAFETY: `pdev` is guaranteed to be a valid, non-null pointer.
            unsafe { bindings::platform_set_drvdata(pdev.as_raw(), data.into_foreign() as _) };
            Ok(0)
        })
    }

    extern "C" fn remove_callback(pdev: *mut bindings::platform_device) -> core::ffi::c_int {
        from_result(|| {
            // SAFETY: `pdev` is guaranteed to be a valid, non-null pointer.
            let ptr = unsafe { bindings::platform_get_drvdata(pdev) };
            // SAFETY:
            //   - we allocated this pointer using `T::Data::into_foreign`,
            //     so it is safe to turn back into a `T::Data`.
            //   - the allocation happened in `probe`, no-one freed the memory,
            //     `remove` is the canonical kernel location to free driver data. so OK
            //     to convert the pointer back to a Rust structure here.
            let data = unsafe { T::Data::from_foreign(ptr) };
            T::remove(&data)?;
            Ok(0)
        })
    }
}

/// A platform driver.
pub trait Driver {
    /// Data stored on device by driver.
    ///
    /// Corresponds to the data set or retrieved via the kernel's
    /// `platform_{set,get}_drvdata()` functions.
    ///
    /// Require that `Data` implements `ForeignOwnable`. We guarantee to
    /// never move the underlying wrapped data structure. This allows
    type Data: ForeignOwnable + Send + Sync;

    /// The type holding information about each device id supported by the driver.
    type IdInfo: 'static;

    /// The table of device ids supported by the driver.
    const OF_DEVICE_ID_TABLE: device_id::IdTable<'static, of::DeviceId, Self::IdInfo>;

    /// Platform driver probe.
    ///
    /// Called when a new platform device is added or discovered.
    /// Implementers should attempt to initialize the device here.
    fn probe(dev: &mut Device, id_info: Option<&Self::IdInfo>) -> Result<Self::Data>;

    /// Platform driver remove.
    ///
    /// Called when a platform device is removed.
    /// Implementers should prepare the device for complete removal here.
    fn remove(_data: &Self::Data) -> Result {
        Ok(())
    }
}

/// A platform device.
///
#[derive(Clone)]
pub struct Device(ARef<device::Device>);

impl Device {
    /// Create a PCI Device instance from an existing `device::Device`.
    ///
    /// # Safety
    ///
    /// `dev` must be an `ARef<device::Device>` whose underlying `bindings::device` is a member of
    /// a `bindings::platform_device`.
    pub unsafe fn from_dev(dev: ARef<device::Device>) -> Self {
        Self(dev)
    }

    fn as_raw(&self) -> *mut bindings::platform_device {
        // SAFETY: Guaranteed by the requirements described in pci::Device::new().
        unsafe { container_of!(self.0.as_raw(), bindings::platform_device, dev) as _ }
    }

    /// Returns id of the platform device.
    pub fn id(&self) -> i32 {
        // SAFETY: By the type invariants, we know that `self.ptr` is non-null and valid.
        unsafe { (*self.as_raw()).id }
    }
}

impl AsRef<device::Device> for Device {
    fn as_ref(&self) -> &device::Device {
        &self.0
    }
}

/// Declares a kernel module that exposes a single platform driver.
///
/// # Examples
///
/// ```ignore
/// # use kernel::{platform, define_of_id_table, module_platform_driver};
/// #
/// struct MyDriver;
/// impl platform::Driver for MyDriver {
///     // [...]
/// #   fn probe(_dev: &mut platform::Device, _id_info: Option<&Self::IdInfo>) -> Result {
/// #       Ok(())
/// #   }
/// #   define_of_id_table! {(), [
/// #       (of::DeviceId::Compatible(b"brcm,bcm2835-rng"), None),
/// #   ]}
/// }
///
/// module_platform_driver! {
///     type: MyDriver,
///     name: "module_name",
///     author: "Author name",
///     license: "GPL",
/// }
/// ```
#[macro_export]
macro_rules! module_platform_driver {
    ($($f:tt)*) => {
        $crate::module_driver!(<T>, $crate::platform::Adapter<T>, { $($f)* });
    };
}
