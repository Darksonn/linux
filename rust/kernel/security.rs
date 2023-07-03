// SPDX-License-Identifier: GPL-2.0

//! Linux Security Modules (LSM).
//!
//! C header: [`include/linux/security.h`](../../../../include/linux/security.h).

use crate::{
    bindings,
    cred::Credential,
    error::{to_result, Result},
};

/// Calls the security modules to determine if the given task can become the manager of a binder
/// context.
pub fn binder_set_context_mgr(mgr: &Credential) -> Result {
    // SAFETY: `mrg.0` is valid because the shared reference guarantees a nonzero refcount.
    to_result(unsafe { bindings::security_binder_set_context_mgr(mgr.0.get()) })
}

/// Calls the security modules to determine if binder transactions are allowed from task `from` to
/// task `to`.
pub fn binder_transaction(from: &Credential, to: &Credential) -> Result {
    // SAFETY: `from` and `to` are valid because the shared references guarantee nonzero refcounts.
    to_result(unsafe { bindings::security_binder_transaction(from.0.get(), to.0.get()) })
}

/// A security context string.
///
/// The struct has the invariant that it always contains a valid security context.
pub struct SecurityCtx {
    secdata: *mut core::ffi::c_char,
    seclen: usize,
}

impl SecurityCtx {
    /// Get the security context given its id.
    pub fn from_secid(secid: u32) -> Result<Self> {
        let mut secdata = core::ptr::null_mut();
        let mut seclen = 0;
        // SAFETY: Just a C FFI call. The pointers are valid for writes.
        unsafe {
            to_result(bindings::security_secid_to_secctx(
                secid,
                &mut secdata,
                &mut seclen,
            ))?;
        }

        // If the above call did not fail, then we have a valid security
        // context, so the invariants are not violated.
        Ok(Self {
            secdata,
            seclen: usize::try_from(seclen).unwrap(),
        })
    }

    /// Returns whether the security context is empty.
    pub fn is_empty(&self) -> bool {
        self.seclen == 0
    }

    /// Returns the length of this security context.
    pub fn len(&self) -> usize {
        self.seclen
    }

    /// Returns the bytes for this security context.
    pub fn as_bytes(&self) -> &[u8] {
        let mut ptr = self.secdata;
        if ptr.is_null() {
            // Many C APIs will use null pointers for strings of length zero, but
            // `slice::from_raw_parts` doesn't allow the pointer to be null even if the length is
            // zero. Replace the pointer with a dangling but non-null pointer in this case.
            debug_assert_eq!(self.seclen, 0);
            ptr = core::ptr::NonNull::dangling().as_ptr();
        }

        // SAFETY: The call to `security_secid_to_secctx` guarantees that the pointer is valid for
        // `seclen` bytes. Furthermore, if the length is zero, then we have ensured that the
        // pointer is not null.
        unsafe { core::slice::from_raw_parts(ptr.cast(), self.seclen) }
    }
}

impl Drop for SecurityCtx {
    fn drop(&mut self) {
        // SAFETY: This frees a pointer that came from a successful call to
        // `security_secid_to_secctx`.
        unsafe {
            bindings::security_release_secctx(self.secdata, self.seclen as u32);
        }
    }
}
