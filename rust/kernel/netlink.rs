// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2026 Google LLC.

//! Rust support for generic netlink.
//!
//! Currently only supports exposing multicast groups.
//!
//! C header: [`include/net/genetlink.h`](srctree/include/net/genetlink.h)
#![cfg(CONFIG_NET)]

use kernel::{
    alloc::{self, AllocError},
    error::to_result,
    prelude::*,
    transmute::AsBytes,
    types::Opaque,
    ThisModule,
};

use core::{
    mem::ManuallyDrop,
    ptr::NonNull, //
};

/// The default netlink message size.
pub const GENLMSG_DEFAULT_SIZE: usize = bindings::GENLMSG_DEFAULT_SIZE;

/// A wrapper around `struct sk_buff` for generic netlink messages.
///
/// # Invariants
///
/// The pointer has ownership over a valid `sk_buff`.
pub struct SkBuff {
    skb: NonNull<kernel::bindings::sk_buff>,
}

impl SkBuff {
    /// Creates a new `SkBuff` with the given size.
    pub fn new(size: usize, flags: alloc::Flags) -> Result<SkBuff, AllocError> {
        // SAFETY: `genlmsg_new` only requires its arguments to be valid integers.
        let skb = unsafe { bindings::genlmsg_new(size, flags.as_raw()) };
        let skb = NonNull::new(skb).ok_or(AllocError)?;
        Ok(SkBuff { skb })
    }

    /// Puts a generic netlink header into the `SkBuff`.
    pub fn genlmsg_put(
        self,
        portid: u32,
        seq: u32,
        family: &'static Family,
        cmd: u8,
    ) -> Result<GenlMsg, AllocError> {
        let skb = self.skb.as_ptr();
        // SAFETY: The skb and family pointers are valid.
        let hdr = unsafe { bindings::genlmsg_put(skb, portid, seq, family.as_raw(), 0, cmd) };
        let hdr = NonNull::new(hdr).ok_or(AllocError)?;
        Ok(GenlMsg { skb: self, hdr })
    }
}

impl Drop for SkBuff {
    fn drop(&mut self) {
        // SAFETY: We have ownership over the `sk_buff`, so we may free it.
        unsafe { bindings::nlmsg_free(self.skb.as_ptr()) }
    }
}

/// A generic netlink message being constructed.
///
/// # Invariants
///
/// `hdr` references the header in this netlink message.
pub struct GenlMsg {
    skb: SkBuff,
    hdr: NonNull<c_void>,
}

impl GenlMsg {
    /// Puts an attribute into the message.
    #[inline]
    fn put<T>(&mut self, attrtype: c_int, value: &T) -> Result
    where
        T: ?Sized + AsBytes,
    {
        let skb = self.skb.skb.as_ptr();
        let len = size_of_val(value);
        let ptr = core::ptr::from_ref(value).cast::<c_void>();
        // SAFETY: `skb` is valid by `SkBuff` type invariants, and the provided value is readable
        // and initialized for its `size_of` bytes.
        to_result(unsafe { bindings::nla_put(skb, attrtype, len as c_int, ptr) })
    }

    /// Puts a `u32` attribute into the message.
    #[inline]
    pub fn put_u32(&mut self, attrtype: c_int, value: u32) -> Result {
        self.put(attrtype, &value)
    }

    /// Puts a string attribute into the message.
    #[inline]
    pub fn put_string(&mut self, attrtype: c_int, value: &CStr) -> Result {
        self.put(attrtype, value.to_bytes_with_nul())
    }

    /// Puts a flag attribute into the message.
    #[inline]
    pub fn put_flag(&mut self, attrtype: c_int) -> Result {
        let skb = self.skb.skb.as_ptr();
        // SAFETY: `skb` is valid by `SkBuff` type invariants, and a null pointer is valid when the
        // length is zero.
        to_result(unsafe { bindings::nla_put(skb, attrtype, 0, core::ptr::null()) })
    }

    /// Sends the generic netlink message as a multicast message.
    #[inline]
    pub fn multicast(
        self,
        family: &'static Family,
        portid: u32,
        group: u32,
        flags: alloc::Flags,
    ) -> Result {
        let me = ManuallyDrop::new(self);
        // SAFETY: The `skb` and `family` pointers are valid. We pass ownership of the `skb` to
        // `genlmsg_multicast` by not dropping `self`.
        unsafe {
            bindings::genlmsg_end(me.skb.skb.as_ptr(), me.hdr.as_ptr());
            to_result(bindings::genlmsg_multicast(
                family.as_raw(),
                me.skb.skb.as_ptr(),
                portid,
                group,
                flags.as_raw(),
            ))
        }
    }
}
impl Drop for GenlMsg {
    fn drop(&mut self) {
        // SAFETY: The `hdr` pointer references the header of this generic netlink message.
        unsafe { bindings::genlmsg_cancel(self.skb.skb.as_ptr(), self.hdr.as_ptr()) };
    }
}

/// A generic netlink family.
#[repr(transparent)]
pub struct Family {
    inner: Opaque<bindings::genl_family>,
}

// SAFETY: The `Family` type is thread safe.
unsafe impl Sync for Family {}

impl Family {
    /// Creates a new `Family` instance.
    pub const fn const_new(
        module: &ThisModule,
        name: &[u8],
        version: u32,
        mcgrps: &'static [MulticastGroup],
    ) -> Family {
        let n_mcgrps = mcgrps.len() as u8;
        if n_mcgrps as usize != mcgrps.len() {
            panic!("too many mcgrps");
        }
        let mut genl_family = bindings::genl_family {
            version,
            // SAFETY: This bitfield is represented as an u8.
            _bitfield_1: unsafe {
                core::mem::transmute::<u8, bindings::__BindgenBitfieldUnit<[u8; 1]>>(0b11u8)
            },
            module: module.as_ptr(),
            mcgrps: mcgrps.as_ptr().cast(),
            n_mcgrps,
            ..pin_init::zeroed()
        };
        if CStr::from_bytes_with_nul(name).is_err() {
            panic!("genl_family name not nul-terminated");
        }
        if genl_family.name.len() < name.len() {
            panic!("genl_family name too long");
        }
        let mut i = 0;
        while i < name.len() {
            genl_family.name[i] = name[i];
            i += 1;
        }
        Family {
            inner: Opaque::new(genl_family),
        }
    }

    /// Checks if there are any listeners for the given multicast group.
    pub fn has_listeners(&self, group: u32) -> bool {
        // SAFETY: The family and init_net pointers are valid.
        unsafe {
            bindings::genl_has_listeners(self.as_raw(), &raw mut bindings::init_net, group) != 0
        }
    }

    /// Returns a raw pointer to the underlying `genl_family` structure.
    pub fn as_raw(&self) -> *mut bindings::genl_family {
        self.inner.get()
    }
}

/// A generic netlink multicast group.
#[repr(transparent)]
pub struct MulticastGroup {
    // No Opaque because fully immutable
    group: bindings::genl_multicast_group,
}

// SAFETY: Pure data so thread safe.
unsafe impl Sync for MulticastGroup {}

impl MulticastGroup {
    /// Creates a new `MulticastGroup` instance.
    pub const fn const_new(name: &CStr) -> MulticastGroup {
        let mut group: bindings::genl_multicast_group = pin_init::zeroed();

        let name = name.to_bytes_with_nul();
        if group.name.len() < name.len() {
            panic!("genl_multicast_group name too long");
        }
        let mut i = 0;
        while i < name.len() {
            group.name[i] = name[i];
            i += 1;
        }

        MulticastGroup { group }
    }
}

/// A registration of a generic netlink family.
///
/// This type represents the registration of a [`Family`]. When an instance of this type is
/// dropped, its respective generic netlink family will be unregistered from the system.
///
/// # Invariants
///
/// `self.family` always holds a valid reference to an initialized and registered [`genl_family`].
pub struct Registration {
    family: &'static Family,
}

impl Family {
    /// Registers the generic netlink family with the kernel.
    pub fn register(&'static self) -> Result<Registration> {
        // SAFETY: `self.as_raw()` is a valid pointer to a `genl_family` struct.
        // The `genl_family` struct is static, so it will outlive the registration.
        to_result(unsafe { bindings::genl_register_family(self.as_raw()) })?;
        Ok(Registration { family: self })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: `self.family.as_raw()` is a valid pointer to a registered `genl_family` struct.
        // The `Registration` struct ensures that `genl_unregister_family` is called exactly once
        // for this family when it goes out of scope.
        unsafe { bindings::genl_unregister_family(self.family.as_raw()) };
    }
}
