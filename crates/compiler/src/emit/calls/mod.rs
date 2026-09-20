//! Call sites: arguments, chains, and partial application.

use whim_bytecode::chunk::descriptors::PresetDescriptor;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::CallDescriptorIndex;
use whim_bytecode::instruction::operands::Count;
use whim_bytecode::instruction::operands::IcSlot;
use whim_bytecode::instruction::operands::Register;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::r#type::TypeArgumentList;
use whim_value::atom::Atom;

use crate::emit::Access;
use crate::emit::Argument;
use crate::emit::ArgumentList;
use crate::emit::ArrayAccess;
use crate::emit::BodyCompiler;
use crate::emit::Call;
use crate::emit::Callee;
use crate::emit::ClassReference;
use crate::emit::CompileError;
use crate::emit::CompileErrorKind;
use crate::emit::Expression;
use crate::emit::HasSpan;
use crate::emit::MethodCall;
use crate::emit::NullSafeMethodCall;
use crate::emit::NullSafePropertyAccess;
use crate::emit::PartialApplication;
use crate::emit::PartialArgument;
use crate::emit::PropertyAccess;
use crate::emit::Scope;
use crate::emit::Span;
use crate::emit::TypeScope;
use crate::emit::ValueUse;
use crate::emit::argument_gate;
use crate::emit::check_call_type_argument_arity;
use crate::emit::check_sequence;
use crate::emit::check_type_argument_arity;
use crate::types::ClassContext;
use crate::types::lowering::lower_type_argument;
use crate::types::lowering::reject_return_only_annotation;

mod callees;
mod chains;
mod invocations;

const fn call_value_instruction(
    value_use: ValueUse,
    argument_count: Count,
    destination: Register,
    callee: Register,
    first_argument: Register,
) -> Instruction {
    if value_use.discarded() {
        Instruction::CallValueDiscarded {
            argument_count,
            destination,
            callee,
            first_argument,
        }
    } else {
        Instruction::CallValue {
            argument_count,
            destination,
            callee,
            first_argument,
        }
    }
}

const fn call_named_instruction(
    value_use: ValueUse,
    argument_count: Count,
    destination: Register,
    first_argument: Register,
    cache: IcSlot,
) -> Instruction {
    if value_use.discarded() {
        Instruction::CallNamedDiscarded {
            argument_count,
            destination,
            first_argument,
            cache,
        }
    } else {
        Instruction::CallNamed {
            argument_count,
            destination,
            first_argument,
            cache,
        }
    }
}

const fn call_method_instruction(
    value_use: ValueUse,
    argument_count: Count,
    destination: Register,
    first_argument: Register,
    cache: IcSlot,
) -> Instruction {
    if value_use.discarded() {
        Instruction::CallMethodDiscarded {
            argument_count,
            destination,
            first_argument,
            cache,
        }
    } else {
        Instruction::CallMethod {
            argument_count,
            destination,
            first_argument,
            cache,
        }
    }
}

const fn call_static_instruction(
    value_use: ValueUse,
    argument_count: Count,
    destination: Register,
    first_argument: Register,
    cache: IcSlot,
) -> Instruction {
    if value_use.discarded() {
        Instruction::CallStaticDiscarded {
            argument_count,
            destination,
            first_argument,
            cache,
        }
    } else {
        Instruction::CallStatic {
            argument_count,
            destination,
            first_argument,
            cache,
        }
    }
}

const fn call_with_names_instruction(
    value_use: ValueUse,
    destination: Register,
    callee: Register,
    descriptor: CallDescriptorIndex,
) -> Instruction {
    if value_use.discarded() {
        Instruction::CallWithNamesDiscarded {
            destination,
            callee,
            descriptor,
        }
    } else {
        Instruction::CallWithNames {
            destination,
            callee,
            descriptor,
        }
    }
}

/// The callee of a descriptor-driven call, before its value is built.
enum CalleeSource<'source, 'arena> {
    Function(&'source Callee<'arena>),
    Method {
        receiver: Register,
        name: &'source str,
    },
    Static {
        class: &'source ClassReference<'arena>,
        name: &'source str,
    },
    Value(Register),
}

fn check_named_arguments(argument_list: &ArgumentList<'_>) -> Result<(), CompileError> {
    let mut seen: Vec<&str> = Vec::new();
    let mut has_named = false;
    for argument in &argument_list.arguments {
        match argument {
            Argument::Positional(positional) => {
                if has_named {
                    return Err(CompileError::new(
                        CompileErrorKind::PositionalArgumentAfterNamedArgument,
                        "a positional argument cannot follow a named argument",
                        positional.span(),
                    ));
                }
            }
            Argument::Named(named) => {
                has_named = true;
                if seen.contains(&named.name.value) {
                    return Err(CompileError::new(
                        CompileErrorKind::DuplicateNamedArgument,
                        format!("the named argument `{}` is passed twice", named.name.value),
                        named.span(),
                    ));
                }
                seen.push(named.name.value);
            }
        }
    }

    Ok(())
}

fn require_class_context<'compilation>(
    scope: &Scope<'compilation>,
    reference: &str,
    span: Span,
) -> Result<&'compilation ClassContext, CompileError> {
    scope.class.ok_or_else(|| {
        CompileError::new(
            CompileErrorKind::ClassContextRequired,
            format!("`{reference}::` is available only inside a class"),
            span,
        )
    })
}

impl BodyCompiler<'_, '_> {
    fn lower_turbofish(
        &self,
        scope: &Scope<'_>,
        arguments: Option<&TypeArgumentList<'_>>,
    ) -> Result<Option<Vec<TypeDescriptor>>, CompileError> {
        let type_scope = TypeScope {
            heap: self.heap,
            resolver: scope.resolver,
            class: scope.class,
            aliases: self.aliases,
            binders: &scope.binders,
            forbidden_binders: &scope.forbidden_binders,
            generics: scope.generics,
        };
        arguments
            .map(|arguments| {
                arguments
                    .arguments
                    .iter()
                    .map(|argument| {
                        reject_return_only_annotation(argument.r#type, "type argument")?;
                        lower_type_argument(&type_scope, argument.r#type)
                    })
                    .collect()
            })
            .transpose()
    }

    fn specialize_callee(
        &mut self,
        scope: &Scope<'_>,
        callee: Register,
        arguments: Option<&TypeArgumentList<'_>>,
        span: Span,
    ) -> Result<Register, CompileError> {
        let Some(type_arguments) = self.lower_turbofish(scope, arguments)? else {
            return Ok(callee);
        };

        let destination = self.allocate(span)?;
        let descriptor = self.add_preset_descriptor(
            PresetDescriptor {
                slots: Vec::new(),
                open_remaining: false,
                type_arguments: Some(type_arguments),
            },
            span,
        )?;

        self.chunk.emit(
            Instruction::MakeBound {
                destination,
                callee,
                descriptor,
            },
            span,
        );

        Ok(destination)
    }

    /// Checks type arguments on a locally known function.
    fn check_callee_turbofish(
        scope: &Scope<'_>,
        callee: &Callee<'_>,
        type_arguments: Option<&TypeArgumentList<'_>>,
    ) -> Result<(), CompileError> {
        let Callee::Identifier(identifier) = callee else {
            return Ok(());
        };

        let resolved = scope.resolver.resolve_text(identifier);
        check_call_type_argument_arity(scope.generics, &resolved, type_arguments, callee.span())
    }

    /// Checks type arguments on a locally known static method.
    fn check_method_turbofish(
        scope: &Scope<'_>,
        class: &ClassReference<'_>,
        method: &str,
        type_arguments: Option<&TypeArgumentList<'_>>,
    ) -> Result<(), CompileError> {
        let class_name = match class {
            ClassReference::Named(named) => scope.resolver.resolve_text(&named.identifier),
            ClassReference::Self_(_) => match scope.class {
                Some(context) => context.name.clone(),
                None => return Ok(()),
            },
            _ => return Ok(()),
        };

        check_call_type_argument_arity(
            scope.generics,
            &format!("{class_name}::{method}"),
            type_arguments,
            class.span(),
        )
    }

    fn class_reference_binder(
        scope: &Scope<'_>,
        reference: &ClassReference<'_>,
    ) -> Result<Option<String>, CompileError> {
        let ClassReference::Named(named) = reference else {
            return Ok(None);
        };
        let Identifier::Local(local) = &named.identifier else {
            return Ok(None);
        };

        if scope.binders.iter().any(|binder| binder == local.value) {
            if let Some(arguments) = &named.type_arguments {
                return Err(CompileError::new(
                    CompileErrorKind::TypeArgumentArityMismatch,
                    format!(
                        "the type parameter `{}` is not generic and takes no type arguments",
                        local.value
                    ),
                    arguments.span(),
                ));
            }

            return Ok(Some(local.value.to_string()));
        }

        if scope
            .forbidden_binders
            .iter()
            .any(|binder| binder == local.value)
        {
            return Err(CompileError::new(
                CompileErrorKind::ClassTypeParameterInStaticMember,
                format!(
                    "the class type parameter `{}` is unavailable in a static member",
                    local.value
                ),
                named.span(),
            ));
        }

        Ok(None)
    }

    pub(crate) fn static_call_class_atom(
        &self,
        scope: &Scope<'_>,
        reference: &ClassReference<'_>,
    ) -> Result<Atom, CompileError> {
        if let Some(binder) = Self::class_reference_binder(scope, reference)? {
            return Ok(self.heap.intern(format!("@{binder}").as_bytes()));
        }

        self.class_reference_atom(scope, reference)
    }

    pub(crate) fn class_reference_atom(
        &self,
        scope: &Scope<'_>,
        reference: &ClassReference<'_>,
    ) -> Result<Atom, CompileError> {
        if let Some(binder) = Self::class_reference_binder(scope, reference)? {
            return Err(CompileError::new(
                CompileErrorKind::TypeParameterClassReference,
                format!(
                    "the type parameter `{binder}` supports only static method calls in class position"
                ),
                reference.span(),
            ));
        }

        match reference {
            ClassReference::Named(named) => {
                let resolved = scope.resolver.resolve_text(&named.identifier);
                check_type_argument_arity(
                    scope.generics,
                    &resolved,
                    named.type_arguments.as_ref(),
                )?;

                Ok(scope.resolver.resolve(self.heap, &named.identifier))
            }
            ClassReference::Self_(keyword) => {
                let class = require_class_context(scope, "self", keyword.span())?;
                Ok(self.heap.intern(class.name.as_bytes()))
            }
            ClassReference::Parent(keyword) => {
                let class = require_class_context(scope, "parent", keyword.span())?;
                Ok(self
                    .heap
                    .intern(class.parent.as_deref().unwrap_or("parent").as_bytes()))
            }
            ClassReference::Static(keyword) => {
                require_class_context(scope, "static", keyword.span())?;
                Ok(self.heap.intern(b"static"))
            }
            ClassReference::Expression(expression) => Err(CompileError::new(
                CompileErrorKind::DynamicClassMemberAccess,
                "a static property or class constant requires a named class",
                expression.span(),
            )),
        }
    }
}
