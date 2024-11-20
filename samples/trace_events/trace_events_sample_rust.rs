/* SPDX-License-Identifier: GPL-2.0 */

//! Exposes a Rust function that calls a tracepoint.
//!
//! Most of the logic is in the `.c` file.

use core::ffi::c_char;
use kernel::{c_str, str::CStr};

// Declare that we wish to use one of the C tracepoints. The signature must match the C declaration
// exactly.
kernel::declare_trace! {
    /// # Safety
    ///
    /// `foo` must be a nul-terminated string.
    unsafe fn foo_bar_with_cond(s: *const c_char, cnt: i32);
}

/// Tracepoints often accept raw C types, so a helper provides a safe way to call the tracepoint.
fn trace_foo_bar_with_cond(s: &CStr, cnt: i32) {
    // SAFETY: A `&CStr` always contains a nul-terminated string.
    unsafe { foo_bar_with_cond(s.as_char_ptr(), cnt) };
}

/// Entry-point for C code to call into Rust.
#[no_mangle]
pub extern "C" fn trigger_tracepoint_from_rust(cnt: i32) {
    trace_foo_bar_with_cond(c_str!("Some times print from Rust"), cnt);
}
