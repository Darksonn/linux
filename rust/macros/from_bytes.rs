// SPDX-License-Identifier: GPL-2.0

//! Implements the `FromBytes` derive macro.

#[allow(unused_imports)]
use crate::maybe_from_bytes;
use proc_macro::TokenStream;

pub(crate) fn from_bytes_derive(ts: TokenStream) -> TokenStream {
    // For now, `FromBytes` is the same as `MaybeFromBytes`.
    maybe_from_bytes::from_bytes_derive(ts)
}
