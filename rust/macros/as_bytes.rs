// SPDX-License-Identifier: GPL-2.0

//! Implements the `AsBytes` derive macro.


use proc_macro::{TokenStream, TokenTree, Delimiter};
use std::iter::Peekable;

fn parse_struct_def(tokens: &mut Peekable<impl Iterator<Item = TokenTree>>) -> (TokenTree, Vec<TokenTree>, Vec<TokenTree>) {
    let name = tokens.next().expect("Missing struct name.");

    let mut generics = Vec::new();
    if let Some(TokenTree::Punct(p)) = tokens.peek() {
        if p.as_char() == '<' {
            tokens.next(); // Consume '<'.
            let mut depth = 1;
            for token in tokens.by_ref() {
                if let TokenTree::Punct(p) = &token {
                    if p.as_char() == '<' {
                        depth += 1;
                    } else if p.as_char() == '>' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                generics.push(token);
            }
        }
    }

    let mut where_clause = Vec::new();
    if let Some(TokenTree::Ident(ident)) = tokens.peek() {
        if ident.to_string() == "where" {
            tokens.next(); // Consume 'where'.
            where_clause.extend(tokens.by_ref());
        }
    }

    (name, generics, where_clause)
}

fn parse_fields(tokens: &mut Peekable<impl Iterator<Item = TokenTree>>) -> Vec<Vec<TokenTree>> {
    let body = tokens.next().and_then(|tt| match tt {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => Some(g),
        _ => None,
    }).expect("Missing struct body.");

    let mut fields = Vec::new();
    let mut field_tokens = Vec::new();
    let mut tokens = body.stream().into_iter().peekable();

    while let Some(token) = tokens.next() {
        if let TokenTree::Punct(p) = &token {
            if p.as_char() == ':' {
                field_tokens.clear();
                let mut depth = 0;
                for ty_token in tokens.by_ref() {
                    if let TokenTree::Punct(p) = &ty_token {
                        match p.as_char() {
                            ',' if depth == 0 => break,
                            '<' => depth += 1,
                            '>' => depth -= 1,
                            _ => {}
                        }
                    }
                    field_tokens.push(ty_token);
                }
                fields.push(field_tokens.clone());
                field_tokens.clear();
                continue;
            }
        }
    }
    fields
}

pub(crate) fn as_bytes_derive(ts: TokenStream) -> TokenStream {
    let mut tokens = ts.into_iter().peekable();

    // Consume attributes until we find the `struct` keyword.
    while let Some(token) = tokens.peek() {
        match token {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                tokens.next(); // Consume '#'.
                tokens.next(); // Consume the attribute body (e.g., `[derive(...)]`).
            }
            TokenTree::Ident(ident) if ident.to_string() == "struct" => {
                break;
            }
            _ => {
                // This handles visibility modifiers like `pub` as well as doc comments.
                tokens.next();
            }
        }
    }

    let struct_kw = tokens.next().expect("Missing `struct` keyword.");
    match &struct_kw {
        TokenTree::Ident(ident) if ident.to_string() == "struct" => (),
        _ => panic!("Expected `struct` keyword, found {}", struct_kw),
    }

    let (name, generics, where_clause) = parse_struct_def(&mut tokens);
    let fields = parse_fields(&mut tokens);

    let generics_ts: TokenStream = generics.into_iter().collect();
    let where_clause_ts: TokenStream = where_clause.into_iter().collect();

    let mut new_where_clause = TokenStream::new();
    if !fields.is_empty() || !where_clause_ts.is_empty() {
        new_where_clause.extend(quote!(where));
    }

    if !where_clause_ts.is_empty() {
        new_where_clause.extend(where_clause_ts);
        if !fields.is_empty() {
            new_where_clause.extend(quote!(,));
        }
    }

    let mut first = true;
    for field in fields {
        if !first {
            new_where_clause.extend(quote!(,));
        }
        let field_ts: TokenStream = field.into_iter().collect();
        new_where_clause.extend(quote!(for<'a> #field_ts: ::ffi::AsBytes));
        first = false;
    }

    quote! {
        unsafe impl<#generics_ts> ::ffi::AsBytes for #name<#generics_ts> #new_where_clause {}
    }
}