// SPDX-License-Identifier: GPL-2.0

//! Bindings.
//!
//! Imports the generated bindings by `bindgen`.
//!
//! This crate may not be directly used. If you need a kernel C API that is
//! not ported or wrapped in the `kernel` crate, then do so first instead of
//! using this crate.

#![no_std]
// See <https://github.com/rust-lang/rust-bindgen/issues/1651>.
#![cfg_attr(test, allow(deref_nullptr))]
#![cfg_attr(test, allow(unaligned_references))]
#![cfg_attr(test, allow(unsafe_op_in_unsafe_fn))]
#![allow(
    clippy::all,
    missing_docs,
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    improper_ctypes,
    unreachable_pub,
    unsafe_op_in_unsafe_fn
)]

#[allow(dead_code)]
mod bindings_raw {
    // Use glob import here to expose all helpers.
    // Symbols defined within the module will take precedence to the glob import.
    pub use super::bindings_helper::*;
    include!(concat!(
        env!("OBJTREE"),
        "/rust/bindings/bindings_generated.rs"
    ));
}

// When both a directly exposed symbol and a helper exists for the same function,
// the directly exposed symbol is preferred and the helper becomes dead code, so
// ignore the warning here.
#[allow(dead_code)]
mod bindings_helper {
    // Import the generated bindings for types.
    use super::bindings_raw::*;
    include!(concat!(
        env!("OBJTREE"),
        "/rust/bindings/bindings_helpers_generated.rs"
    ));
}

pub use bindings_raw::*;

mod spin_unlocked {
    use super::{arch_spinlock_t, SPIN_UNLOCKED_RUST_HELPER};

    // This will copy the first `sizeof(arch_spinlock_t)` bytes from the provided argument to make
    // the value of `__ARCH_SPIN_LOCK_UNLOCKED`. Most of the times, the helper has the same size as
    // `arch_spinlock_t`, but there are some exceptions.
    //
    // The repetition is there for arches where `arch_spinlock_t` is large. In that case, the value
    // of the helper is repeated.
    //
    // SAFETY: The value is defined by the C side to be okay to transmute to `arch_spinlock_t`.
    // 64 repetitions are enough for all arches.
    pub const __ARCH_SPIN_LOCK_UNLOCKED: arch_spinlock_t = unsafe {
        core::mem::transmute_copy(&[SPIN_UNLOCKED_RUST_HELPER; 64])
    };
}
pub use self::spin_unlocked::__ARCH_SPIN_LOCK_UNLOCKED;
