// SPDX-License-Identifier: GPL-2.0-only OR MIT
#![recursion_limit = "2048"]

//! Panthor.

use core::ffi::c_ulong;
use kernel::{bindings, types::Opaque};

mod devfreq;
use self::devfreq::PanthorDevfreq;

/// Wrapper around the panthor_device struct defined by C.
#[repr(transparent)]
struct PanthorDevice {
    inner: Opaque<bindings::panthor_device>,
}

impl PanthorDevice {
    fn as_raw(&self) -> *mut bindings::panthor_device {
        self.inner.get()
    }

    fn core_clk_get_rate(&self) -> c_ulong {
        // SAFETY: `as_raw` returns a pointer to a valid `panthor_device`.
        unsafe { bindings::clk_get_rate((*self.as_raw()).clks.core) }
    }

    fn devfreq(&self) -> &PanthorDevfreq {
        // SAFETY: `as_raw` returns a pointer to a valid `panthor_device`.
        let devfreq_ptr = unsafe { (*self.as_raw()).devfreq };
        // SAFETY: The `devfreq` pointer is always valid.
        unsafe { PanthorDevfreq::from_raw_devfreq(devfreq_ptr) }
    }
}
