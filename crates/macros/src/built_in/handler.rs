//! The shared body of a built-in handler shim.

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;
use syn::FnArg;
use syn::ReturnType;
use syn::Type;
use syn::punctuated::Punctuated;
use syn::token::Comma;

#[derive(Clone, Copy)]
pub(super) enum Receiver {
    None,
    Instance,
    Static,
    Closure { parameters: usize },
}

pub(super) fn shim_body(
    callee: &TokenStream,
    inputs: &Punctuated<FnArg, Comma>,
    output: &ReturnType,
    receiver: Receiver,
) -> syn::Result<TokenStream> {
    let shape = Shape::classify(inputs)?;
    let outcome = Outcome::classify(output)?;

    let window_arguments = match receiver {
        Receiver::None => quote!(__whim_window),
        Receiver::Instance | Receiver::Static => quote!(&__whim_window[1..]),
        Receiver::Closure { parameters } => quote!(&__whim_window[..#parameters]),
    };
    let receiver_binding = match receiver {
        Receiver::Instance => {
            quote!(__whim_scope.set_receiver(::core::clone::Clone::clone(&__whim_window[0]));)
        }
        Receiver::Closure { parameters } => {
            quote!(__whim_scope.set_captures(&__whim_window[#parameters..]);)
        }
        Receiver::None | Receiver::Static => quote!(),
    };
    let arguments_binding = if shape.uses_arguments() {
        quote! {
            let __whim_arguments = crate::builtin::arguments::Arguments::new(
                #window_arguments,
                __whim_scope.vm.heap(),
            );
        }
    } else {
        quote!()
    };
    let body = outcome.wrap(shape.call(callee));

    Ok(quote! {
        #receiver_binding
        #arguments_binding
        #body
    })
}

#[derive(Clone, Copy)]
enum Shape {
    ContextArguments,
    Context,
    Arguments,
    Empty,
}

impl Shape {
    fn classify(inputs: &Punctuated<FnArg, Comma>) -> syn::Result<Self> {
        let kinds = inputs
            .iter()
            .map(classify_parameter)
            .collect::<syn::Result<Vec<_>>>()?;
        match kinds.as_slice() {
            [Parameter::Context, Parameter::Arguments] => Ok(Self::ContextArguments),
            [Parameter::Context] => Ok(Self::Context),
            [Parameter::Arguments] => Ok(Self::Arguments),
            [] => Ok(Self::Empty),
            _ => Err(syn::Error::new(
                Span::call_site(),
                "a built-in handler takes one of (context, arguments), (context), \
                 (arguments), or ()",
            )),
        }
    }

    const fn uses_arguments(self) -> bool {
        matches!(self, Self::ContextArguments | Self::Arguments)
    }

    fn call(self, callee: &TokenStream) -> TokenStream {
        match self {
            Self::ContextArguments => quote!(#callee(__whim_scope, __whim_arguments)),
            Self::Context => quote!(#callee(__whim_scope)),
            Self::Arguments => quote!(#callee(__whim_arguments)),
            Self::Empty => quote!(#callee()),
        }
    }
}

enum Parameter {
    Context,
    Arguments,
}

fn classify_parameter(input: &FnArg) -> syn::Result<Parameter> {
    let FnArg::Typed(typed) = input else {
        return Err(syn::Error::new_spanned(
            input,
            "a built-in handler takes no `self`; the receiver comes from the context",
        ));
    };

    match type_head(&typed.ty).as_deref() {
        Some("Context") => Ok(Parameter::Context),
        Some("Arguments") => Ok(Parameter::Arguments),
        _ => Err(syn::Error::new_spanned(
            &typed.ty,
            "a built-in handler parameter must be `&mut Context` or `Arguments`",
        )),
    }
}

/// How the handler's return is adapted to `Result<Value, Throw>`.
#[derive(Clone, Copy)]
enum Outcome {
    Fallible,
    Infallible,
    Void,
}

impl Outcome {
    fn classify(output: &ReturnType) -> syn::Result<Self> {
        match output {
            ReturnType::Default => Ok(Self::Void),
            ReturnType::Type(_, r#type) => match type_head(r#type).as_deref() {
                Some("Result") => Ok(Self::Fallible),
                Some("Value") => Ok(Self::Infallible),
                _ => Err(syn::Error::new_spanned(
                    r#type,
                    "a built-in handler returns `Result<Value, Throw>`, `Value`, or nothing",
                )),
            },
        }
    }

    fn wrap(self, call: TokenStream) -> TokenStream {
        match self {
            Self::Fallible => call,
            Self::Infallible => quote!(::core::result::Result::Ok(#call)),
            Self::Void => quote! {{
                #call;
                ::core::result::Result::Ok(crate::value::Value::null())
            }},
        }
    }
}

fn type_head(r#type: &Type) -> Option<String> {
    match r#type {
        Type::Reference(reference) => type_head(&reference.elem),
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}
