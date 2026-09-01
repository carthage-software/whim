//! The expansion of `whim_core!`.

use crate::unreachable_invariant;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::Token;
use syn::parse::Parse;
use syn::parse::ParseStream;

struct CoreInput {
    functions: Vec<syn::Path>,
    classes: Vec<syn::Path>,
    interfaces: Vec<syn::Path>,
    enums: Vec<syn::Path>,
    newtypes: Vec<syn::Path>,
    constants: Vec<syn::Path>,
}

impl Parse for CoreInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut functions = Vec::new();
        let mut classes = Vec::new();
        let mut interfaces = Vec::new();
        let mut enums = Vec::new();
        let mut newtypes = Vec::new();
        let mut constants = Vec::new();
        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            if key == "functions" {
                functions = path_list(input)?;
            } else if key == "classes" {
                classes = path_list(input)?;
            } else if key == "interfaces" {
                interfaces = path_list(input)?;
            } else if key == "enums" {
                enums = path_list(input)?;
            } else if key == "newtypes" {
                newtypes = path_list(input)?;
            } else if key == "constants" {
                constants = path_list(input)?;
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    "expected `functions`, `classes`, `interfaces`, `enums`, `newtypes`, or `constants`",
                ));
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(Self {
            functions,
            classes,
            interfaces,
            enums,
            newtypes,
            constants,
        })
    }
}

fn path_list(input: ParseStream<'_>) -> syn::Result<Vec<syn::Path>> {
    let content;
    syn::bracketed!(content in input);
    Ok(content
        .parse_terminated(<syn::Path as Parse>::parse, Token![,])?
        .into_iter()
        .collect())
}

fn specification_path(declared: &syn::Path, prefix: &str) -> syn::Path {
    let mut path = declared.clone();
    let Some(last) = path.segments.last_mut() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a declared item path has a last segment") }
    };
    last.ident = format_ident!("{prefix}{}", last.ident);
    path
}

pub(super) fn expand(input: TokenStream) -> Result<TokenStream, syn::Error> {
    let core: CoreInput = syn::parse2(input)?;

    let functions: Vec<syn::Path> = core
        .functions
        .iter()
        .map(|path| specification_path(path, "__whim_function_"))
        .collect();
    let classes: Vec<syn::Path> = core
        .classes
        .iter()
        .map(|path| specification_path(path, "__whim_class_"))
        .collect();
    let interfaces: Vec<syn::Path> = core
        .interfaces
        .iter()
        .map(|path| specification_path(path, "__whim_interface_"))
        .collect();
    let enums: Vec<syn::Path> = core
        .enums
        .iter()
        .map(|path| specification_path(path, "__whim_enum_"))
        .collect();
    let newtypes: Vec<syn::Path> = core
        .newtypes
        .iter()
        .map(|path| specification_path(path, "__whim_newtype_"))
        .collect();
    let constants: Vec<syn::Path> = core
        .constants
        .iter()
        .map(|path| specification_path(path, "__whim_constant_"))
        .collect();

    Ok(quote! {
        #[must_use]
        pub(crate) fn declarations() -> crate::builtin::spec::CoreDeclarations {
            crate::builtin::spec::CoreDeclarations {
                functions: ::std::vec![#(#functions()),*].into_boxed_slice(),
                classes: ::std::vec![#(#classes()),*].into_boxed_slice(),
                interfaces: ::std::vec![#(#interfaces()),*].into_boxed_slice(),
                enums: ::std::vec![#(#enums()),*].into_boxed_slice(),
                newtypes: ::std::vec![#(#newtypes()),*].into_boxed_slice(),
                constants: &[#(#constants),*],
            }
        }
    })
}
