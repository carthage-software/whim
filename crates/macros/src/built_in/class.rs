//! The signature-string `#[whim_class("Whim\\Result\\Ok<out T>", final)]` path.

use std::collections::HashSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::format_ident;
use quote::quote;
use syn::ItemStruct;
use syn::LitStr;
use whim_syn::arena;
use whim_syn::fragment;

use whim_syn::cst::atom::Modifier;

use crate::built_in::attribute_name;
use crate::built_in::attributes::AttributeArguments;
use crate::built_in::attributes::visibility_tokens;
use crate::built_in::base_spec;
use crate::built_in::class_constant_spec;
use crate::built_in::empty_method_provider;
use crate::built_in::generics;
use crate::built_in::split_name;
use crate::built_in::string_arguments;
use crate::built_in::type_spec;

pub(super) fn expand(attribute: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let arguments: AttributeArguments = syn::parse2(attribute)?;
    arguments.validate(
        1,
        &["final", "abstract", "readonly", "traced"],
        &["attribute"],
    )?;
    let declaration = arguments
        .positional_string(0)?
        .ok_or_else(|| syn::Error::new(Span::call_site(), "expected a class name"))?;
    let (name, generics_tail) = split_name(&declaration.value());
    let generics = generics::lower(&generics_tail)?;
    let name = LitStr::new(&name, declaration.span());

    let modifiers = Modifiers {
        is_final: arguments.has_flag("final"),
        is_abstract: arguments.has_flag("abstract"),
        is_readonly: arguments.has_flag("readonly"),
        traced: arguments.has_flag("traced"),
    };
    let attribute_flags = match arguments.value_expr("attribute") {
        Some(value) => quote!(::core::option::Option::Some(#value)),
        None => quote!(::core::option::Option::None),
    };

    let mut definition: ItemStruct = syn::parse2(item)?;
    let representation_mode = if definition.fields.is_empty() {
        RepresentationMode::None
    } else {
        RepresentationMode::BuiltIn
    };
    if matches!(representation_mode, RepresentationMode::None) {
        definition.attrs.push(syn::parse_quote!(#[allow(
            dead_code,
            reason = "fieldless declarations exist to generate built-in metadata"
        )]));
    }
    let representation = definition.ident.clone();

    let mut parent = quote!(::core::option::Option::None);
    let mut interfaces = Vec::new();
    let mut permits = Vec::new();
    let mut constants = Vec::new();
    let mut properties = Vec::new();
    let mut retained = Vec::new();
    for attribute in definition.attrs.drain(..) {
        match attribute_name(&attribute).as_deref() {
            Some("whim_extends") => {
                if let Some(first) = string_arguments(&attribute)?.first() {
                    let base = base_spec(first, &generics.names)?;
                    parent = quote!(::core::option::Option::Some(#base));
                }
            }
            Some("whim_implements") => {
                for entry in string_arguments(&attribute)? {
                    interfaces.push(base_spec(&entry, &generics.names)?);
                }
            }
            Some("whim_permits") => {
                permits.extend(string_arguments(&attribute)?);
            }
            Some("whim_class_like_constant") => {
                constants.push(class_constant_spec(
                    &attribute.parse_args::<AttributeArguments>()?,
                )?);
            }
            Some("whim_property") => {
                let source = attribute.parse_args::<LitStr>()?;
                properties.push(property_spec(
                    &source,
                    modifiers.is_readonly,
                    &generics.names,
                )?);
            }
            _ => retained.push(attribute),
        }
    }
    definition.attrs = retained;

    let items = emit(
        &representation,
        &name,
        &modifiers,
        representation_mode,
        &generics.spec,
        parent,
        &interfaces,
        &permits,
        &constants,
        &properties,
        attribute_flags,
    );

    Ok(quote! {
        #definition
        #items
    })
}

struct Modifiers {
    is_final: bool,
    is_abstract: bool,
    is_readonly: bool,
    traced: bool,
}

enum RepresentationMode {
    None,
    BuiltIn,
}

pub(super) fn property_spec(
    source: &LitStr,
    class_is_readonly: bool,
    names: &HashSet<String>,
) -> syn::Result<TokenStream> {
    let arena = arena::LocalArena::new();
    let property = fragment::parse_property(&arena, &source.value()).map_err(|error| {
        syn::Error::new(
            source.span(),
            format!("invalid property `{}`: {error}", source.value()),
        )
    })?;

    let name = property.variable.name.trim_start_matches('$');
    let visibility = property_visibility(property.modifiers)?;
    let is_readonly = class_is_readonly || property.modifiers.iter().any(Modifier::is_readonly);
    let type_spec = if let Some(subject) = property.r#type {
        let spec = type_spec::type_spec(subject, names)?;
        quote!(::core::option::Option::Some(#spec))
    } else {
        quote!(::core::option::Option::None)
    };

    Ok(quote! {
        crate::builtin::spec::PropertySpec {
            name: #name,
            visibility: #visibility,
            is_readonly: #is_readonly,
            type_spec: #type_spec,
        }
    })
}

fn property_visibility(modifiers: &[Modifier<'_>]) -> syn::Result<TokenStream> {
    let visibility = modifiers.iter().find_map(|modifier| match modifier {
        Modifier::Public(_) => Some("public"),
        Modifier::Protected(_) => Some("protected"),
        Modifier::Private(_) => Some("private"),
        _ => None,
    });

    visibility_tokens(visibility)
}

#[expect(
    clippy::too_many_arguments,
    reason = "emission keeps each parsed declaration component independently borrowed"
)]
fn emit(
    representation: &syn::Ident,
    name: &LitStr,
    modifiers: &Modifiers,
    representation_mode: RepresentationMode,
    type_parameters: &TokenStream,
    parent: TokenStream,
    interfaces: &[TokenStream],
    permits: &[String],
    constants: &[TokenStream],
    properties: &[TokenStream],
    attribute_flags: TokenStream,
) -> TokenStream {
    let Modifiers {
        is_final,
        is_abstract,
        is_readonly,
        traced,
    } = *modifiers;
    let sealed_to = if permits.is_empty() {
        quote!(::core::option::Option::None)
    } else {
        quote!(::core::option::Option::Some(&[#(#permits),*] as &[&str]))
    };

    let constructor = format_ident!("__whim_class_{representation}");
    let name_const = format_ident!("__whim_class_name_{representation}");
    let enqueue_fn = format_ident!("__whim_class_enqueue_{representation}");
    let visit_fn = format_ident!("__whim_class_visit_{representation}");
    let hooks_fn = format_ident!("__whim_class_hooks_{representation}");
    let initializer_fn = format_ident!("__whim_class_initializer_{representation}");
    let method_provider_type = format_ident!("__WhimMethods_{representation}");
    let method_provider_definition = empty_method_provider(representation);

    let (built_in_block, built_in_hooks_expr, built_in_initializer_expr) = match representation_mode
    {
        RepresentationMode::None => (
            quote!(),
            quote!(::core::option::Option::None),
            quote!(::core::option::Option::None),
        ),
        RepresentationMode::BuiltIn => {
            let new_fn = format_ident!("__whim_class_new_{representation}");
            let traced_hooks = if traced {
                quote! {
                    #[doc(hidden)]
                    unsafe fn #enqueue_fn(
                        __whim_data: ::core::ptr::NonNull<()>,
                        __whim_queue: &crate::value::heap::queue::DropQueue,
                        __whim_mode: crate::value::heap::metadata::TeardownMode,
                    ) {
                        let __whim_state = unsafe { &mut *__whim_data.cast::<#representation>().as_ptr() };
                        crate::builtin::convert::BuiltInChildren::enqueue_built_in_children(
                            __whim_state, __whim_queue, __whim_mode,
                        );
                    }

                    #[doc(hidden)]
                    unsafe fn #visit_fn(
                        __whim_data: ::core::ptr::NonNull<()>,
                        __whim_visitor: &mut crate::value::heap::metadata::TraceVisitor<'_>,
                    ) {
                        let __whim_state = unsafe { &*__whim_data.cast::<#representation>().as_ptr() };
                        crate::builtin::convert::BuiltInChildren::visit_built_in_children(
                            __whim_state, __whim_visitor,
                        );
                    }
                }
            } else {
                quote!()
            };
            let enqueue_children = if traced {
                quote!(::core::option::Option::Some(#enqueue_fn))
            } else {
                quote!(::core::option::Option::None)
            };
            let visit_children = if traced {
                quote!(::core::option::Option::Some(#visit_fn))
            } else {
                quote!(::core::option::Option::None)
            };
            let block = quote! {
                #[doc(hidden)]
                fn #new_fn(
                    __whim_vm: &mut crate::vm::VirtualMachine<'_>,
                ) -> ::core::result::Result<#representation, crate::builtin::throw::Throw> {
                    #representation::new(__whim_vm)
                }

                #traced_hooks

                #[doc(hidden)]
                fn #hooks_fn() -> &'static crate::value::object::BuiltInHooks {
                    &const {
                        crate::value::object::BuiltInHooks {
                            state_type: ::core::any::TypeId::of::<#representation>(),
                            layout: ::core::alloc::Layout::new::<#representation>(),
                            drop_in_place: crate::value::object::drop_built_in_state::<#representation>,
                            enqueue_children: #enqueue_children,
                            visit_children: #visit_children,
                        }
                    }
                }

                #[doc(hidden)]
                fn #initializer_fn(
                    __whim_vm: &mut crate::vm::VirtualMachine<'_>,
                    __whim_destination: ::core::ptr::NonNull<()>,
                ) -> ::core::result::Result<(), crate::builtin::throw::Throw> {
                    let __whim_state = #new_fn(__whim_vm)?;
                    unsafe {
                        __whim_destination.cast::<#representation>().as_ptr().write(__whim_state);
                    }
                    ::core::result::Result::Ok(())
                }
            };
            (
                block,
                quote!(::core::option::Option::Some(#hooks_fn())),
                quote!(::core::option::Option::Some(#initializer_fn)),
            )
        }
    };

    quote! {
        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        pub(crate) const #name_const: &'static str = #name;

        #built_in_block

        #method_provider_definition

        #[doc(hidden)]
        pub(crate) fn #constructor() -> crate::builtin::spec::ClassSpec {
            const INTERFACES: &[crate::builtin::spec::BaseSpec] = &[#(#interfaces),*];
            const CONSTANTS: &[crate::builtin::spec::ClassConstantSpec] = &[#(#constants),*];
            const PROPERTIES: &[crate::builtin::spec::PropertySpec] = &[#(#properties),*];

            crate::builtin::spec::ClassSpec {
                name: #name_const,
                type_parameters: #type_parameters,
                parent: #parent,
                interfaces: INTERFACES,
                is_final: #is_final,
                is_abstract: #is_abstract,
                is_readonly: #is_readonly,
                sealed_to: #sealed_to,
                constants: CONSTANTS,
                properties: PROPERTIES,
                methods: #method_provider_type.__whim_methods(),
                built_in_hooks: #built_in_hooks_expr,
                built_in_initializer: #built_in_initializer_expr,
                attribute_flags: #attribute_flags,
            }
        }
    }
}
