//! `FromJson` derive macro used by `qqmusic-api` response models.
//!
//! The generated implementation mirrors the semantics of the upstream
//! pydantic models:
//!
//! * `#[json(alias = "a")]` / `#[json(alias("a", "b"))]` – keys tried in
//!   order before the field name itself (`populate_by_name`).
//! * `#[json(path = "$.a.b[*]")]` – JSONPath subset evaluated before plain
//!   key lookup.
//! * `#[json(default)]` / `#[json(default = <expr>)]` – value used when the
//!   key is missing or `null`.
//! * `#[json(required)]` – overrides a container level `#[json(default)]`.
//! * `#[json(with = "path::to::fn")]` – custom `fn(&Value) -> Result<T, JsonError>`.
//! * `#[json(flatten)]` – parse the field from the same object.
//! * `#[json(skip)]` – always `Default::default()`.
//!
//! Container attributes:
//!
//! * `#[json(default)]` – every field defaults unless marked `required`.
//! * `#[json(preprocess = "path::to::fn")]` – `fn(&mut Value)` executed on a
//!   cloned input before field extraction.
//! * `#[json(crate = "path")]` – path of the runtime crate (defaults to
//!   `::qqmusic_api`).

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Data, DeriveInput, Expr, Fields, GenericParam, LitStr, Path, Token, parenthesized, parse_macro_input, parse_quote,
    punctuated::Punctuated, spanned::Spanned,
};

/// Derive `qqmusic_api::json::FromJson` for a struct with named fields.
#[proc_macro_derive(FromJson, attributes(json))]
pub fn derive_from_json(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[derive(Default)]
struct ContainerAttrs {
    default: bool,
    preprocess: Option<Path>,
    krate: Option<Path>,
}

enum DefaultKind {
    Trait,
    Expr(Box<Expr>),
}

#[derive(Default)]
struct FieldAttrs {
    aliases: Vec<LitStr>,
    path: Option<LitStr>,
    default: Option<DefaultKind>,
    required: bool,
    with: Option<Path>,
    flatten: bool,
    skip: bool,
}

fn parse_container_attrs(input: &DeriveInput) -> syn::Result<ContainerAttrs> {
    let mut attrs = ContainerAttrs::default();
    for attr in &input.attrs {
        if !attr.path().is_ident("json") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("default") {
                attrs.default = true;
                Ok(())
            } else if meta.path.is_ident("preprocess") {
                let lit: LitStr = meta.value()?.parse()?;
                attrs.preprocess = Some(lit.parse()?);
                Ok(())
            } else if meta.path.is_ident("crate") {
                let lit: LitStr = meta.value()?.parse()?;
                attrs.krate = Some(lit.parse()?);
                Ok(())
            } else {
                Err(meta.error("unsupported container attribute"))
            }
        })?;
    }
    Ok(attrs)
}

fn parse_field_attrs(field: &syn::Field) -> syn::Result<FieldAttrs> {
    let mut attrs = FieldAttrs::default();
    for attr in &field.attrs {
        if !attr.path().is_ident("json") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("alias") {
                if meta.input.peek(Token![=]) {
                    let lit: LitStr = meta.value()?.parse()?;
                    attrs.aliases.push(lit);
                } else {
                    let content;
                    parenthesized!(content in meta.input);
                    let lits = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
                    attrs.aliases.extend(lits);
                }
                Ok(())
            } else if meta.path.is_ident("path") {
                let lit: LitStr = meta.value()?.parse()?;
                validate_path(&lit)?;
                attrs.path = Some(lit);
                Ok(())
            } else if meta.path.is_ident("default") {
                if meta.input.peek(Token![=]) {
                    let expr: Expr = meta.value()?.parse()?;
                    attrs.default = Some(DefaultKind::Expr(Box::new(expr)));
                } else {
                    attrs.default = Some(DefaultKind::Trait);
                }
                Ok(())
            } else if meta.path.is_ident("required") {
                attrs.required = true;
                Ok(())
            } else if meta.path.is_ident("with") {
                let lit: LitStr = meta.value()?.parse()?;
                attrs.with = Some(lit.parse()?);
                Ok(())
            } else if meta.path.is_ident("flatten") {
                attrs.flatten = true;
                Ok(())
            } else if meta.path.is_ident("skip") {
                attrs.skip = true;
                Ok(())
            } else {
                Err(meta.error("unsupported field attribute"))
            }
        })?;
    }
    if attrs.required && attrs.default.is_some() {
        return Err(syn::Error::new(field.span(), "`required` and `default` are mutually exclusive"));
    }
    Ok(attrs)
}

/// Light compile-time validation of the supported JSONPath subset.
fn validate_path(lit: &LitStr) -> syn::Result<()> {
    let value = lit.value();
    if !value.starts_with('$') {
        return Err(syn::Error::new(lit.span(), "JSONPath must start with `$`"));
    }
    let mut rest = &value[1..];
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('.') {
            let end = stripped.find(['.', '[']).unwrap_or(stripped.len());
            if end == 0 {
                return Err(syn::Error::new(lit.span(), "empty JSONPath segment"));
            }
            rest = &stripped[end..];
        } else if let Some(stripped) = rest.strip_prefix('[') {
            let end = stripped.find(']').ok_or_else(|| syn::Error::new(lit.span(), "unterminated `[` in JSONPath"))?;
            let inner = &stripped[..end];
            if inner != "*" && inner.parse::<i64>().is_err() {
                return Err(syn::Error::new(lit.span(), "only `[*]` and `[<index>]` are supported in JSONPath"));
            }
            rest = &stripped[end + 1..];
        } else {
            return Err(syn::Error::new(lit.span(), "invalid JSONPath syntax"));
        }
    }
    Ok(())
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let container = parse_container_attrs(&input)?;
    let krate: Path = container.krate.clone().unwrap_or_else(|| parse_quote!(::qqmusic_api));
    let name = &input.ident;
    let name_str = name.to_string();

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(syn::Error::new(input.span(), "FromJson only supports structs with named fields"));
            }
        },
        _ => {
            return Err(syn::Error::new(input.span(), "FromJson only supports structs"));
        }
    };

    let mut generics = input.generics.clone();
    for param in &mut generics.params {
        if let GenericParam::Type(ty) = param {
            ty.bounds.push(parse_quote!(#krate::json::FromJson));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let mut parse_stmts = Vec::new();
    let mut default_inits = Vec::new();
    let mut idents = Vec::new();

    for field in fields {
        let attrs = parse_field_attrs(field)?;
        let ident = field.ident.clone().expect("named field");
        let ty = &field.ty;
        let field_name = ident.to_string();
        let field_name = field_name.strip_prefix("r#").unwrap_or(&field_name).to_string();
        idents.push(ident.clone());

        if attrs.skip {
            parse_stmts.push(quote! {
                let #ident: #ty = ::core::default::Default::default();
            });
            default_inits.push(quote! { #ident: ::core::default::Default::default() });
            continue;
        }

        if attrs.flatten {
            parse_stmts.push(quote! {
                let #ident: #ty = <#ty as #krate::json::FromJson>::from_json(__value)
                    .map_err(|e| e.with_field(#field_name))?;
            });
            default_inits.push(quote! { #ident: <#ty as #krate::json::FromJson>::json_default()? });
            continue;
        }

        let mut keys: Vec<String> = attrs.aliases.iter().map(LitStr::value).collect();
        if !keys.iter().any(|k| k == &field_name) {
            keys.push(field_name.clone());
        }
        let path_tokens = match &attrs.path {
            Some(lit) => quote! { ::core::option::Option::Some(#lit) },
            None => quote! { ::core::option::Option::None },
        };

        let convert = match &attrs.with {
            Some(path) => quote! { #path },
            None => quote! { <#ty as #krate::json::FromJson>::from_json },
        };

        let use_trait_default = matches!(attrs.default, Some(DefaultKind::Trait))
            || (container.default && !attrs.required && attrs.default.is_none());

        let (missing, default_init) = match &attrs.default {
            Some(DefaultKind::Expr(expr)) => {
                (quote! { ::core::convert::Into::into(#expr) }, quote! { ::core::convert::Into::into(#expr) })
            }
            _ if use_trait_default => (
                quote! {
                    <#ty as #krate::json::FromJson>::json_default()
                        .ok_or_else(|| #krate::json::JsonError::missing(#field_name))?
                },
                quote! { <#ty as #krate::json::FromJson>::json_default()? },
            ),
            _ => (
                quote! {
                    <#ty as #krate::json::FromJson>::from_missing()
                        .ok_or_else(|| #krate::json::JsonError::missing(#field_name))?
                },
                quote! { <#ty as #krate::json::FromJson>::from_missing()? },
            ),
        };

        parse_stmts.push(quote! {
            let #ident: #ty = match #krate::json::__private::lookup(
                __value,
                __obj,
                &[#(#keys),*],
                #path_tokens,
            ) {
                ::core::option::Option::Some(__found) => #convert(&*__found)
                    .map_err(|e| e.with_field(#field_name))?,
                ::core::option::Option::None => #missing,
            };
        });
        default_inits.push(quote! { #ident: #default_init });
    }

    let preprocess = match &container.preprocess {
        Some(path) => quote! {
            let mut __cloned = ::core::clone::Clone::clone(__value);
            #path(&mut __cloned);
            let __value: &#krate::json::Value = &__cloned;
        },
        None => quote! {},
    };

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::json::FromJson for #name #ty_generics #where_clause {
            fn from_json(
                __value: &#krate::json::Value,
            ) -> ::core::result::Result<Self, #krate::json::JsonError> {
                #preprocess
                let __obj = #krate::json::__private::expect_object(__value, #name_str)?;
                let _ = __obj;
                #(#parse_stmts)*
                ::core::result::Result::Ok(Self { #(#idents),* })
            }

            fn json_default() -> ::core::option::Option<Self> {
                ::core::option::Option::Some(Self { #(#default_inits),* })
            }
        }
    })
}
