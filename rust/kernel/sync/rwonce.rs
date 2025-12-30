// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2025 Google LLC.

//! Rust version of the raw `READ_ONCE`/`WRITE_ONCE` functions.
//!
//! C header: [`include/asm-generic/rwonce.h`](srctree/include/asm-generic/rwonce.h)

/// Read the pointer once.
///
/// # Safety
///
/// It must be safe to `READ_ONCE` the `ptr` with this type.
#[inline(always)]
#[must_use]
#[track_caller]
#[expect(non_snake_case)]
pub unsafe fn READ_ONCE<T: RwOnceType>(ptr: *const T) -> T {
    // SAFETY: It's safe to read `ptr` once with this type.
    unsafe { T::read_once(ptr) }
}

/// Write the pointer once.
///
/// # Safety
///
/// It must be safe to `WRITE_ONCE` the `ptr` with this type.
#[inline(always)]
#[track_caller]
#[expect(non_snake_case)]
pub unsafe fn WRITE_ONCE<T: RwOnceType>(ptr: *mut T, val: T) {
    // SAFETY: It's safe to write `ptr` once with this type.
    unsafe { T::write_once(ptr, val) };
}

/// This module contains the generic implementations.
#[expect(clippy::undocumented_unsafe_blocks)]
#[expect(clippy::missing_safety_doc)]
mod rwonce_generic_impl {
    use core::ffi::c_void;
    #[allow(unused_imports)]
    use core::ptr::{read_volatile, write_volatile};

    #[inline(always)]
    #[track_caller]
    #[cfg(not(CONFIG_ARCH_USE_CUSTOM_READ_ONCE))]
    pub(super) unsafe fn read_once_1(ptr: *const u8) -> u8 {
        unsafe { read_volatile::<u8>(ptr) }
    }

    #[inline(always)]
    #[track_caller]
    #[cfg(not(CONFIG_ARCH_USE_CUSTOM_READ_ONCE))]
    pub(super) unsafe fn read_once_2(ptr: *const u16) -> u16 {
        unsafe { read_volatile::<u16>(ptr) }
    }

    #[inline(always)]
    #[track_caller]
    #[cfg(not(CONFIG_ARCH_USE_CUSTOM_READ_ONCE))]
    pub(super) unsafe fn read_once_4(ptr: *const u32) -> u32 {
        unsafe { read_volatile::<u32>(ptr) }
    }

    #[inline(always)]
    #[track_caller]
    #[cfg(not(CONFIG_ARCH_USE_CUSTOM_READ_ONCE))]
    pub(super) unsafe fn read_once_8(ptr: *const u64) -> u64 {
        unsafe { read_volatile::<u64>(ptr) }
    }

    #[inline(always)]
    #[track_caller]
    #[cfg(not(CONFIG_ARCH_USE_CUSTOM_READ_ONCE))]
    pub(super) unsafe fn read_once_ptr(ptr: *const *mut c_void) -> *mut c_void {
        unsafe { read_volatile::<*mut c_void>(ptr) }
    }

    #[inline(always)]
    #[track_caller]
    pub(super) unsafe fn write_once_1(ptr: *mut u8, val: u8) {
        unsafe { write_volatile::<u8>(ptr, val) }
    }

    #[inline(always)]
    #[track_caller]
    pub(super) unsafe fn write_once_2(ptr: *mut u16, val: u16) {
        unsafe { write_volatile::<u16>(ptr, val) }
    }

    #[inline(always)]
    #[track_caller]
    pub(super) unsafe fn write_once_4(ptr: *mut u32, val: u32) {
        unsafe { write_volatile::<u32>(ptr, val) }
    }

    #[inline(always)]
    #[track_caller]
    pub(super) unsafe fn write_once_8(ptr: *mut u64, val: u64) {
        unsafe { write_volatile::<u64>(ptr, val) }
    }

    #[inline(always)]
    #[track_caller]
    pub(super) unsafe fn write_once_ptr(ptr: *mut *mut c_void, val: *mut c_void) {
        unsafe { write_volatile::<*mut c_void>(ptr, val) }
    }
}
use rwonce_generic_impl::*;

#[cfg(CONFIG_ARCH_USE_CUSTOM_READ_ONCE)]
use bindings::{read_once_1, read_once_2, read_once_4, read_once_8, read_once_ptr};

/// Rust trait for types that may be used with `READ_ONCE`/`WRITE_ONCE`.
///
/// This serves a similar purpose to the `compiletime_assert_rwonce_type` macro in the C header.
pub trait RwOnceType {
    /// The `READ_ONCE` for this type.
    ///
    /// # Safety
    ///
    /// It must be safe to `READ_ONCE` the `ptr` with this type.
    unsafe fn read_once(ptr: *const Self) -> Self;

    /// The `WRITE_ONCE` for this type.
    ///
    /// # Safety
    ///
    /// It must be safe to `WRITE_ONCE` the `ptr` with this type.
    unsafe fn write_once(ptr: *mut Self, val: Self);
}

macro_rules! impl_rw_once_type {
    ($($t:ty, $read:ident, $write:ident $(, <$u:ident>)?;)*) => {$(
        #[allow(unknown_lints, reason = "unnecessary_transmutes is unknown prior to MSRV 1.88.0")]
        #[allow(unnecessary_transmutes)]
        #[allow(clippy::missing_transmute_annotations)]
        #[allow(clippy::useless_transmute)]
        impl$(<$u>)? RwOnceType for $t {
            #[inline(always)]
            #[track_caller]
            unsafe fn read_once(ptr: *const Self) -> Self {
                // SAFETY: The caller ensures we can `READ_ONCE`.
                //
                // Note that `transmute` fails to compile if the two types are of different sizes.
                unsafe { core::mem::transmute($read(ptr.cast())) }
            }

            #[inline(always)]
            #[track_caller]
            unsafe fn write_once(ptr: *mut Self, val: Self) {
                // SAFETY: The caller ensures we can `WRITE_ONCE`.
                unsafe { $write(ptr.cast(), core::mem::transmute(val)) };
            }
        }
    )*}
}

// These macros determine which types may be used with rwonce, and which helper function should be
// used if so.
//
// Note that `core::mem::transmute` fails the build if the source and target type have different
// sizes, so picking the wrong helper should lead to a build error.

impl_rw_once_type! {
    bool, read_once_bool, write_once_1;
    u8,   read_once_1, write_once_1;
    i8,   read_once_1, write_once_1;
    u16,  read_once_2, write_once_2;
    i16,  read_once_2, write_once_2;
    u32,  read_once_4, write_once_4;
    i32,  read_once_4, write_once_4;
    u64,  read_once_8, write_once_8;
    i64,  read_once_8, write_once_8;
    *mut T, read_once_ptr, write_once_ptr, <T>;
    *const T, read_once_ptr, write_once_ptr, <T>;
}

#[cfg(target_pointer_width = "32")]
impl_rw_once_type! {
    usize, read_once_4, write_once_4;
    isize, read_once_4, write_once_4;
}

#[cfg(target_pointer_width = "64")]
impl_rw_once_type! {
    usize, read_once_8, write_once_8;
    isize, read_once_8, write_once_8;
}

/// Read an integer as a boolean once.
///
/// Returns `true` if the value behind the pointer is non-zero. Otherwise returns `false`.
///
/// # Safety
///
/// It must be safe to `READ_ONCE` the `ptr` with type `u8`.
#[inline(always)]
#[track_caller]
unsafe fn read_once_bool(ptr: *const bool) -> bool {
    // Implement `read_once_bool` in terms of `read_once_1`. The arch-specific logic is inside
    // of `read_once_1`.
    //
    // SAFETY: It is safe to `READ_ONCE` the `ptr` with type `u8`.
    let byte = unsafe { read_once_1(ptr.cast::<u8>()) };
    byte != 0u8
}
