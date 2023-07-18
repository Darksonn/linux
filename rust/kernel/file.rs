// SPDX-License-Identifier: GPL-2.0

//! Files and file descriptors.
//!
//! C headers: [`include/linux/fs.h`](../../../../include/linux/fs.h) and
//! [`include/linux/file.h`](../../../../include/linux/file.h)

use crate::{
    bindings,
    cred::Credential,
    error::{code::*, Error, Result},
    types::{ARef, AlwaysRefCounted, Opaque},
};
use alloc::boxed::Box;
use core::{alloc::AllocError, marker::PhantomData, mem, ptr};

mod poll_table;
pub use self::poll_table::{PollCondVar, PollTable};

/// Flags associated with a [`File`].
pub mod flags {
    /// File is opened in append mode.
    pub const O_APPEND: u32 = bindings::O_APPEND;

    /// Signal-driven I/O is enabled.
    pub const O_ASYNC: u32 = bindings::FASYNC;

    /// Close-on-exec flag is set.
    pub const O_CLOEXEC: u32 = bindings::O_CLOEXEC;

    /// File was created if it didn't already exist.
    pub const O_CREAT: u32 = bindings::O_CREAT;

    /// Direct I/O is enabled for this file.
    pub const O_DIRECT: u32 = bindings::O_DIRECT;

    /// File must be a directory.
    pub const O_DIRECTORY: u32 = bindings::O_DIRECTORY;

    /// Like [`O_SYNC`] except metadata is not synced.
    pub const O_DSYNC: u32 = bindings::O_DSYNC;

    /// Ensure that this file is created with the `open(2)` call.
    pub const O_EXCL: u32 = bindings::O_EXCL;

    /// Large file size enabled (`off64_t` over `off_t`).
    pub const O_LARGEFILE: u32 = bindings::O_LARGEFILE;

    /// Do not update the file last access time.
    pub const O_NOATIME: u32 = bindings::O_NOATIME;

    /// File should not be used as process's controlling terminal.
    pub const O_NOCTTY: u32 = bindings::O_NOCTTY;

    /// If basename of path is a symbolic link, fail open.
    pub const O_NOFOLLOW: u32 = bindings::O_NOFOLLOW;

    /// File is using nonblocking I/O.
    pub const O_NONBLOCK: u32 = bindings::O_NONBLOCK;

    /// Also known as `O_NDELAY`.
    ///
    /// This is effectively the same flag as [`O_NONBLOCK`] on all architectures
    /// except SPARC64.
    pub const O_NDELAY: u32 = bindings::O_NDELAY;

    /// Used to obtain a path file descriptor.
    pub const O_PATH: u32 = bindings::O_PATH;

    /// Write operations on this file will flush data and metadata.
    pub const O_SYNC: u32 = bindings::O_SYNC;

    /// This file is an unnamed temporary regular file.
    pub const O_TMPFILE: u32 = bindings::O_TMPFILE;

    /// File should be truncated to length 0.
    pub const O_TRUNC: u32 = bindings::O_TRUNC;

    /// Bitmask for access mode flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use kernel::file;
    /// # fn do_something() {}
    /// # let flags = 0;
    /// if (flags & file::flags::O_ACCMODE) == file::flags::O_RDONLY {
    ///     do_something();
    /// }
    /// ```
    pub const O_ACCMODE: u32 = bindings::O_ACCMODE;

    /// File is read only.
    pub const O_RDONLY: u32 = bindings::O_RDONLY;

    /// File is write only.
    pub const O_WRONLY: u32 = bindings::O_WRONLY;

    /// File can be both read and written.
    pub const O_RDWR: u32 = bindings::O_RDWR;
}

/// Wraps the kernel's `struct file`.
///
/// # Invariants
///
/// Instances of this type are always ref-counted, that is, a call to `get_file` ensures that the
/// allocation remains valid at least until the matching call to `fput`.
#[repr(transparent)]
pub struct File(pub(crate) Opaque<bindings::file>);

// SAFETY: By design, the only way to access a `File` is via an immutable reference or an `ARef`.
// This means that the only situation in which a `File` can be accessed mutably is when the
// refcount drops to zero and the destructor runs. It is safe for that to happen on any thread, so
// it is ok for this type to be `Send`.
unsafe impl Send for File {}

// SAFETY: It's OK to access `File` through shared references from other threads because we're
// either accessing properties that don't change or that are properly synchronised by C code.
unsafe impl Sync for File {}

impl File {
    /// Constructs a new `struct file` wrapper from a file descriptor.
    ///
    /// The file descriptor belongs to the current process.
    pub fn from_fd(fd: u32) -> Result<ARef<Self>, BadFdError> {
        // SAFETY: FFI call, there are no requirements on `fd`.
        let ptr = ptr::NonNull::new(unsafe { bindings::fget(fd) }).ok_or(BadFdError)?;

        // SAFETY: `fget` increments the refcount before returning.
        Ok(unsafe { ARef::from_raw(ptr.cast()) })
    }

    /// Creates a reference to a [`File`] from a valid pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` points at a valid file and that its refcount does not
    /// reach zero until after the end of the lifetime 'a.
    pub unsafe fn from_ptr<'a>(ptr: *const bindings::file) -> &'a File {
        // SAFETY: The safety requirements guarantee the validity of the dereference, while the
        // `File` type being transparent makes the cast ok.
        unsafe { &*ptr.cast() }
    }

    /// Returns the credentials of the task that originally opened the file.
    pub fn cred(&self) -> &Credential {
        // SAFETY: The file is valid because the shared reference guarantees a nonzero refcount.
        //
        // This uses a volatile read because C code may be modifying this field in parallel using
        // non-atomic unsynchronized writes. This corresponds to how the C macro READ_ONCE is
        // implemented.
        let ptr = unsafe { core::ptr::addr_of!((*self.0.get()).f_cred).read_volatile() };
        // SAFETY: The lifetimes of `self` and `Credential` are tied, so it is guaranteed that
        // the credential pointer remains valid (because the file is still alive, and it doesn't
        // change over the lifetime of a file).
        unsafe { Credential::from_ptr(ptr) }
    }

    /// Returns the flags associated with the file.
    ///
    /// The flags are a combination of the constants in [`flags`].
    pub fn flags(&self) -> u32 {
        // SAFETY: The file is valid because the shared reference guarantees a nonzero refcount.
        //
        // This uses a volatile read because C code may be modifying this field in parallel using
        // non-atomic unsynchronized writes. This corresponds to how the C macro READ_ONCE is
        // implemented.
        unsafe { core::ptr::addr_of!((*self.0.get()).f_flags).read_volatile() }
    }
}

// SAFETY: The type invariants guarantee that `File` is always ref-counted.
unsafe impl AlwaysRefCounted for File {
    fn inc_ref(&self) {
        // SAFETY: The existence of a shared reference means that the refcount is nonzero.
        unsafe { bindings::get_file(self.0.get()) };
    }

    unsafe fn dec_ref(obj: ptr::NonNull<Self>) {
        // SAFETY: The safety requirements guarantee that the refcount is nonzero.
        unsafe { bindings::fput(obj.cast().as_ptr()) }
    }
}

/// A file descriptor reservation.
///
/// This allows the creation of a file descriptor in two steps: first, we reserve a slot for it,
/// then we commit or drop the reservation. The first step may fail (e.g., the current process ran
/// out of available slots), but commit and drop never fail (and are mutually exclusive).
///
/// # Invariants
///
/// The fd stored in this struct must correspond to a reserved file descriptor of the current task.
pub struct FileDescriptorReservation {
    fd: u32,
    /// Prevent values of this type from being moved to a different task.
    ///
    /// This is necessary because the C FFI calls assume that `current` is set to the task that
    /// owns the fd in question.
    _not_send_sync: PhantomData<*mut ()>,
}

impl FileDescriptorReservation {
    /// Creates a new file descriptor reservation.
    pub fn new(flags: u32) -> Result<Self> {
        // SAFETY: FFI call, there are no safety requirements on `flags`.
        let fd: i32 = unsafe { bindings::get_unused_fd_flags(flags) };
        if fd < 0 {
            return Err(Error::from_errno(fd));
        }
        Ok(Self {
            fd: fd as _,
            _not_send_sync: PhantomData,
        })
    }

    /// Returns the file descriptor number that was reserved.
    pub fn reserved_fd(&self) -> u32 {
        self.fd
    }

    /// Commits the reservation.
    ///
    /// The previously reserved file descriptor is bound to `file`.
    pub fn commit(self, file: ARef<File>) {
        // SAFETY: `self.fd` was previously returned by `get_unused_fd_flags`, and `file.ptr` is
        // guaranteed to have an owned ref count by its type invariants.
        unsafe { bindings::fd_install(self.fd, file.0.get()) };

        // `fd_install` consumes both the file descriptor and the file reference, so we cannot run
        // the destructors.
        core::mem::forget(self);
        core::mem::forget(file);
    }
}

impl Drop for FileDescriptorReservation {
    fn drop(&mut self) {
        // SAFETY: `self.fd` was returned by a previous call to `get_unused_fd_flags`.
        unsafe { bindings::put_unused_fd(self.fd) };
    }
}

/// Helper used for closing file descriptors in a way that is safe even if the file is currently
/// held using `fdget`.
///
/// See comments on `binder_do_fd_close` and commit `80cd795630d65`.
pub struct DeferredFdCloser {
    inner: Box<DeferredFdCloserInner>,
}

/// SAFETY: This just holds an allocation with no real content, so there's no safety issue with
/// moving it across threads.
unsafe impl Send for DeferredFdCloser {}
unsafe impl Sync for DeferredFdCloser {}

#[repr(C)]
struct DeferredFdCloserInner {
    twork: mem::MaybeUninit<bindings::callback_head>,
    file: *mut bindings::file,
}

impl DeferredFdCloser {
    /// Create a new `DeferredFdCloser`.
    pub fn new() -> Result<Self, AllocError> {
        Ok(Self {
            inner: Box::try_new(DeferredFdCloserInner {
                twork: mem::MaybeUninit::uninit(),
                file: core::ptr::null_mut(),
            })?,
        })
    }

    /// Schedule a task work that closes the file descriptor when this task returns to userspace.
    pub fn close_fd(mut self, fd: u32) {
        let file = unsafe { bindings::close_fd_get_file(fd) };
        if !file.is_null() {
            self.inner.file = file;

            // SAFETY: Since DeferredFdCloserInner is `#[repr(C)]`, casting the pointers gives a
            // pointer to the `twork` field.
            let inner = Box::into_raw(self.inner) as *mut bindings::callback_head;

            // SAFETY: Getting a pointer to current is always safe.
            let current = unsafe { bindings::get_current() };
            // SAFETY: The `file` pointer points at a valid file.
            unsafe { bindings::get_file(file) };
            // SAFETY: Due to the above `get_file`, even if the current task holds an `fdget` to
            // this file right now, the refcount will not drop to zero until after it is released
            // with `fdput`. This is because when using `fdget`, you must always use `fdput` before
            // returning to userspace, and our task work runs after any `fdget` users have returned
            // to user space.
            //
            // Note: fl_owner_t is currently a void pointer.
            unsafe { bindings::filp_close(file, (*current).files as bindings::fl_owner_t) };
            // SAFETY: The `inner` pointer is compatible with the `do_close_fd` method.
            //
            // The call to `task_work_add` can't fail, because we are scheduling the task work to
            // the current task.
            unsafe {
                bindings::init_task_work(inner, Some(Self::do_close_fd));
                bindings::task_work_add(current, inner, bindings::task_work_notify_mode_TWA_RESUME);
            }
        } else {
            // Free the allocation.
            drop(self.inner);
        }
    }

    unsafe extern "C" fn do_close_fd(inner: *mut bindings::callback_head) {
        // SAFETY: In `close_fd` we use this method together with a pointer that originates from a
        // `Box<DeferredFdCloserInner>`, and we have just been given ownership of that allocation.
        let inner = unsafe { Box::from_raw(inner as *mut DeferredFdCloserInner) };
        // SAFETY: This drops a refcount we acquired in `close_fd`.
        unsafe { bindings::fput(inner.file) };
        // Free the allocation.
        drop(inner);
    }
}

/// Represents the EBADF error code.
///
/// Used for methods that can only fail with EBADF.
pub struct BadFdError;

impl From<BadFdError> for Error {
    fn from(_: BadFdError) -> Error {
        EBADF
    }
}
