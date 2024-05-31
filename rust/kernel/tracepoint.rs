// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2024 Google LLC.

//! Logic for tracepoints.

/// Declare the Rust entry point for a tracepoint.
#[macro_export]
macro_rules! declare_trace {
    ($($(#[$attr:meta])* $pub:vis fn $name:ident($($argname:ident : $argtyp:ty),* $(,)?);)*) => {$(
        $( #[$attr] )*
        #[inline(always)]
        $pub unsafe fn $name($($argname : $argtyp),*) {
            #[cfg(CONFIG_TRACEPOINTS)]
            {
                use $crate::bindings::*;

                // SAFETY: It's always okay to query the static key for a tracepoint.
                let should_trace = unsafe {
                    $crate::macros::paste! {
                        $crate::static_key::static_key_false!(
                            [< __tracepoint_ $name >],
                            $crate::bindings::tracepoint,
                            key
                        )
                    }
                };

                if should_trace {
                    $crate::macros::paste! {
                        // SAFETY: The caller guarantees that it is okay to call this tracepoint.
                        unsafe { [< rust_do_trace_ $name >]($($argname),*) };
                    }
                }
            }

            #[cfg(not(CONFIG_TRACEPOINTS))]
            {
                // If tracepoints are disabled, insert a trivial use of each argument
                // to avoid unused argument warnings.
                $( let _unused = $argname; )*
            }
        }
    )*}
}

pub use declare_trace;
