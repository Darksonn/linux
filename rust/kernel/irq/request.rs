// SPDX-License-Identifier: GPL-2.0
// SPDX-FileCopyrightText: Copyright 2025 Collabora ltd.

//! This module provides types like [`Registration`] and
//! [`ThreadedRegistration`], which allow users to register handlers for a given
//! IRQ line.

use core::marker::PhantomPinned;

use crate::alloc::Allocator;
use crate::device::Bound;
use crate::device::Device;
use crate::devres::Devres;
use crate::error::to_result;
use crate::irq::flags::Flags;
use crate::prelude::*;
use crate::str::CStr;
use crate::sync::Arc;

/// The value that can be returned from an IrqHandler or a ThreadedIrqHandler.
#[repr(u32)]
pub enum IrqReturn {
    /// The interrupt was not from this device or was not handled.
    None = bindings::irqreturn_IRQ_NONE,

    /// The interrupt was handled by this device.
    Handled = bindings::irqreturn_IRQ_HANDLED,
}

/// Callbacks for an IRQ handler.
pub trait Handler: Sync {
    /// The hard IRQ handler.
    ///
    /// This is executed in interrupt context, hence all corresponding
    /// limitations do apply.
    ///
    /// All work that does not necessarily need to be executed from
    /// interrupt context, should be deferred to a threaded handler.
    /// See also [`ThreadedRegistration`].
    fn handle(&self) -> IrqReturn;
}

impl<T: ?Sized + Handler + Send> Handler for Arc<T> {
    fn handle(&self) -> IrqReturn {
        T::handle(self)
    }
}

impl<T: ?Sized + Handler, A: Allocator> Handler for Box<T, A> {
    fn handle(&self) -> IrqReturn {
        T::handle(self)
    }
}

/// # Invariants
///
/// - `self.irq` is the same as the one passed to `request_{threaded}_irq`.
/// - `cookie` was passed to `request_{threaded}_irq` as the cookie. It
///    is guaranteed to be unique by the type system, since each call to
///    `new` will return a different instance of `Registration`.
#[pin_data(PinnedDrop)]
struct RegistrationInner {
    irq: u32,
    cookie: *mut c_void,
}

impl RegistrationInner {
    fn synchronize(&self) {
        // SAFETY: safe as per the invariants of `RegistrationInner`
        unsafe { bindings::synchronize_irq(self.irq) };
    }
}

#[pinned_drop]
impl PinnedDrop for RegistrationInner {
    fn drop(self: Pin<&mut Self>) {
        // SAFETY:
        //
        // Safe as per the invariants of `RegistrationInner` and:
        //
        // - The containing struct is `!Unpin` and was initialized using
        // pin-init, so it occupied the same memory location for the entirety of
        // its lifetime.
        //
        // Notice that this will block until all handlers finish executing,
        // i.e.: at no point will &self be invalid while the handler is running.
        unsafe { bindings::free_irq(self.irq, self.cookie) };
    }
}

// SAFETY: We only use `inner` on drop, which called at most once with no
// concurrent access.
unsafe impl Sync for RegistrationInner {}

// SAFETY: It is safe to send `RegistrationInner` across threads.
unsafe impl Send for RegistrationInner {}

/// A request for an IRQ line for a given device.
///
/// # Invariants
///
/// - `ìrq` is the number of an interrupt source of `dev`.
/// - `irq` has not been registered yet.
pub struct IrqRequest<'a> {
    dev: &'a Device<Bound>,
    irq: u32,
}

impl<'a> IrqRequest<'a> {
    /// Creates a new IRQ request for the given device and IRQ number.
    ///
    /// # Safety
    ///
    /// - `irq` should be a valid IRQ number for `dev`.
    pub(crate) unsafe fn new(dev: &'a Device<Bound>, irq: u32) -> Self {
        IrqRequest { dev, irq }
    }

    /// Returns the IRQ number of an [`IrqRequest`].
    pub fn irq(&self) -> u32 {
        self.irq
    }
}

/// A registration of an IRQ handler for a given IRQ line.
///
/// # Examples
///
/// The following is an example of using `Registration`. It uses a
/// [`AtomicU32`](core::sync::AtomicU32) to provide the interior mutability.
///
/// ```
/// use core::sync::atomic::AtomicU32;
/// use core::sync::atomic::Ordering;
///
/// use kernel::prelude::*;
/// use kernel::device::Bound;
/// use kernel::irq::flags::Flags;
/// use kernel::irq::Registration;
/// use kernel::irq::IrqRequest;
/// use kernel::irq::IrqReturn;
/// use kernel::sync::Arc;
/// use kernel::c_str;
/// use kernel::alloc::flags::GFP_KERNEL;
///
/// // Declare a struct that will be passed in when the interrupt fires. The u32
/// // merely serves as an example of some internal data.
/// struct Data(AtomicU32);
///
/// // [`kernel::irq::request::Handler::handle`] takes `&self`. This example
/// // illustrates how interior mutability can be used when sharing the data
/// // between process context and IRQ context.
///
/// type Handler = Data;
///
/// impl kernel::irq::request::Handler for Handler {
///     // This is executing in IRQ context in some CPU. Other CPUs can still
///     // try to access to data.
///     fn handle(&self) -> IrqReturn {
///         self.0.fetch_add(1, Ordering::Relaxed);
///
///         IrqReturn::Handled
///     }
/// }
///
/// // Registers an IRQ handler for the given IrqRequest.
/// //
/// // This is executing in process context and assumes that `request` was
/// // previously acquired from a device.
/// fn register_irq(handler: Handler, request: IrqRequest<'_>) -> Result<Arc<Registration<Handler>>> {
///     let registration = Registration::new(request, Flags::SHARED, c_str!("my_device"), handler);
///
///     let registration = Arc::pin_init(registration, GFP_KERNEL)?;
///
///     // The data can be accessed from process context too.
///     registration.handler().0.fetch_add(1, Ordering::Relaxed);
///
///     Ok(registration)
/// }
/// # Ok::<(), Error>(())
/// ```
///
/// # Invariants
///
/// * We own an irq handler using `&self.handler` as its private data.
///
#[pin_data]
pub struct Registration<T: Handler + 'static> {
    #[pin]
    inner: Devres<RegistrationInner>,

    #[pin]
    handler: T,

    /// Pinned because we need address stability so that we can pass a pointer
    /// to the callback.
    #[pin]
    _pin: PhantomPinned,
}

impl<T: Handler + 'static> Registration<T> {
    /// Registers the IRQ handler with the system for the given IRQ number.
    pub fn new<'a>(
        request: IrqRequest<'a>,
        flags: Flags,
        name: &'static CStr,
        handler: T,
    ) -> impl PinInit<Self, Error> + 'a {
        try_pin_init!(&this in Self {
            handler,
            inner <- Devres::new(
                request.dev,
                try_pin_init!(RegistrationInner {
                    // SAFETY: `this` is a valid pointer to the `Registration` instance
                    cookie: unsafe { &raw mut (*this.as_ptr()).handler }.cast(),
                    irq: {
                        // SAFETY:
                        // - The callbacks are valid for use with request_irq.
                        // - If this succeeds, the slot is guaranteed to be valid until the
                        //   destructor of Self runs, which will deregister the callbacks
                        //   before the memory location becomes invalid.
                        to_result(unsafe {
                            bindings::request_irq(
                                request.irq,
                                Some(handle_irq_callback::<T>),
                                flags.into_inner(),
                                name.as_char_ptr(),
                                (&raw mut (*this.as_ptr()).handler).cast(),
                            )
                        })?;
                        request.irq
                    }
                })
            ),
            _pin: PhantomPinned,
        })
    }

    /// Returns a reference to the handler that was registered with the system.
    pub fn handler(&self) -> &T {
        &self.handler
    }

    /// Wait for pending IRQ handlers on other CPUs.
    ///
    /// This will attempt to access the inner [`Devres`] container.
    pub fn try_synchronize(&self) -> Result {
        let inner = self.inner.try_access().ok_or(ENODEV)?;
        inner.synchronize();
        Ok(())
    }

    /// Wait for pending IRQ handlers on other CPUs.
    pub fn synchronize(&self, dev: &Device<Bound>) -> Result {
        let inner = self.inner.access(dev)?;
        inner.synchronize();
        Ok(())
    }
}

/// # Safety
///
/// This function should be only used as the callback in `request_irq`.
unsafe extern "C" fn handle_irq_callback<T: Handler>(_irq: i32, ptr: *mut c_void) -> c_uint {
    // SAFETY: `ptr` is a pointer to T set in `Registration::new`
    let handler = unsafe { &*(ptr as *const T) };
    T::handle(handler) as c_uint
}

/// The value that can be returned from `ThreadedHandler::handle_irq`.
#[repr(u32)]
pub enum ThreadedIrqReturn {
    /// The interrupt was not from this device or was not handled.
    None = bindings::irqreturn_IRQ_NONE,

    /// The interrupt was handled by this device.
    Handled = bindings::irqreturn_IRQ_HANDLED,

    /// The handler wants the handler thread to wake up.
    WakeThread = bindings::irqreturn_IRQ_WAKE_THREAD,
}

/// Callbacks for a threaded IRQ handler.
pub trait ThreadedHandler: Sync {
    /// The hard IRQ handler.
    ///
    /// This is executed in interrupt context, hence all corresponding
    /// limitations do apply. All work that does not necessarily need to be
    /// executed from interrupt context, should be deferred to the threaded
    /// handler, i.e. [`ThreadedHandler::handle_threaded`].
    fn handle(&self) -> ThreadedIrqReturn;

    /// The threaded IRQ handler.
    ///
    /// This is executed in process context. The kernel creates a dedicated
    /// kthread for this purpose.
    fn handle_threaded(&self) -> IrqReturn;
}

impl<T: ?Sized + ThreadedHandler + Send> ThreadedHandler for Arc<T> {
    fn handle(&self) -> ThreadedIrqReturn {
        T::handle(self)
    }

    fn handle_threaded(&self) -> IrqReturn {
        T::handle_threaded(self)
    }
}

impl<T: ?Sized + ThreadedHandler, A: Allocator> ThreadedHandler for Box<T, A> {
    fn handle(&self) -> ThreadedIrqReturn {
        T::handle(self)
    }

    fn handle_threaded(&self) -> IrqReturn {
        T::handle_threaded(self)
    }
}

/// A registration of a threaded IRQ handler for a given IRQ line.
///
/// Two callbacks are required: one to handle the IRQ, and one to handle any
/// other work in a separate thread.
///
/// The thread handler is only called if the IRQ handler returns `WakeThread`.
///
/// # Examples
///
/// The following is an example of using `ThreadedRegistration`. It uses a
/// [`AtomicU32`](core::sync::AtomicU32) to provide the interior mutability.
///
/// ```
/// use core::sync::atomic::AtomicU32;
/// use core::sync::atomic::Ordering;
///
/// use kernel::prelude::*;
/// use kernel::device::Bound;
/// use kernel::irq::flags::Flags;
/// use kernel::irq::ThreadedIrqReturn;
/// use kernel::irq::ThreadedRegistration;
/// use kernel::irq::IrqRequest;
/// use kernel::irq::IrqReturn;
/// use kernel::sync::Arc;
/// use kernel::c_str;
/// use kernel::alloc::flags::GFP_KERNEL;
///
/// // Declare a struct that will be passed in when the interrupt fires. The u32
/// // merely serves as an example of some internal data.
/// struct Data(AtomicU32);
///
/// // [`kernel::irq::request::ThreadedHandler::handle`] takes `&self`. This example
/// // illustrates how interior mutability can be used when sharing the data
/// // between process context and IRQ context.
/// type Handler = Data;
///
/// impl kernel::irq::request::ThreadedHandler for Handler {
///     // This is executing in IRQ context in some CPU. Other CPUs can still
///     // try to access the data.
///     fn handle(&self) -> ThreadedIrqReturn {
///         self.0.fetch_add(1, Ordering::Relaxed);
///         // By returning `WakeThread`, we indicate to the system that the
///         // thread function should be called. Otherwise, return
///         // ThreadedIrqReturn::Handled.
///         ThreadedIrqReturn::WakeThread
///     }
///
///     // This will run (in a separate kthread) if and only if `handle`
///     // returns `WakeThread`.
///     fn handle_threaded(&self) -> IrqReturn {
///         self.0.fetch_add(1, Ordering::Relaxed);
///         IrqReturn::Handled
///     }
/// }
///
/// // Registers a threaded IRQ handler for the given IrqRequest.
/// //
/// // This is executing in process context and assumes that `request` was
/// // previously acquired from a device.
/// fn register_threaded_irq(handler: Handler, request: IrqRequest<'_>) -> Result<Arc<ThreadedRegistration<Handler>>> {
///     let registration = ThreadedRegistration::new(request, Flags::SHARED, c_str!("my_device"), handler);
///
///     let registration = Arc::pin_init(registration, GFP_KERNEL)?;
///
///     // The data can be accessed from process context too.
///     registration.handler().0.fetch_add(1, Ordering::Relaxed);
///
///     Ok(registration)
/// }
/// # Ok::<(), Error>(())
/// ```
///
/// # Invariants
///
/// * We own an irq handler using `&T` as its private data.
///
#[pin_data]
pub struct ThreadedRegistration<T: ThreadedHandler + 'static> {
    #[pin]
    inner: Devres<RegistrationInner>,

    #[pin]
    handler: T,

    /// Pinned because we need address stability so that we can pass a pointer
    /// to the callback.
    #[pin]
    _pin: PhantomPinned,
}

impl<T: ThreadedHandler + 'static> ThreadedRegistration<T> {
    /// Registers the IRQ handler with the system for the given IRQ number.
    pub fn new<'a>(
        request: IrqRequest<'a>,
        flags: Flags,
        name: &'static CStr,
        handler: T,
    ) -> impl PinInit<Self, Error> + 'a {
        try_pin_init!(&this in Self {
            handler,
            inner <- Devres::new(
                request.dev,
                try_pin_init!(RegistrationInner {
                    // SAFETY: `this` is a valid pointer to the `ThreadedRegistration` instance.
                    cookie: unsafe { &raw mut (*this.as_ptr()).handler }.cast(),
                    irq: {
                        // SAFETY:
                        // - The callbacks are valid for use with request_threaded_irq.
                        // - If this succeeds, the slot is guaranteed to be valid until the
                        // destructor of Self runs, which will deregister the callbacks
                        // before the memory location becomes invalid.
                        to_result(unsafe {
                            bindings::request_threaded_irq(
                                request.irq,
                                Some(handle_threaded_irq_callback::<T>),
                                Some(thread_fn_callback::<T>),
                                flags.into_inner() as usize,
                                name.as_char_ptr(),
                                (&raw mut (*this.as_ptr()).handler).cast(),
                            )
                        })?;
                        request.irq
                    }
                })
            ),
            _pin: PhantomPinned,
        })
    }

    /// Returns a reference to the handler that was registered with the system.
    pub fn handler(&self) -> &T {
        &self.handler
    }

    /// Wait for pending IRQ handlers on other CPUs.
    ///
    /// This will attempt to access the inner [`Devres`] container.
    pub fn try_synchronize(&self) -> Result {
        let inner = self.inner.try_access().ok_or(ENODEV)?;
        inner.synchronize();
        Ok(())
    }

    /// Wait for pending IRQ handlers on other CPUs.
    pub fn synchronize(&self, dev: &Device<Bound>) -> Result {
        let inner = self.inner.access(dev)?;
        inner.synchronize();
        Ok(())
    }
}

/// # Safety
///
/// This function should be only used as the callback in `request_threaded_irq`.
unsafe extern "C" fn handle_threaded_irq_callback<T: ThreadedHandler>(
    _irq: i32,
    ptr: *mut c_void,
) -> c_uint {
    // SAFETY: `ptr` is a pointer to T set in `ThreadedRegistration::new`
    let handler = unsafe { &*(ptr as *const T) };
    T::handle(handler) as c_uint
}

/// # Safety
///
/// This function should be only used as the callback in `request_threaded_irq`.
unsafe extern "C" fn thread_fn_callback<T: ThreadedHandler>(_irq: i32, ptr: *mut c_void) -> c_uint {
    // SAFETY: `ptr` is a pointer to T set in `ThreadedRegistration::new`
    let handler = unsafe { &*(ptr as *const T) };
    T::handle_threaded(handler) as c_uint
}
