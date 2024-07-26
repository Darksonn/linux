// SPDX-License-Identifier: GPL-2.0-only OR MIT

//! Generic Dynamic Voltage and Frequency Scaling (DVFS) Framework for Non-CPU Devices.

use core::{
    ffi::{c_int, c_ulong, c_void},
    mem::MaybeUninit,
    ptr::{addr_of_mut, drop_in_place, null_mut},
};
use kernel::{
    bindings,
    error::{from_err_ptr, to_result, Error, Result},
    prelude::*,
    str::CStr,
    types::{ForeignOwnable, Opaque},
};

/// Fields for using the simple on-demand governor.
pub type SimpleOnDemandDataFields = bindings::devfreq_simple_ondemand_data;

/// Fields for the devfreq profile.
pub struct DevfreqProfileFields {
    /// The polling frequency.
    pub polling_ms: u64,
    /// The initial frequency.
    pub initial_freq: u64,
}

impl DevfreqProfileFields {
    fn into_raw<P: DevFreqProfile>(self) -> bindings::devfreq_dev_profile {
        bindings::devfreq_dev_profile {
            //timer: bindings::DEVFREQ_TIMER_DELAYED as _,
            initial_freq: self.initial_freq as _,
            polling_ms: self.polling_ms as _,
            target: Some(target::<P>),
            get_dev_status: Some(get_dev_status::<P>),
            // SAFETY: The remaining fields are allowed to be zeroed.
            ..unsafe { MaybeUninit::zeroed().assume_init() }
        }
    }
}

/// docs go here
///
/// # Invariants
///
/// `devfreq` is a valid devfreq device using `profile` and `gov_data` as its profile and governor
/// data respectively.
#[pin_data(PinnedDrop)]
pub struct DevFreq<G> {
    devfreq: *mut bindings::devfreq,
    #[pin]
    profile: Opaque<bindings::devfreq_dev_profile>,
    #[pin]
    gov_data: Opaque<G>,
}

impl<G: GovernorData> DevFreq<G> {
    /// Create a new `DevFreq`.
    ///
    /// # Safety
    ///
    /// The `dev` pointer must point at a valid device, and it must not be destroyed before this
    /// `DevFreq` is destroyed.
    ///
    /// The driver data of `dev` must be compatible with `P::DriverData`.
    pub unsafe fn new_with_raw_device<P: DevFreqProfile>(
        dev: *mut bindings::device,
        gov_data: G,
        profile_fields: DevfreqProfileFields,
    ) -> impl PinInit<Self, Error> {
        // SAFETY: The closure will not leave `slot` in an invalid state.
        unsafe {
            crate::init::pin_init_from_closure(move |slot: *mut DevFreq<G>| {
                let profile = profile_fields.into_raw::<P>();

                let profile_ptr = Opaque::raw_get(addr_of_mut!((*slot).profile));
                let gov_data_ptr = Opaque::raw_get(addr_of_mut!((*slot).gov_data));

                (*gov_data_ptr) = gov_data;
                (*profile_ptr) = profile;

                // SAFETY: All of `dev`, `profile`, and `gov_data` must remain valid for the
                // lifetime of the resulting `devfreq`. For `dev` this is promised by the caller.
                // For `profile` and `gov_data`, this is ensured because this type is pinned, which
                // means that this `DevFreq` must remain at this address until its destructor runs.
                //
                // The implementation of `GovernorData` promises that the type behind
                // `gov_data_ptr` is compatible with the governor of the requested name.
                let devfreq = bindings::devm_devfreq_add_device(
                    dev,
                    profile_ptr,
                    (*gov_data_ptr).governor_name().as_char_ptr(),
                    gov_data_ptr as *mut c_void,
                );

                match from_err_ptr(devfreq) {
                    Ok(devfreq) => {
                        // SAFETY: The slot is valid for writing.
                        (*slot).devfreq = devfreq;
                        Ok(())
                    }
                    Err(err) => {
                        // SAFETY: Initialization failed, so these values may be destroyed.
                        drop_in_place(profile_ptr);
                        drop_in_place(gov_data_ptr);
                        Err(err)
                    }
                }
            })
        }
    }

    /// Resume devfreq of this device.
    pub fn resume_device(&self) -> Result<()> {
        // SAFETY: `self.devfreq` is a valid devfreq instance by the type invariants.
        to_result(unsafe { bindings::devfreq_resume_device(self.devfreq) })
    }

    /// Suspend devfreq of this device.
    pub fn suspend_device(&self) -> Result<()> {
        // SAFETY: `self.devfreq` is a valid devfreq instance by the type invariants.
        to_result(unsafe { bindings::devfreq_suspend_device(self.devfreq) })
    }

    /// Register cooling device.
    pub fn cooling_em_register(&self) -> Result<()> {
        let ptr = unsafe { bindings::devfreq_cooling_em_register(self.devfreq, null_mut()) };
        from_err_ptr(ptr).map(|_ptr| ())
    }
}

#[pinned_drop]
impl<G> PinnedDrop for DevFreq<G> {
    fn drop(self: Pin<&mut Self>) {
        // TODO: Implement a destructor.
        kernel::build_error("Must not destroy DevFreq");
    }
}

/// Data passed to the governor.
///
/// # Safety
///
/// This value must be compatible with the governor of the given name.
pub unsafe trait GovernorData {
    /// The name of the governor that this data works with.
    fn governor_name(&self) -> &CStr;
}

/// Data used for the on-demand governor.
#[repr(transparent)]
pub struct SimpleOnDemandData {
    inner: bindings::devfreq_simple_ondemand_data,
}

impl SimpleOnDemandData {
    /// Create a new `SimpleOnDemandData`.
    pub fn new(value: SimpleOnDemandDataFields) -> Self {
        Self { inner: value }
    }
}

// SAFETY: The governor data for `DEVFREQ_GOV_SIMPLE_ONDEMAND` is `devfreq_simple_ondemand_data`.
unsafe impl GovernorData for SimpleOnDemandData {
    fn governor_name(&self) -> &CStr {
        // SAFETY: The `DEVFREQ_GOV_SIMPLE_ONDEMAND` constant is a nul-terminated string.
        unsafe { CStr::from_char_ptr(bindings::DEVFREQ_GOV_SIMPLE_ONDEMAND.as_ptr().cast()) }
    }
}

/// Type used for out-parameter of `DevFreqProfile::get_dev_status`.
pub type DevStatus = bindings::devfreq_dev_status;

/// The profile for this devfreq.
pub trait DevFreqProfile {
    /// The driver data.
    type DriverData: ForeignOwnable;

    /// Returns the device status.
    fn get_dev_status(
        data: <Self::DriverData as ForeignOwnable>::Borrowed<'_>,
        status_out: &mut DevStatus,
    ) -> Result<()>;
}

/// Helper for populating `get_dev_status` in `devfreq_dev_profile`.
unsafe extern "C" fn get_dev_status<P: DevFreqProfile>(
    dev: *mut bindings::device,
    status: *mut bindings::devfreq_dev_status,
) -> c_int {
    // SAFETY: Caller provides a valid device.
    let drv_data_raw = unsafe { (*dev).driver_data };
    // SAFETY: It's okay to access the driver data in this callback.
    let drv_data = unsafe { <P::DriverData as ForeignOwnable>::borrow(drv_data_raw) };
    // SAFETY: Caller provides a valid, writable pointer as out-parameter.
    let status = unsafe { &mut *status };

    match P::get_dev_status(drv_data, status) {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

/// Helper for populating `target` in `devfreq_dev_profile`.
///
/// TODO: Make this customizable via the trait.
unsafe extern "C" fn target<P: DevFreqProfile>(
    dev: *mut bindings::device,
    freq: *mut c_ulong,
    flags: u32,
) -> c_int {
    // SAFETY: TODO, I have no idea what this does.
    let opp = unsafe { bindings::devfreq_recommended_opp(dev, freq, flags) };
    let opp = match from_err_ptr(opp) {
        Ok(opp) => opp,
        Err(err) => return err.to_errno(),
    };

    // SAFETY: TODO
    unsafe { bindings::dev_pm_opp_put(opp) };

    // SAFETY: TODO
    return unsafe { bindings::dev_pm_opp_set_rate(dev, *freq) };
}
