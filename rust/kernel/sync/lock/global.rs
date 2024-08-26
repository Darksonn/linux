// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2024 Google LLC.

//! Support for defining statics containing locks.

/// Defines a global lock.
///
/// Supports the following options:
///
/// * `value` specifies the initial value in the global lock.
/// * `wrapper` specifies the name of the wrapper struct.
/// * `guard` specifies the name of the guard type.
/// * `locked_by` specifies the name of the `LockedBy` type.
///
/// # Examples
///
/// A global counter.
///
/// ```
/// kernel::sync::global_lock! {
///     // SAFETY: Initialized in module initializer before first use.
///     static MY_COUNTER: Mutex<u32> = unsafe { uninit };
///     value: 0;
/// }
///
/// fn increment_counter() -> u32 {
///     let mut guard = MY_COUNTER.lock();
///     *guard += 1;
///     *guard
/// }
///
/// impl kernel::Module for MyModule {
///     fn init(_module: &'static ThisModule) -> Result<Self> {
///         // SAFETY: called exactly once
///         unsafe { MY_COUNTER.init() };
///
///         Ok(MyModule {})
///     }
/// }
/// # struct MyModule {}
/// ```
///
/// A global mutex used to protect all instances of a given struct.
///
/// ```
/// kernel::sync::global_lock! {
///     // SAFETY: Initialized in module initializer before first use.
///     static MY_MUTEX: Mutex<()> = unsafe { uninit };
///     value: ();
///     guard: MyGuard;
///     locked_by: LockedByMyMutex;
/// }
///
/// /// All instances of this struct are protected by `MY_MUTEX`.
/// struct MyStruct {
///     my_counter: LockedByMyMutex<u32>,
/// }
///
/// impl MyStruct {
///     /// Increment the counter in this instance.
///     ///
///     /// The caller must hold the `MY_MUTEX` mutex.
///     fn increment(&self, guard: &mut MyGuard) -> u32 {
///         let my_counter = self.my_counter.as_mut(guard);
///         *my_counter += 1;
///         *my_counter
///     }
/// }
///
/// impl kernel::Module for MyModule {
///     fn init(_module: &'static ThisModule) -> Result<Self> {
///         // SAFETY: called exactly once
///         unsafe { MY_MUTEX.init() };
///
///         Ok(MyModule {})
///     }
/// }
/// # struct MyModule {}
/// ```
#[macro_export]
macro_rules! global_lock {
    {
        $(#[$meta:meta])* $pub:vis static $name:ident: $kind:ident<$valuety:ty> = unsafe { uninit };
        value: $value:expr;
        wrapper: $wrapper:ident;
        $( name: $lname:literal; )?
        $(
            guard: $guard:ident;
            locked_by: $locked_by:ident;
        )?
    } => {
        $crate::macros::paste! {
            type [< __static_lock_ty_ $name >] = $valuety;

            #[allow(unused_pub)]
            mod [< __static_lock_mod_ $name >] {
                use super::[< __static_lock_ty_ $name >] as Val;
                type Backend = $crate::global_lock_inner!(backend $kind);
                type GuardTyp = $crate::global_lock_inner!(guard $kind, Val $(, $guard)?);

                /// # Safety
                ///
                /// Must be used to initialize `super::$name`.
                pub(super) const unsafe fn new() -> $wrapper {
                    let state = $crate::types::Opaque::uninit();
                    $wrapper {
                        // SAFETY: The user of this macro promises to call `init` before calling
                        // `lock`.
                        inner: unsafe {
                            $crate::sync::lock::Lock::global_lock_helper_new(state, $value)
                        }
                    }
                }

                /// Wrapper type for a global lock.
                pub struct $wrapper {
                    inner: $crate::sync::lock::Lock<Val, Backend>,
                }

                impl $wrapper {
                    /// Initialize the global lock.
                    ///
                    /// # Safety
                    ///
                    /// This method must not be called more than once.
                    pub unsafe fn init(&'static self) {
                        // SAFETY:
                        // * This type can only be created by `new`.
                        // * Caller promises to not call this method more than once.
                        unsafe {
                            $crate::sync::lock::Lock::global_lock_helper_init(
                                ::core::pin::Pin::static_ref(&self.inner),
                                $crate::optional_name!($($lname)?),
                                $crate::static_lock_class!(),
                            );
                        }
                    }

                    /// Lock this global lock.
                    pub fn lock(&'static self) -> GuardTyp {
                        $crate::global_lock_inner!(new_guard $($guard)? {
                            self.inner.lock()
                        })
                    }
                }

                $(
                pub struct $guard($crate::sync::lock::Guard<'static, Val, Backend>);

                impl ::core::ops::Deref for $guard {
                    type Target = Val;
                    fn deref(&self) -> &Val {
                        &self.0
                    }
                }

                impl ::core::ops::DerefMut for $guard {
                    fn deref_mut(&mut self) -> &mut Val {
                        &mut self.0
                    }
                }

                pub struct $locked_by<T: ?Sized>(::core::cell::UnsafeCell<T>);

                impl<T> $locked_by<T> {
                    pub fn new(val: T) -> Self {
                        Self(::core::cell::UnsafeCell::new(val))
                    }
                }

                impl<T: ?Sized> $locked_by<T> {
                    pub fn as_ref<'a>(&'a self, _guard: &'a $guard) -> &'a T {
                        // SAFETY: The lock is globally unique, so there can only be one guard.
                        unsafe { &*self.0.get() }
                    }

                    pub fn as_mut<'a>(&'a self, _guard: &'a mut $guard) -> &'a mut T {
                        // SAFETY: The lock is globally unique, so there can only be one guard.
                        unsafe { &mut *self.0.get() }
                    }

                    pub fn get_mut(&mut self) -> &mut T {
                        self.0.get_mut()
                    }
                }
                )?
            }

            use [< __static_lock_mod_ $name >]::$wrapper;
            $( use [< __static_lock_mod_ $name >]::{$guard, $locked_by}; )?

            $(#[$meta])*
            $pub static $name: $wrapper = {
                // SAFETY: We are using this to initialize $name.
                unsafe { [< __static_lock_mod_ $name >]::new() }
            };
        }
    };

    {
        $(#[$meta:meta])* $pub:vis static $name:ident: $kind:ident<$valuety:ty> = unsafe { uninit };
        value: $value:expr;
        $( name: $lname:literal; )?
        $(
            guard: $guard:ident;
            locked_by: $locked_by:ident;
        )?
    } => {
        $crate::macros::paste! {
            $crate::global_lock! {
                $(#[$meta])* $pub static $name: $kind<$valuety> = unsafe { uninit };
                value: $value;
                wrapper: [< __static_lock_wrapper_ $name >];
                $( name: $lname; )?
                $( guard: $guard; locked_by: $locked_by; )?
            }
        }
    }
}
pub use global_lock;

#[doc(hidden)]
#[macro_export]
macro_rules! global_lock_inner {
    (backend Mutex) => { $crate::sync::lock::mutex::MutexBackend };
    (backend SpinLock) => { $crate::sync::lock::spinlock::SpinLockBackend };
    (guard Mutex, $val:ty) => {
        $crate::sync::lock::Guard<'static, $val, $crate::sync::lock::mutex::MutexBackend>
    };
    (guard SpinLock, $val:ty) => {
        $crate::sync::lock::Guard<'static, $val, $crate::sync::lock::spinlock::SpinLockBackend>
    };
    (guard $kind:ident, $val:ty, $name:ident) => { $name };
    (new_guard { $val:expr }) => { $val };
    (new_guard $name:ident { $val:expr }) => { $name($val) };
}
