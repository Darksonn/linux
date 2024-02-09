//! A reimplementation of the `refcount_t` methods in Rust.
//!
//! This is defined in the bindings crate because this is intended as a drop-in
//! replacement for C `refcount_inc` and `refcount_dec_and_test` functions.

use crate::bindings_raw::*;
use core::ffi::c_int;
use core::sync::atomic::{self, Ordering};

use crate::bindings_raw::{
    refcount_saturation_type_REFCOUNT_ADD_OVF as REFCOUNT_ADD_OVF,
    refcount_saturation_type_REFCOUNT_ADD_UAF as REFCOUNT_ADD_UAF,
    refcount_saturation_type_REFCOUNT_SUB_UAF as REFCOUNT_SUB_UAF,
};

// Use a trait to pick the right atomic type for c_int.
trait HasAtomic {
    type AtomicInt;
}
impl HasAtomic for i16 {
    type AtomicInt = atomic::AtomicI16;
}
impl HasAtomic for i32 {
    type AtomicInt = atomic::AtomicI32;
}
impl HasAtomic for i64 {
    type AtomicInt = atomic::AtomicI64;
}
impl HasAtomic for isize {
    type AtomicInt = atomic::AtomicIsize;
}

type AtomicCInt = <c_int as HasAtomic>::AtomicInt;

/// Create a new `refcount_t` with the given initial refcount.
///
/// # Safety
///
/// This method is safe to call.
#[inline(always)]
pub unsafe fn REFCOUNT_INIT(n: c_int) -> refcount_t {
    refcount_t {
        refs: atomic_t { counter: n },
    }
}

/// Increment the refcount.
///
/// Saturates if the refcount wraps around.
///
/// # Safety
///
/// * The provided pointer must point at a valid `refcount_t`.
/// * The `refcount_t` may only be accessed concurrently by other atomic
///   operations defined in this file.
#[inline(always)]
pub unsafe fn refcount_inc(r: *mut refcount_t) {
    // SAFETY: All concurrent accesses agree that this is currently an
    // `AtomicCInt`.
    let atomic = unsafe { &*r.cast::<AtomicCInt>() };
    let old = atomic.fetch_add(1, Ordering::Relaxed);

    if old == 0 {
        // SAFETY: The caller guarantees that this is okay to call.
        unsafe { warn_saturate(r, REFCOUNT_ADD_UAF) };
    } else if old.wrapping_add(1) <= 0 {
        // SAFETY: The caller guarantees that this is okay to call.
        unsafe { warn_saturate(r, REFCOUNT_ADD_OVF) };
    }
}

/// Decrement the refcount and return whether we dropped it to zero.
///
/// If this returns `true`, then this call dropped the refcount to zero and
/// all previous operations on the refcount happens-before this call.
///
/// # Safety
///
/// * The provided pointer must point at a valid `refcount_t`.
/// * The `refcount_t` may only be accessed concurrently by other atomic
///   operations defined in this file.
#[inline(always)]
#[must_use]
pub unsafe fn refcount_dec_and_test(r: *mut refcount_t) -> bool {
    // SAFETY: All concurrent accesses agree that this is currently an
    // `AtomicCInt`.
    let atomic = unsafe { &*r.cast::<AtomicCInt>() };
    let old = atomic.fetch_sub(1, Ordering::Release);

    if old == 1 {
        atomic::fence(Ordering::Acquire);
        return true;
    }

    if old <= 0 {
        // SAFETY: The caller guarantees that this is okay to call.
        unsafe { warn_saturate(r, REFCOUNT_SUB_UAF) };
    }

    false
}

/// A helper function so that we can use #[cold] to hint to the branch predictor.
///
/// # Safety
///
/// * The provided pointer must point at a valid `refcount_t`.
/// * The `refcount_t` may only be accessed concurrently by other atomic
///   operations defined in this file.
#[cold]
unsafe fn warn_saturate(r: *mut refcount_t, t: refcount_saturation_type) {
    // SAFETY: Caller promises that `r` is not dangling.
    unsafe { refcount_warn_saturate(r, t) };
}
