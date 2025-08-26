// SPDX-License-Identifier: GPL-2.0

//! Implements the `AsBytes` derive macro.

#[allow(unused_imports)]
use crate::maybe_as_bytes;
use proc_macro::TokenStream;

pub(crate) fn as_bytes_derive(ts: TokenStream) -> TokenStream {
    // For now, `AsBytes` is the same as `MaybeAsBytes`.
    maybe_as_bytes::maybe_as_bytes_derive(ts)
}
