//! The signature-string built-in declaration path.

mod attributes;
mod class;
mod closure;
mod constant;
mod enumeration;
mod function;
mod generics;
mod handler;
mod interface;
mod method;
mod newtype;
mod signature;
mod type_spec;

use std::collections::HashSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Attribute;
use syn::Ident;
use syn::LitStr;
use syn::punctuated::Punctuated;
use syn::token::Comma;
use whim_syn::arena;
use whim_syn::fragment;

use crate::built_in::attributes::AttributeArguments;
use crate::built_in::attributes::constant_value_tokens;
use crate::built_in::attributes::visibility_tokens;

pub(super) enum DeclarationKind {
    Class,
    Closure,
    Constant,
    Enumeration,
    Function,
    Interface,
    Method,
    Newtype,
}

pub(super) fn expand(
    kind: DeclarationKind,
    attribute: TokenStream,
    item: TokenStream,
) -> syn::Result<TokenStream> {
    match kind {
        DeclarationKind::Class => class::expand(attribute, item),
        DeclarationKind::Closure => closure::expand(attribute, item),
        DeclarationKind::Constant => constant::expand(attribute, item),
        DeclarationKind::Enumeration => enumeration::expand(attribute, item),
        DeclarationKind::Function => function::expand(attribute, item),
        DeclarationKind::Interface => interface::expand(attribute, item),
        DeclarationKind::Method => method::expand(attribute, item),
        DeclarationKind::Newtype => newtype::expand(attribute, item),
    }
}

fn take_attribute(attributes: &mut Vec<Attribute>, name: &str) -> Option<Attribute> {
    let index = attributes
        .iter()
        .position(|attribute| attribute.path().is_ident(name))?;

    Some(attributes.remove(index))
}

fn callable_markers(arguments: &AttributeArguments) -> TokenStream {
    callable_marker_tokens(
        arguments.has_flag("no_track_caller"),
        arguments.has_flag("no_trace_boundary"),
        arguments.has_flag("must_use"),
    )
}

fn callable_marker_tokens(
    no_track_caller: bool,
    no_trace_boundary: bool,
    must_use: bool,
) -> TokenStream {
    let track_caller = if no_track_caller {
        quote!(::core::option::Option::Some(false))
    } else {
        quote!(::core::option::Option::None)
    };
    let trace_boundary = if no_trace_boundary {
        quote!(::core::option::Option::Some(false))
    } else {
        quote!(::core::option::Option::None)
    };
    quote! {
        crate::bytecode::unit::BuiltInCallableMarkers {
            track_caller: #track_caller,
            trace_boundary: #trace_boundary,
            must_use: #must_use,
        }
    }
}

fn validate_must_use(must_use: bool, return_type: Option<&str>, span: Span) -> syn::Result<()> {
    if must_use && matches!(return_type, Some("void" | "never")) {
        return Err(syn::Error::new(
            span,
            "a callable returning void or never cannot be must-use",
        ));
    }

    Ok(())
}

/// Records successful results only for callables declared must-use.
fn mark_must_use(body: TokenStream, name: &LitStr, must_use: bool) -> TokenStream {
    if !must_use {
        return body;
    }

    quote! {
        let __whim_outcome = { #body };
        if __whim_outcome.is_ok() {
            __whim_scope.vm.remember_built_in_must_use(#name);
        }
        __whim_outcome
    }
}

fn split_name(source: &str) -> (String, String) {
    let boundary = source.find(['<', '(']).unwrap_or(source.len());
    let (name, tail) = source.split_at(boundary);

    (name.trim().to_owned(), tail.to_owned())
}

fn attribute_name(attribute: &Attribute) -> Option<String> {
    attribute
        .path()
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn string_arguments(attribute: &Attribute) -> syn::Result<Vec<String>> {
    let literals = attribute.parse_args_with(Punctuated::<LitStr, Comma>::parse_terminated)?;

    Ok(literals
        .into_iter()
        .map(|literal| literal.value())
        .collect())
}

fn base_spec(source: &str, names: &HashSet<String>) -> syn::Result<TokenStream> {
    let arena = arena::LocalArena::new();
    let parsed = fragment::parse_type(&arena, source).map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("invalid base `{source}`: {error}"),
        )
    })?;

    type_spec::base_spec(parsed, names)
}

fn class_constant_spec(arguments: &AttributeArguments) -> syn::Result<TokenStream> {
    arguments.validate(2, &[], &["visibility", "literal"])?;
    let name = arguments
        .positional_string(0)?
        .ok_or_else(|| syn::Error::new(Span::call_site(), "a class-like constant needs a name"))?;
    let declared_type = arguments.positional_string(1)?.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "a class-like constant needs a declared type",
        )
    })?;
    let arena = arena::LocalArena::new();
    let parsed = fragment::parse_type(&arena, &declared_type.value()).map_err(|error| {
        syn::Error::new(
            declared_type.span(),
            format!("invalid class-like constant type: {error}"),
        )
    })?;
    let type_spec = type_spec::type_spec(parsed, &HashSet::new())?;
    let visibility = visibility_tokens(arguments.value_string("visibility")?.as_deref())?;
    let literal = arguments.value_lit("literal")?.ok_or_else(|| {
        syn::Error::new(name.span(), "a class-like constant needs a literal value")
    })?;
    let value = constant_value_tokens(literal)?;

    Ok(quote! {
        crate::builtin::spec::ClassConstantSpec {
            name: #name,
            visibility: #visibility,
            type_spec: #type_spec,
            value: #value,
        }
    })
}

fn empty_method_provider(representation: &Ident) -> TokenStream {
    let provider = quote::format_ident!("__WhimMethods_{representation}");
    let default_trait = quote::format_ident!("__WhimDefaultMethods_{representation}");

    quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub(crate) struct #provider;

        #[doc(hidden)]
        #[allow(non_camel_case_types, dead_code)]
        trait #default_trait {
            fn __whim_methods(&self)
            -> ::std::boxed::Box<[crate::builtin::spec::MethodSpec]>;
        }

        impl #default_trait for #provider {
            fn __whim_methods(&self)
            -> ::std::boxed::Box<[crate::builtin::spec::MethodSpec]> {
                ::std::boxed::Box::from([])
            }
        }
    }
}
