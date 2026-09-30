//! `#[derive(Reflect)]` for `orr_reflect`.
//!
//! Works on structs with named fields and no generics. Field offsets come
//! from `core::mem::offset_of!`, so the descriptor always matches the real
//! layout.
//!
//! Struct attribute:
//! - `#[reflect(default_with = "path::to::fn")]`: new components start from that
//!   function (a `fn() -> Self`).
//! - `#[reflect(default)]`: new components start from `Default::default()`
//!   (otherwise all bytes zero, except that each visible field starts from
//!   the `default_value()` of its type, so an `Entity` field is `Entity::NONE`).
//!
//! Field attributes (`#[reflect(...)]`):
//! - `skip`: hide the field (padding, caches, handles). It is not read from
//!   or written to scene files; it keeps the value of `default_value()`.
//! - `bool`: an integer field (`u8` or `u32`) that holds 0 or 1.
//! - `enumeration = "name=0,other=1"`: an unsigned integer field with named values.
//! - `flags = "sensor=1,solid=2"`: an unsigned integer field with named bits.
//! - `range = "0.05..=1000"`: allowed values, inclusive (either end may be empty).
//!
//! Doc comments on the struct and its fields become descriptions in the
//! schema and the inspector.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Expr, ExprLit, Fields, Lit, Meta};

#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn doc_of(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for a in attrs {
        if !a.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &a.meta {
            if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = &nv.value {
                lines.push(s.value().trim().to_string());
            }
        }
    }
    lines.join(" ").trim().to_string()
}

#[derive(Default)]
struct FieldAttrs {
    skip: bool,
    bool_: bool,
    enumeration: Option<String>,
    flags: Option<String>,
    range: Option<String>,
}

fn field_attrs(attrs: &[syn::Attribute]) -> syn::Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for a in attrs {
        if !a.path().is_ident("reflect") {
            continue;
        }
        a.parse_nested_meta(|m| {
            if m.path.is_ident("skip") {
                out.skip = true;
            } else if m.path.is_ident("bool") {
                out.bool_ = true;
            } else if m.path.is_ident("enumeration") {
                out.enumeration = Some(m.value()?.parse::<syn::LitStr>()?.value());
            } else if m.path.is_ident("flags") {
                out.flags = Some(m.value()?.parse::<syn::LitStr>()?.value());
            } else if m.path.is_ident("range") {
                out.range = Some(m.value()?.parse::<syn::LitStr>()?.value());
            } else {
                return Err(m.error("unknown reflect attribute (skip, bool, enumeration, flags, range)"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn named_values(text: &str, span: proc_macro2::Span) -> syn::Result<Vec<(String, u64)>> {
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (n, v) = part.split_once('=').ok_or_else(|| syn::Error::new(span, "expected name=value"))?;
        let v: u64 = v.trim().parse().map_err(|_| syn::Error::new(span, "value must be an unsigned integer"))?;
        out.push((n.trim().to_string(), v));
    }
    Ok(out)
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&input.generics, "Reflect cannot be derived for generic types"));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "Reflect can only be derived for structs"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(name, "Reflect needs a struct with named fields"));
    };

    let mut use_default = false;
    let mut default_with: Option<syn::Path> = None;
    for a in &input.attrs {
        if a.path().is_ident("reflect") {
            a.parse_nested_meta(|m| {
                if m.path.is_ident("default") {
                    use_default = true;
                    Ok(())
                } else if m.path.is_ident("default_with") {
                    default_with = Some(m.value()?.parse::<syn::LitStr>()?.parse()?);
                    Ok(())
                } else {
                    Err(m.error("unknown struct-level reflect attribute (default, default_with)"))
                }
            })?;
        }
    }

    let mut descs = Vec::new();
    let mut defaults = Vec::new();
    for f in &fields.named {
        let attrs = field_attrs(&f.attrs)?;
        if attrs.skip {
            continue;
        }
        let fname = f.ident.as_ref().expect("named field");
        let fstr = fname.to_string();
        let fty = &f.ty;
        let span = fname.span();
        let mut ty: TokenStream2 = if attrs.bool_ {
            quote! { ::orr_reflect::TypeDesc::bool_of_width(::core::mem::size_of::<#fty>()) }
        } else if let Some(text) = &attrs.enumeration {
            let vals = named_values(text, span)?;
            let names = vals.iter().map(|(n, _)| n);
            let nums = vals.iter().map(|(_, v)| v);
            quote! { ::orr_reflect::TypeDesc::enumeration(::core::mem::size_of::<#fty>(), &[#((#names, #nums)),*]) }
        } else if let Some(text) = &attrs.flags {
            let vals = named_values(text, span)?;
            let names = vals.iter().map(|(n, _)| n);
            let nums = vals.iter().map(|(_, v)| v);
            quote! { ::orr_reflect::TypeDesc::flags(::core::mem::size_of::<#fty>(), &[#((#names, #nums)),*]) }
        } else {
            quote! { <#fty as ::orr_reflect::Reflect>::describe() }
        };
        if let Some(r) = &attrs.range {
            ty = quote! { #ty.with_range_str(#r) };
        }
        defaults.push(quote! { v.#fname = <#fty as ::orr_reflect::Reflect>::default_value(); });
        let doc = doc_of(&f.attrs);
        descs.push(quote! {
            ::orr_reflect::FieldDesc::new(#fstr, ::core::mem::offset_of!(#name, #fname), #ty).with_doc(#doc)
        });
    }

    let type_doc = doc_of(&input.attrs);
    let default_fn = if let Some(path) = &default_with {
        quote! { fn default_value() -> Self { #path() } }
    } else if use_default {
        quote! { fn default_value() -> Self { <Self as ::core::default::Default>::default() } }
    } else {
        // Zeroed, except that every visible field starts from the default of its own type
        // (an `Entity` field starts as `Entity::NONE`, not as entity 0).
        quote! {
            fn default_value() -> Self {
                let mut v = <Self as ::orr_reflect::bytemuck::Zeroable>::zeroed();
                #(#defaults)*
                v
            }
        }
    };
    Ok(quote! {
        impl ::orr_reflect::Reflect for #name {
            fn describe() -> ::orr_reflect::TypeDesc {
                ::orr_reflect::TypeDesc::structure(::core::mem::size_of::<Self>(), ::std::vec![#(#descs),*]).with_doc(#type_doc)
            }
            #default_fn
        }
    })
}
