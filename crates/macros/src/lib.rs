//! Generates the Rust-backed declarations required by the Whim runtime.

use std::hint;

use proc_macro::TokenStream;

use crate::built_in::DeclarationKind;

mod built_in;
mod core;

/// Declares a Rust-backed Whim function from its complete Whim signature.
#[proc_macro_attribute]
pub fn whim_function(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Function, attribute, item)
}

/// Declares a Rust-backed Whim constant from a Rust `const` item.
#[proc_macro_attribute]
pub fn whim_constant(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Constant, attribute, item)
}

/// Declares the body and signature of a Rust-backed closure.
#[proc_macro_attribute]
pub fn whim_closure(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Closure, attribute, item)
}

/// Declares a Rust-backed Whim interface from a Rust trait.
#[proc_macro_attribute]
pub fn whim_interface(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Interface, attribute, item)
}

/// Declares a Rust-backed Whim class and its optional inline Rust state.
#[proc_macro_attribute]
pub fn whim_class(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Class, attribute, item)
}

/// Declares the Rust-backed methods attached to a `whim_class`.
#[proc_macro_attribute]
pub fn whim_methods(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Method, attribute, item)
}

/// Declares a Rust-backed Whim enum from a unit Rust enum.
#[proc_macro_attribute]
pub fn whim_enum(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Enumeration, attribute, item)
}

/// Declares a Rust-backed Whim newtype from a fieldless Rust struct.
#[proc_macro_attribute]
pub fn whim_newtype(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand_built_in(DeclarationKind::Newtype, attribute, item)
}

fn expand_built_in(
    kind: DeclarationKind,
    attribute: TokenStream,
    item: TokenStream,
) -> TokenStream {
    match built_in::expand(kind, attribute.into(), item.into()) {
        Ok(expanded) => expanded.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Collects every Rust-backed declaration into the mandatory core table.
#[proc_macro]
pub fn whim_core(input: TokenStream) -> TokenStream {
    match core::expand(input.into()) {
        Ok(expanded) => expanded.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[inline(always)]
unsafe fn unreachable_invariant(message: &'static str) -> ! {
    if cfg!(debug_assertions) {
        panic!("whim-macros invariant violated: {message}");
    } else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { hint::unreachable_unchecked() }
    }
}
