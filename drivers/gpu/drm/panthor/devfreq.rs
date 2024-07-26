// SPDX-License-Identifier: GPL-2.0-only OR MIT

use crate::PanthorDevice;
use core::{
    ffi::{c_int, c_ulong},
    mem::size_of,
};
use kernel::{
    bindings,
    devfreq::{
        DevFreq, DevFreqProfile, DevStatus, DevfreqProfileFields, SimpleOnDemandData,
        SimpleOnDemandDataFields,
    },
    error::Error,
    prelude::*,
    sync::SpinLock,
    time::Ktime,
};

#[pin_data]
pub(crate) struct PanthorDevfreq {
    #[pin]
    devfreq: DevFreq<SimpleOnDemandData>,
    // TODO: use irqsave spinlock
    #[pin]
    inner: SpinLock<Inner>,
}

struct Inner {
    busy_time: Ktime,
    idle_time: Ktime,
    time_last_update: Ktime,
    /// True if the GPU was busy last time we updated the state.
    last_busy_state: bool,
}

impl PanthorDevfreq {
    /// # Safety
    ///
    /// The `dev` pointer must point at a valid device, and it must not be destroyed before this
    /// `DevFreq` is destroyed. The driver data must be `PanthorDevice`.
    unsafe fn new_with_raw_device(
        dev: *mut bindings::device,
        initial_freq: u64,
    ) -> impl PinInit<Self, Error> {
        let profile = DevfreqProfileFields {
            polling_ms: 50, /* ~3 frames */
            initial_freq,
        };

        // Setup default thresholds for the simple_ondemand governor.
        // The values are chosen based on experiments.
        let gov_data = SimpleOnDemandData::new(SimpleOnDemandDataFields {
            upthreshold: 45,
            downdifferential: 5,
        });

        try_pin_init!(PanthorDevfreq {
            // SAFETY: Caller promises that `dev` is valid for long enough, and that the driver
            // data is `PanthorDevice`.
            devfreq <- unsafe { DevFreq::new_with_raw_device::<Self>(dev, gov_data, profile) },
            inner <- kernel::new_spinlock!(Inner::new()),
        })
    }

    /// # Safety
    ///
    /// The provided pointer must be a valid pointer to a `PanthorDevfreq` and must be valid for
    /// the duration of `'a`.
    pub(crate) unsafe fn from_raw_devfreq<'a>(ptr: *const bindings::panthor_devfreq) -> &'a Self {
        // SAFETY: The caller promises that the pointer is valid for 'a.
        unsafe { &*(ptr as *const Self) }
    }

    fn resume(&self) -> Result<()> {
        self.devfreq.resume_device()
    }

    fn suspend(&self) -> Result<()> {
        self.devfreq.suspend_device()
    }

    fn record_busy(&self) {
        // TODO: Use irqsave spinlock.
        let mut inner = self.inner.lock();
        inner.update_utilization();
        inner.last_busy_state = true;
    }

    fn record_idle(&self) {
        // TODO: Use irqsave spinlock.
        let mut inner = self.inner.lock();
        inner.update_utilization();
        inner.last_busy_state = false;
    }
}

impl DevFreqProfile for PanthorDevfreq {
    // TODO: Box is probably not the right smart pointer, but for now we just need something with
    // Borrowed = &PanthorDevice.
    type DriverData = Box<PanthorDevice>;

    fn get_dev_status(ptdev: &PanthorDevice, status: &mut DevStatus) -> Result<()> {
        let devfreq = ptdev.devfreq();
        status.current_frequency = ptdev.core_clk_get_rate();

        let mut inner = devfreq.inner.lock();
        inner.update_utilization();
        status.total_time = (inner.busy_time + inner.idle_time).to_ns() as c_ulong;
        status.busy_time = inner.busy_time.to_ns() as c_ulong;
        inner.reset();
        drop(inner);

        // TODO: print debug info
        Ok(())
    }
}

impl Inner {
    fn new() -> Self {
        Self {
            busy_time: Ktime::zero(),
            idle_time: Ktime::zero(),
            time_last_update: Ktime::ktime_get(),
            last_busy_state: false,
        }
    }

    fn update_utilization(&mut self) {
        let now = Ktime::ktime_get();
        let last = self.time_last_update;

        if self.last_busy_state {
            self.busy_time += now - last;
        } else {
            self.idle_time += now - last;
        }
    }

    fn reset(&mut self) {
        self.busy_time = Ktime::zero();
        self.idle_time = Ktime::zero();
        self.time_last_update = Ktime::ktime_get();
    }
}

// ===== exports to C code =====
// To be deleted when all modules that call into devfreq are converted to Rust.

#[no_mangle]
static PANTHOR_DEVFREQ_SIZEOF: usize = size_of::<PanthorDevfreq>();

#[no_mangle]
unsafe extern "C" fn panthor_devfreq_init_rust(
    slot: *mut bindings::panthor_devfreq,
    ptdev: *mut bindings::panthor_device,
    initial_freq: u64,
) -> c_int {
    let slot = slot as *mut PanthorDevfreq;

    // SAFETY: The `dev` pointer is valid for long enough and the type of the driver data is
    // `PanthorDevice`.
    let initializer =
        unsafe { PanthorDevfreq::new_with_raw_device((*ptdev).base.dev, initial_freq) };

    // SAFETY: `slot` is a pointer to an uninitialized region of memory that has space for a
    // `PanthorDevfreq`.
    let res = unsafe { initializer.__pinned_init(slot) };

    match res {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

#[no_mangle]
unsafe extern "C" fn panthor_devfreq_cooling_register(
    devfreq: *mut bindings::panthor_devfreq,
) -> c_int {
    // SAFETY: The provided pointer is valid.
    let res = unsafe { PanthorDevfreq::from_raw_devfreq(devfreq) }
        .devfreq
        .cooling_em_register();

    match res {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

#[no_mangle]
unsafe extern "C" fn panthor_devfreq_resume(ptdev: *mut bindings::panthor_device) -> c_int {
    // SAFETY: The caller passes a valid pointer.
    let pdevfreq = unsafe { (*ptdev).devfreq };
    if pdevfreq.is_null() {
        return 0;
    }

    // SAFETY: `ptdev->devfreq` is always null or valid, and we just checked for null.
    let res = unsafe { PanthorDevfreq::from_raw_devfreq(pdevfreq) }.resume();

    match res {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

#[no_mangle]
unsafe extern "C" fn panthor_devfreq_suspend(ptdev: *mut bindings::panthor_device) -> c_int {
    // SAFETY: The caller passes a valid pointer.
    let pdevfreq = unsafe { (*ptdev).devfreq };
    if pdevfreq.is_null() {
        return 0;
    }

    // SAFETY: `ptdev->devfreq` is always null or valid, and we just checked for null.
    let res = unsafe { PanthorDevfreq::from_raw_devfreq(pdevfreq) }.suspend();

    match res {
        Ok(()) => 0,
        Err(err) => err.to_errno(),
    }
}

#[no_mangle]
unsafe extern "C" fn panthor_devfreq_record_busy(ptdev: *mut bindings::panthor_device) {
    // SAFETY: The caller passes a valid pointer.
    let pdevfreq = unsafe { (*ptdev).devfreq };
    if pdevfreq.is_null() {
        return;
    }

    // SAFETY: `ptdev->devfreq` is always null or valid, and we just checked for null.
    unsafe { PanthorDevfreq::from_raw_devfreq(pdevfreq) }.record_busy();
}

#[no_mangle]
unsafe extern "C" fn panthor_devfreq_record_idle(ptdev: *mut bindings::panthor_device) {
    // SAFETY: The caller passes a valid pointer.
    let pdevfreq = unsafe { (*ptdev).devfreq };
    if pdevfreq.is_null() {
        return;
    }

    // SAFETY: `ptdev->devfreq` is always null or valid, and we just checked for null.
    unsafe { PanthorDevfreq::from_raw_devfreq(pdevfreq) }.record_idle();
}
