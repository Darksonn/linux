// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2024 Google LLC.

//! Miscdevice support.
//!
//! C headers: [`include/linux/miscdevice.h`](srctree/include/linux/miscdevice.h).
//!
//! Reference: <https://www.kernel.org/doc/html/latest/driver-api/misc_devices.html>

use crate::{
    bindings,
    error::{to_result, Error, Result, VTABLE_DEFAULT_ERROR},
    fs::{File, LocalFile},
    mm::virt::VmArea,
    prelude::*,
    str::CStr,
    types::{ForeignOwnable, Opaque},
};
use core::{
    ffi::{c_int, c_long, c_uint, c_ulong},
    marker::PhantomData,
    mem::MaybeUninit,
    pin::Pin,
    ptr::NonNull,
};

/// The kernel `loff_t` type.
#[allow(non_camel_case_types)]
pub type loff_t = bindings::loff_t;

/// Options for creating a misc device.
#[derive(Copy, Clone)]
pub struct MiscDeviceOptions {
    /// The name of the miscdevice.
    pub name: &'static CStr,
}

impl MiscDeviceOptions {
    /// Create a raw `struct miscdev` ready for registration.
    pub const fn into_raw<T: MiscDevice>(self) -> bindings::miscdevice {
        // SAFETY: All zeros is valid for this C type.
        let mut result: bindings::miscdevice = unsafe { MaybeUninit::zeroed().assume_init() };
        result.minor = bindings::MISC_DYNAMIC_MINOR as _;
        result.name = self.name.as_char_ptr();
        result.fops = create_vtable::<T>();
        result
    }
}

/// A registration of a miscdevice.
///
/// # Invariants
///
/// `inner` is a registered misc device.
#[repr(transparent)]
#[pin_data(PinnedDrop)]
pub struct MiscDeviceRegistration<T> {
    #[pin]
    inner: Opaque<bindings::miscdevice>,
    _t: PhantomData<T>,
}

unsafe impl<T> Send for MiscDeviceRegistration<T> {}
unsafe impl<T> Sync for MiscDeviceRegistration<T> {}

impl<T: MiscDevice> MiscDeviceRegistration<T> {
    /// Register a misc device.
    pub fn register(opts: MiscDeviceOptions) -> impl PinInit<Self, Error> {
        try_pin_init!(Self {
            inner <- Opaque::try_ffi_init(move |slot: *mut bindings::miscdevice| {
                // SAFETY: The initializer can write to the provided `slot`.
                unsafe { slot.write(opts.into_raw::<T>()) };

                // SAFETY: We just wrote the misc device options to the slot. The miscdevice will
                // get unregistered before `slot` is deallocated because the memory is pinned and
                // the destructor of this type deallocates the memory.
                // INVARIANT: If this returns `Ok(())`, then the `slot` will contain a registered
                // misc device.
                to_result(unsafe { bindings::misc_register(slot) })
            }),
            _t: PhantomData,
        })
    }

    /// Returns the private data associated with the provided file.
    ///
    /// Returns `None` if the file is not associated with this misc device.
    pub fn try_get_private_data<'a>(
        &self,
        file: &'a LocalFile,
    ) -> Option<<T::Ptr as ForeignOwnable>::Borrowed<'a>> {
        // SAFETY: `fops` of a miscdevice is immutable after initialization.
        let fops_this = unsafe { (*self.as_raw()).fops };
        // SAFETY: `f_op` of a file is immutable after initialization.
        let fops_file = unsafe { (*file.as_ptr()).f_op };

        if core::ptr::eq(fops_this, fops_file) {
            // SAFETY: We know that `file` is associated with a `MiscDeviceRegistration<T>`, so
            // `private_data` is immutable.
            let private_data = unsafe { (*file.as_ptr()).private_data };
            // SAFETY:
            // * The fops match, so the file's private date has the right type.
            // * The returned borrow cannot outlive the file.
            Some(unsafe { <T::Ptr as ForeignOwnable>::borrow(private_data) })
        } else {
            None
        }
    }

    /// Returns a raw pointer to the misc device.
    pub fn as_raw(&self) -> *mut bindings::miscdevice {
        self.inner.get()
    }
}

#[pinned_drop]
impl<T> PinnedDrop for MiscDeviceRegistration<T> {
    fn drop(self: Pin<&mut Self>) {
        // SAFETY: We know that the device is registered by the type invariants.
        unsafe { bindings::misc_deregister(self.inner.get()) };
    }
}

/// Trait implemented by the private data of an open misc device.
#[vtable]
pub trait MiscDevice {
    /// What kind of pointer should `Self` be wrapped in.
    type Ptr: ForeignOwnable + Send + Sync;

    /// Called when the misc device is opened.
    ///
    /// The returned pointer will be stored as the private data for the file.
    fn open(_file: &File) -> Result<Self::Ptr>;

    /// Called when the misc device is released.
    fn release(device: Self::Ptr, _file: &File) {
        drop(device);
    }

    /// Handle for mmap.
    fn mmap(
        _device: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        _file: &File,
        _vma: Pin<&mut VmArea>,
    ) -> Result<()> {
        kernel::build_error(VTABLE_DEFAULT_ERROR)
    }

    /// Seeks this miscdevice.
    fn llseek(
        _device: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        _file: &LocalFile,
        _offset: loff_t,
        _whence: c_int,
    ) -> Result<loff_t> {
        kernel::build_error(VTABLE_DEFAULT_ERROR)
    }

    /// Read from this miscdevice.
    fn read_iter(_kiocb: Kiocb<'_, Self::Ptr>, _iov: &mut IovIter) -> Result<usize> {
        kernel::build_error(VTABLE_DEFAULT_ERROR)
    }

    /// Handler for ioctls
    ///
    /// The `cmd` argument is usually manipulated using the utilties in [`kernel::ioctl`].
    ///
    /// [`kernel::ioctl`]: mod@crate::ioctl
    fn ioctl(
        _device: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        _file: &File,
        _cmd: u32,
        _arg: usize,
    ) -> Result<c_long> {
        kernel::build_error(VTABLE_DEFAULT_ERROR)
    }

    /// Handler for ioctls
    ///
    /// Used for 32-bit userspace on 64-bit platforms.
    #[cfg(CONFIG_COMPAT)]
    fn compat_ioctl(
        device: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        file: &File,
        cmd: u32,
        arg: usize,
    ) -> Result<c_long> {
        Self::ioctl(device, file, cmd, arg)
    }
}

/// Wrapper for the kernel's `struct kiocb`.
///
/// The type `T` represents the private data of the file.
pub struct Kiocb<'a, T> {
    inner: NonNull<bindings::kiocb>,
    _phantom: PhantomData<&'a T>,
}

impl<'a, T: ForeignOwnable> Kiocb<'a, T> {
    /// Get the private data in this kiocb.
    pub fn private_data(&self) -> <T as ForeignOwnable>::Borrowed<'a> {
        // SAFETY: The `kiocb` lets us access the private data.
        let private = unsafe { (*(*self.inner.as_ptr()).ki_filp).private_data };
        // SAFETY: The kiocb has shared access to the private data.
        unsafe { <T as ForeignOwnable>::borrow(private) }
    }

    /// Gets the current value of `ki_pos`.
    pub fn ki_pos(&self) -> loff_t {
        // SAFETY: The `kiocb` can access `ki_pos`.
        unsafe { (*self.inner.as_ptr()).ki_pos }
    }

    /// Gets a mutable reference to the `ki_pos` field.
    pub fn ki_pos_mut(&mut self) -> &mut loff_t {
        // SAFETY: The `kiocb` can access `ki_pos`.
        unsafe { &mut (*self.inner.as_ptr()).ki_pos }
    }
}

/// Wrapper for the kernel's `struct iov_iter`.
pub struct IovIter {
    inner: Opaque<bindings::iov_iter>,
}

impl IovIter {
    /// Gets a raw pointer to the contents.
    pub fn as_raw(&self) -> *mut bindings::iov_iter {
        self.inner.get()
    }
}

const fn create_vtable<T: MiscDevice>() -> &'static bindings::file_operations {
    const fn maybe_fn<T: Copy>(check: bool, func: T) -> Option<T> {
        if check {
            Some(func)
        } else {
            None
        }
    }

    struct VtableHelper<T: MiscDevice> {
        _t: PhantomData<T>,
    }
    impl<T: MiscDevice> VtableHelper<T> {
        const VTABLE: bindings::file_operations = bindings::file_operations {
            open: Some(fops_open::<T>),
            release: Some(fops_release::<T>),
            mmap: maybe_fn(T::HAS_MMAP, fops_mmap::<T>),
            llseek: maybe_fn(T::HAS_LLSEEK, fops_llseek::<T>),
            read_iter: maybe_fn(T::HAS_READ_ITER, fops_read_iter::<T>),
            unlocked_ioctl: maybe_fn(T::HAS_IOCTL, fops_ioctl::<T>),
            #[cfg(CONFIG_COMPAT)]
            compat_ioctl: maybe_fn(T::HAS_IOCTL || T::HAS_COMPAT_IOCTL, fops_compat_ioctl::<T>),
            ..unsafe { MaybeUninit::zeroed().assume_init() }
        };
    }

    &VtableHelper::<T>::VTABLE
}

unsafe extern "C" fn fops_open<T: MiscDevice>(
    inode: *mut bindings::inode,
    file: *mut bindings::file,
) -> c_int {
    // SAFETY: The pointers are valid and for a file being opened.
    let ret = unsafe { bindings::generic_file_open(inode, file) };
    if ret != 0 {
        return ret;
    }

    // SAFETY:
    // * The file is valid for the duration of this call.
    // * There is no active fdget_pos region on the file on this thread.
    let ptr = match T::open(unsafe { File::from_raw_file(file) }) {
        Ok(ptr) => ptr,
        Err(err) => return err.to_errno(),
    };

    // SAFETY: The open call of a file owns the private data.
    unsafe { (*file).private_data = ptr.into_foreign().cast_mut() };

    0
}

unsafe extern "C" fn fops_release<T: MiscDevice>(
    _inode: *mut bindings::inode,
    file: *mut bindings::file,
) -> c_int {
    // SAFETY: The release call of a file owns the private data.
    let private = unsafe { (*file).private_data };
    // SAFETY: We are taking ownership of the private data, so we can drop it.
    let ptr = unsafe { <T::Ptr as ForeignOwnable>::from_foreign(private) };
    // SAFETY:
    // * The file is valid for the duration of this call.
    // * There is no active fdget_pos region on the file on this thread.
    T::release(ptr, unsafe { File::from_raw_file(file) });

    0
}

unsafe extern "C" fn fops_mmap<T: MiscDevice>(
    file: *mut bindings::file,
    vma: *mut bindings::vm_area_struct,
) -> c_int {
    // SAFETY: The release call of a file owns the private data.
    let private = unsafe { (*file).private_data };
    // SAFETY: Ioctl calls can borrow the private data of the file.
    let device = unsafe { <T::Ptr as ForeignOwnable>::borrow(private) };
    // SAFETY:
    // * The file is valid for the duration of this call.
    // * There is no active fdget_pos region on the file on this thread.
    let file = unsafe { File::from_raw_file(file) };
    // SAFETY: The caller ensures that the vma is valid.
    let area = unsafe { kernel::mm::virt::VmArea::from_raw_mut(vma) };

    match T::mmap(device, file, area) {
        Ok(()) => 0,
        Err(err) => err.to_errno() as c_int,
    }
}

unsafe extern "C" fn fops_llseek<T: MiscDevice>(
    file: *mut bindings::file,
    offset: loff_t,
    whence: c_int,
) -> loff_t {
    // SAFETY: The release call of a file owns the private data.
    let private = unsafe { (*file).private_data };
    // SAFETY: Ioctl calls can borrow the private data of the file.
    let device = unsafe { <T::Ptr as ForeignOwnable>::borrow(private) };
    // SAFETY:
    // * The file is valid for the duration of this call.
    // * We are inside an fdget_pos region, so there cannot be any active fdget_pos regions on
    //   other threads.
    let file = unsafe { LocalFile::from_raw_file(file) };

    match T::llseek(device, file, offset, whence) {
        Ok(res) => res as loff_t,
        Err(err) => err.to_errno() as loff_t,
    }
}

unsafe extern "C" fn fops_read_iter<T: MiscDevice>(
    kiocb: *mut bindings::kiocb,
    iter: *mut bindings::iov_iter,
) -> isize {
    let kiocb = Kiocb {
        inner: unsafe { NonNull::new_unchecked(kiocb) },
        _phantom: PhantomData,
    };
    let iov = unsafe { &mut *iter.cast::<IovIter>() };

    match T::read_iter(kiocb, iov) {
        Ok(res) => res as isize,
        Err(err) => err.to_errno() as isize,
    }
}

unsafe extern "C" fn fops_ioctl<T: MiscDevice>(
    file: *mut bindings::file,
    cmd: c_uint,
    arg: c_ulong,
) -> c_long {
    // SAFETY: The release call of a file owns the private data.
    let private = unsafe { (*file).private_data };
    // SAFETY: Ioctl calls can borrow the private data of the file.
    let device = unsafe { <T::Ptr as ForeignOwnable>::borrow(private) };
    // SAFETY:
    // * The file is valid for the duration of this call.
    // * There is no active fdget_pos region on the file on this thread.
    let file = unsafe { File::from_raw_file(file) };

    match T::ioctl(device, file, cmd as u32, arg as usize) {
        Ok(ret) => ret,
        Err(err) => err.to_errno() as c_long,
    }
}

#[cfg(CONFIG_COMPAT)]
unsafe extern "C" fn fops_compat_ioctl<T: MiscDevice>(
    file: *mut bindings::file,
    cmd: c_uint,
    arg: c_ulong,
) -> c_long {
    // SAFETY: The release call of a file owns the private data.
    let private = unsafe { (*file).private_data };
    // SAFETY: Ioctl calls can borrow the private data of the file.
    let device = unsafe { <T::Ptr as ForeignOwnable>::borrow(private) };
    // SAFETY:
    // * The file is valid for the duration of this call.
    // * There is no active fdget_pos region on the file on this thread.
    let file = unsafe { File::from_raw_file(file) };

    match T::compat_ioctl(device, file, cmd as u32, arg as usize) {
        Ok(ret) => ret,
        Err(err) => err.to_errno() as c_long,
    }
}
