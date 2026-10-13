// SPDX-License-Identifier: GPL-2.0

//! Scoped work items.
//!
//! Provides [`ScopedWork`] for work items that may borrow data with some non-`'static` lifetime.
//!
//! Unlike [`Work`]-based items, ownership of a [`ScopedWork`] is not transferred to
//! the workqueue. Instead, its destructor calls `cancel_work_sync()`, which guarantees that the
//! work function is not running (and will not run) once the destructor returns. This is what
//! allows the inner data to carry non-`'static` lifetimes.
//!
//! Creating a [`ScopedWork`] is `unsafe`, because the above guarantee only holds if the destructor
//! actually runs: the value must not be forgotten. Once created, a [`ScopedWork`] can be enqueued
//! on any queue using the safe [`Queue::enqueue_scoped`].
//!
//! # Examples
//!
//! ```
//! use kernel::workqueue::{
//!     new_scoped_work,
//!     Queue,
//!     ScopedWork,
//!     ScopedWorkItem,
//! };
//!
//! struct MyWork {
//!     value: u32,
//! }
//!
//! impl ScopedWorkItem for MyWork {
//!     fn run(work: &ScopedWork<Self>) {
//!         pr_info!("value = {}\n", work.value);
//!     }
//! }
//!
//! let work = KBox::pin_init(
//!     // SAFETY: `work` is not forgotten.
//!     unsafe { new_scoped_work!("MyWork", MyWork { value: 42 }) },
//!     GFP_KERNEL,
//! )?;
//!
//! let queue = Queue::new_unbound().build(c"my-wq")?;
//! queue.enqueue_scoped(&work);
//!
//! // Dropping `queue` first drains it, so the work item is guaranteed to have run by the time
//! // `work` is dropped.
//! drop(queue);
//! # Ok::<(), Error>(())
//! ```

use crate::{
    bindings,
    container_of,
    prelude::*,
    sync::LockClassKey,
    types::Opaque,
    workqueue::RawWorkItem, //
};

#[cfg(doc)]
use super::{
    Queue,
    Work, //
};

use pin_init::Wrapper;

use core::ops::Deref;

/// Trait for types that can be used as scoped work items.
///
/// Implementers define the work function that executes when the item is dequeued by a workqueue
/// thread. The callback receives a reference to the containing [`ScopedWork`], which provides
/// access to the inner data via [`Deref`] and can be passed to [`Queue::enqueue_scoped`] to
/// re-enqueue the work item from within the work function.
pub trait ScopedWorkItem: Sized {
    /// Called when the work item is executed.
    fn run(work: &ScopedWork<Self>);
}

/// The payload of a [`ScopedWork`].
///
/// # Invariants
///
/// `work` is an initialized `work_struct` whose work function is [`ScopedWork::<T>::run`]. An
/// `Inner<T>` only ever exists inside a [`ScopedWork<T>`].
#[pin_data]
struct Inner<T: ScopedWorkItem> {
    #[pin]
    work: Opaque<bindings::work_struct>,
    #[pin]
    data: T,
}

/// A scoped work item that cancels synchronously on drop.
///
/// `ScopedWork<T>` contains a `work_struct` and the user data `T`. Its destructor calls
/// `cancel_work_sync()`, guaranteeing the work function is not running when the data is dropped.
///
/// This allows `T` to carry non-`'static` lifetimes.
///
/// Construct via [`new_scoped_work!`], which returns an `impl PinInit` suitable for embedding
/// in-place inside other pinned structs. Because the soundness of [`Queue::enqueue_scoped`]
/// relies on the destructor running, [`ScopedWork::new`] is `unsafe` and requires that the
/// resulting value is never forgotten.
///
/// # Examples
///
/// Self-re-enqueueing from within the work function:
///
/// ```
/// # use kernel::sync::atomic::{Atomic, Relaxed};
/// # use kernel::time::{Delta, delay::fsleep};
/// use kernel::workqueue::{
///     self,
///     new_scoped_work,
///     Queue,
///     ScopedWork,
///     ScopedWorkItem,
/// };
///
/// struct RequeueWork<'a> {
///     counter: Atomic<u32>,
///     queue: &'a Queue,
/// }
///
/// impl ScopedWorkItem for RequeueWork<'_> {
///     fn run(work: &ScopedWork<Self>) {
///         if work.counter.fetch_add(1u32, Relaxed) < 2 {
///             work.queue.enqueue_scoped(work);
///         }
///     }
/// }
///
/// let queue = Queue::new_ordered().build(c"requeue_wq")?;
///
/// let work = KBox::pin_init(
///     // SAFETY: `work` is not forgotten.
///     unsafe {
///         new_scoped_work!(
///             "RequeueWork",
///             RequeueWork {
///                 counter: Atomic::new(0u32),
///                 queue: &queue,
///             }
///         )
///     },
///     GFP_KERNEL,
/// )?;
///
/// queue.enqueue_scoped(&work);
/// # fsleep(Delta::from_millis(300));
///
/// assert_eq!(work.counter.load(Relaxed), 3);
/// # Ok::<(), Error>(())
/// ```
#[pin_data(PinnedDrop)]
#[repr(transparent)]
pub struct ScopedWork<T: ScopedWorkItem> {
    /// The whole payload is wrapped in [`Opaque`] on purpose: while the destructor waits in
    /// `cancel_work_sync()`, a worker thread may still be executing [`ScopedWorkItem::run`] with a
    /// shared reference to this value, concurrently with the `&mut Self` held by `drop`. [`Opaque`]
    /// opts out of the uniqueness guarantee of that `&mut`. Wrapping individual fields would not be
    /// enough, since the `&mut Self` would still claim exclusive access to the padding bytes.
    ///
    /// # Invariants
    ///
    /// `inner` is always initialized.
    #[pin]
    inner: Opaque<Inner<T>>,
}

// SAFETY: A `&ScopedWork<T>` only hands out `&T` (via `Deref`) and a pointer to the `work_struct`,
// which is only ever accessed through the C workqueue API. Hence sharing it is safe when `T: Sync`.
unsafe impl<T: ScopedWorkItem + Sync> Sync for ScopedWork<T> {}

// SAFETY: The `work_struct` is only ever accessed through the C workqueue API, which is safe to
// use from any thread. Sending the contained `T` requires `T: Send`.
unsafe impl<T: ScopedWorkItem + Send> Send for ScopedWork<T> {}

impl<T: ScopedWorkItem> Deref for ScopedWork<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: `inner` is always initialized (type invariant), and `data` is only ever accessed
        // through shared references.
        unsafe { &(*self.inner.get()).data }
    }
}

impl<T: ScopedWorkItem> ScopedWork<T> {
    /// Creates a pin-initializer for a new scoped work item.
    ///
    /// Use [`new_scoped_work!`] to automatically provide the lock class key.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the destructor of the initialized `ScopedWork<T>` runs before
    /// the memory it lives in is reused or any lifetime in `T` expires. In particular, the value
    /// must not be leaked, e.g. via [`core::mem::forget`] or by leaking the allocation that
    /// contains it.
    #[inline]
    pub unsafe fn new<E>(
        name: &'static CStr,
        key: Pin<&'static LockClassKey>,
        init: impl PinInit<T, E>,
    ) -> impl PinInit<Self, E> {
        try_pin_init!(Self {
            inner <- Opaque::pin_init(try_pin_init!(Inner::<T> {
                work <- Opaque::try_ffi_init(|slot| {
                    // SAFETY: `slot` is a valid pointer to uninitialized memory for a
                    // `work_struct`, and `Self::run` is a valid work function for a `work_struct`
                    // embedded in an `Inner<T>` that is embedded in a `ScopedWork<T>`.
                    unsafe {
                        bindings::init_work_with_key(
                            slot,
                            Some(Self::run),
                            false,
                            name.as_char_ptr(),
                            key.as_ptr(),
                        )
                    };
                    Ok::<(), E>(())
                }),
                data <- init,
            }? E)),
        }? E)
    }

    /// Returns a raw pointer to the `work_struct`.
    #[inline]
    fn raw_work(&self) -> *mut bindings::work_struct {
        // SAFETY: `inner` is always initialized (type invariant).
        unsafe { Opaque::cast_into(&raw const (*self.inner.get()).work) }
    }

    /// The work function registered with the C workqueue.
    ///
    /// # Safety
    ///
    /// `ptr` must point to the `work` field of the `Inner<T>` of a live `ScopedWork<T>`.
    unsafe extern "C" fn run(ptr: *mut bindings::work_struct) {
        // SAFETY: `Opaque<T>` is `#[repr(transparent)]`, and by the safety requirements `ptr`
        // points to the `work` field of an `Inner<T>`.
        let inner =
            unsafe { container_of!(ptr.cast::<Opaque<bindings::work_struct>>(), Inner<T>, work) };

        // SAFETY: `Inner<T>` only ever exists inside a `ScopedWork<T>` (type invariant), and both
        // `Opaque` and `ScopedWork` are `#[repr(transparent)]`, so `inner` is also a valid pointer
        // to the containing `ScopedWork<T>`. By the safety requirements it is live; its destructor
        // calls `cancel_work_sync()` before the memory is released, so it stays live for the
        // duration of this call.
        T::run(unsafe { &*inner.cast::<Self>() });
    }
}

// SAFETY: `&ScopedWork<T>` points to a `work_struct` whose work function is
// `ScopedWork::<T>::run` (type invariant), so passing it to `queue_work_on` is sound. The pointer
// stays valid until the work function has run or the item is cancelled, because the `ScopedWork`
// destructor calls `cancel_work_sync()` and `ScopedWork::new` requires that destructor to run.
unsafe impl<'a, T: ScopedWorkItem + Sync> RawWorkItem<0> for &'a ScopedWork<T> {
    type EnqueueOutput = bool;

    unsafe fn __enqueue<F>(self, queue_work_on: F) -> Self::EnqueueOutput
    where
        F: FnOnce(*mut bindings::work_struct) -> bool,
    {
        queue_work_on(self.raw_work())
    }
}

#[pinned_drop]
impl<T: ScopedWorkItem> PinnedDrop for ScopedWork<T> {
    #[inline]
    fn drop(self: Pin<&mut Self>) {
        // SAFETY: `raw_work()` returns a valid `work_struct`. After `cancel_work_sync()` returns,
        // the work function is guaranteed to not be running.
        unsafe { bindings::cancel_work_sync(self.raw_work()) };

        // SAFETY: `inner` is initialized (type invariant), no longer accessed by the work
        // function, and never accessed again after this point. `Opaque` does not drop its
        // contents, so we have to do it here.
        unsafe { core::ptr::drop_in_place(self.inner.get()) };
    }
}

/// Creates a [`ScopedWork`] pin-initializer with a new lock class.
///
/// This macro expands to a call to [`ScopedWork::new`], which is `unsafe`, so the invocation must
/// be wrapped in an `unsafe` block.
///
/// # Safety
///
/// The caller must uphold the safety requirements of [`ScopedWork::new`]: the destructor of the
/// initialized `ScopedWork` must run before the memory it lives in is reused or any lifetime in
/// the inner type expires.
///
/// # Examples
///
/// ```
/// use kernel::workqueue::{
///     new_scoped_work,
///     ScopedWork,
///     ScopedWorkItem,
/// };
///
/// struct MyWork {
///     value: u32,
/// }
///
/// impl ScopedWorkItem for MyWork {
///     fn run(work: &ScopedWork<Self>) {
///         pr_info!("value = {}\n", work.value);
///     }
/// }
///
/// #[pin_data]
/// struct MyData {
///     #[pin]
///     work: ScopedWork<MyWork>,
/// }
///
/// fn init_data() -> impl PinInit<MyData> {
///     pin_init!(MyData {
///         // SAFETY: `MyData` is only ever pin-initialized, so its `work` field is dropped
///         // whenever `MyData` is.
///         work <- unsafe { new_scoped_work!("MyWork", MyWork { value: 7 }) },
///     })
/// }
/// ```
#[macro_export]
macro_rules! new_scoped_work {
    ($name:literal, $init:expr) => {
        $crate::workqueue::ScopedWork::new(
            $crate::c_str!($name),
            $crate::static_lock_class!(),
            $init,
        )
    };
}
pub use new_scoped_work;
