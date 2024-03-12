// SPDX-License-Identifier: GPL-2.0

// Copyright (C) 2024 Google LLC.

//! Helpers for implementing list traits safely.

use crate::list::ListLinks;

/// Declares that this type has a `ListLinks<ID>` field at a fixed offset.
///
/// This trait is only used to help implement `ListItem` safely. If `ListItem` is implemented
/// manually, then this trait is not needed.
///
/// # Safety
///
/// All values of this type must have a `ListLinks<ID>` field at the given offset.
pub unsafe trait HasListLinks<const ID: u64 = 0> {
    /// The offset of the `ListLinks` field.
    const OFFSET: usize;

    /// Returns a pointer to the [`ListLinks<T, ID>`] field.
    ///
    /// # Safety
    ///
    /// The provided pointer must point at a valid struct of type `Self`.
    ///
    /// [`ListLinks<T, ID>`]: ListLinks
    // We don't really need this method, but it's necessary for the implementation of
    // `impl_has_work!` to be correct.
    #[inline]
    unsafe fn raw_get_list_links(ptr: *mut Self) -> *mut ListLinks<ID> {
        // SAFETY: The caller promises that the pointer is valid. The implementer promises that the
        // `OFFSET` constant is correct.
        unsafe { (ptr as *mut u8).add(Self::OFFSET) as *mut ListLinks<ID> }
    }
}

/// Implements the [`HasListLinks`] trait for the given type.
#[macro_export]
macro_rules! impl_has_list_links {
    ($(impl$(<$($implarg:ident),*>)?
       HasListLinks$(<$id:tt>)?
       for $self:ident $(<$($selfarg:ty),*>)?
       { self$(.$field:ident)* }
    )*) => {$(
        // SAFETY: The implementation of `raw_get_list_links` only compiles if the field has the
        // right type.
        unsafe impl$(<$($implarg),*>)? $crate::list::HasListLinks$(<$id>)? for
            $self $(<$($selfarg),*>)?
        {
            const OFFSET: usize = ::core::mem::offset_of!(Self, $($field).*) as usize;

            #[inline]
            unsafe fn raw_get_list_links(ptr: *mut Self) -> *mut $crate::list::ListLinks$(<$id>)? {
                // SAFETY: The caller promises that the pointer is not dangling.
                unsafe {
                    ::core::ptr::addr_of_mut!((*ptr)$(.$field)*)
                }
            }
        }
    )*};
}
pub use impl_has_list_links;

/// Implements the [`ListItem`] trait for the given type.
///
/// Assumes that the type implements [`HasListLinks`].
///
/// [`ListItem`]: crate::list::ListItem
#[macro_export]
macro_rules! impl_list_item {
    (
        impl$({$($generics:tt)*})? $crate::list::ListItem<$num:tt> for $t:ty {
            using ListLinks;
        } $($rest:tt)*
    ) => {
        unsafe impl$(<$($generics)*>)? $crate::list::ListItem<$num> for $t {
            unsafe fn view_links(me: *const Self) -> *mut $crate::list::ListLinks<$num> {
                unsafe {
                    <Self as $crate::list::HasListLinks<$num>>::raw_get_list_links(me.cast_mut())
                }
            }

            unsafe fn view_value(me: *mut $crate::list::ListLinks<$num>) -> *const Self {
                let offset = <Self as $crate::list::HasListLinks<$num>>::OFFSET;
                unsafe { (me as *const u8).sub(offset) as *const Self }
            }

            unsafe fn prepare_to_insert(me: *const Self) -> *mut $crate::list::ListLinks<$num> {
                unsafe { <Self as $crate::list::ListItem<$num>>::view_links(me) }
            }

            unsafe fn post_remove(me: *mut $crate::list::ListLinks<$num>) -> *const Self {
                unsafe { <Self as $crate::list::ListItem<$num>>::view_value(me) }
            }
        }
    };
}
pub use impl_list_item;
