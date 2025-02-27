// SPDX-License-Identifier: GPL-2.0

use crate::helpers::function_name;
use proc_macro::TokenStream;

pub(crate) fn export(_attr: TokenStream, ts: TokenStream) -> TokenStream {
    let Some(name) = function_name(ts.clone()) else {
        return "::core::compile_error!(\"The #[export] attribute must be used on a function.\");"
            .parse::<TokenStream>()
            .unwrap();
    };

    let signature_check = quote!(
        const _: () = {
            if true {
                ::kernel::bindings::#name
            } else {
                #name
            };
        };
    );

    let no_mangle = "#[no_mangle]".parse::<TokenStream>().unwrap();
    TokenStream::from_iter([signature_check, no_mangle, ts])
}
