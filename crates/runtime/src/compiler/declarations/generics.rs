//! Generic-declaration collection and type-parameter compilation.

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::Program;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::node::Node;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeParameterList;
use whim_syn::cst::r#type::TypeVariance;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::Variance;
use crate::compiler::declarations::namespace_name;
use crate::compiler::emit::Scope;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::limits::check_sequence;
use crate::compiler::types;
use crate::compiler::types::AliasExpansion;
use crate::compiler::types::DeclaredTypeKind;
use crate::compiler::types::GenericDecl;
use crate::compiler::types::GenericTable;
use crate::compiler::types::TypeScope;
use crate::compiler::types::rendering::binder_arity;
use crate::compiler::types::rendering::render_annotation;
use crate::value::heap::Heap;

/// Records each local generic arity before types are lowered. An arity of zero
/// rejects type arguments on local non-generic names.
pub(in crate::compiler) fn collect_generics<'arena>(
    program: &Program<'arena>,
) -> GenericTable<'arena> {
    let mut table = GenericTable::new();
    extend_generics(program, &mut table);
    table
}

pub(crate) fn extend_generics<'arena>(program: &Program<'arena>, table: &mut GenericTable<'arena>) {
    collect_generic_statements(program.statements, "", table);
}

fn collect_generic_statements<'arena>(
    statements: &'arena [Statement<'arena>],
    namespace: &str,
    table: &mut GenericTable<'arena>,
) {
    for statement in statements {
        match statement {
            Statement::Namespace(namespace_declaration) => {
                collect_generic_statements(
                    namespace_declaration.statements(),
                    &namespace_name(namespace_declaration),
                    table,
                );
            }
            Statement::Function(function) => {
                record_declaration(
                    table,
                    namespace,
                    function.name.value,
                    function.type_parameters.as_ref(),
                    DeclaredTypeKind::Function,
                );
            }
            Statement::Constant(constant) => {
                record_declaration(
                    table,
                    namespace,
                    constant.name.value,
                    None,
                    DeclaredTypeKind::Constant,
                );
            }
            Statement::Class(class) => {
                record_declaration(
                    table,
                    namespace,
                    class.name.value,
                    class.type_parameters.as_ref(),
                    DeclaredTypeKind::ClassLike,
                );

                record_member_arities(table, namespace, class.name.value, class.members);
            }
            Statement::Interface(interface) => {
                record_declaration(
                    table,
                    namespace,
                    interface.name.value,
                    interface.type_parameters.as_ref(),
                    DeclaredTypeKind::ClassLike,
                );

                record_member_arities(table, namespace, interface.name.value, interface.members);
            }
            Statement::Enum(declaration) => {
                record_declaration(
                    table,
                    namespace,
                    declaration.name.value,
                    None,
                    DeclaredTypeKind::ClassLike,
                );
                record_member_arities(
                    table,
                    namespace,
                    declaration.name.value,
                    declaration.members,
                );
            }
            Statement::TypeAlias(alias) => {
                let (required, total) = alias.type_parameters.as_ref().map_or((0, 0), binder_arity);
                let expansion = AliasExpansion {
                    type_parameters: alias.type_parameters.as_ref(),
                    aliased: alias.aliased,
                };

                table.insert(
                    qualify_in(namespace, alias.name.value),
                    GenericDecl {
                        kind: DeclaredTypeKind::TypeAlias,
                        required,
                        total,
                        variances: declaration_variances(alias.type_parameters.as_ref()),
                        is_alias: true,
                        is_callable: false,
                        alias: Some(expansion),
                    },
                );
            }
            Statement::Newtype(newtype) => {
                record_declaration(
                    table,
                    namespace,
                    newtype.name.value,
                    newtype.type_parameters.as_ref(),
                    DeclaredTypeKind::Newtype,
                );
            }
            _ => {}
        }
    }
}

fn record_declaration<'arena>(
    table: &mut GenericTable<'arena>,
    namespace: &str,
    name: &str,
    type_parameters: Option<&'arena TypeParameterList<'arena>>,
    kind: DeclaredTypeKind,
) {
    let (required, total) = type_parameters.map_or((0, 0), |list| binder_arity(list));
    table.insert(
        qualify_in(namespace, name),
        GenericDecl {
            kind,
            required,
            total,
            variances: declaration_variances(type_parameters),
            is_alias: false,
            is_callable: kind == DeclaredTypeKind::Function,
            alias: None,
        },
    );
}

/// Records each method's own type-argument arity under `Class::method`, so a
/// turbofish on a statically named method is checked against the same table a
/// turbofish on a function is.
fn record_member_arities<'arena>(
    table: &mut GenericTable<'arena>,
    namespace: &str,
    class_name: &str,
    members: &'arena [ClassLikeMember<'arena>],
) {
    for member in members {
        let (name, type_parameters, kind) = match member {
            ClassLikeMember::Method(method) => (
                method.name.value,
                method.type_parameters.as_ref(),
                DeclaredTypeKind::Method,
            ),
            ClassLikeMember::Constant(constant) => {
                (constant.name.value, None, DeclaredTypeKind::ClassConstant)
            }
            ClassLikeMember::EnumCase(case) => (case.name.value, None, DeclaredTypeKind::EnumCase),
            ClassLikeMember::Property(_) => continue,
        };
        let (required, total) = type_parameters.map_or((0, 0), |list| binder_arity(list));

        table.insert(
            format!("{}::{name}", qualify_in(namespace, class_name)),
            GenericDecl {
                kind,
                required,
                total,
                variances: declaration_variances(type_parameters),
                is_alias: false,
                is_callable: kind == DeclaredTypeKind::Method,
                alias: None,
            },
        );
    }
}

fn declaration_variances(parameters: Option<&TypeParameterList<'_>>) -> Vec<Variance> {
    parameters.map_or_else(Vec::new, |parameters| {
        parameters
            .parameters
            .iter()
            .map(|parameter| match parameter.variance {
                Some(TypeVariance::In(_)) => Variance::Contravariant,
                Some(TypeVariance::Out(_)) => Variance::Covariant,
                None => Variance::Invariant,
            })
            .collect()
    })
}

fn qualify_in(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}\\{name}")
    }
}

/// The binder names a type-parameter list introduces, in declaration order.
/// Empty when the declaration is not generic.
pub(in crate::compiler) fn binder_names(
    type_parameters: Option<&TypeParameterList<'_>>,
) -> Vec<String> {
    type_parameters
        .map(|list| {
            list.parameters
                .iter()
                .map(|parameter| parameter.name.value.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The binder names in scope inside a type-parameter list's own bounds and
/// defaults: the enclosing declaration's binders together with the list's.
fn binders_within(scope: &Scope<'_>, list: &TypeParameterList<'_>) -> Vec<String> {
    let mut binders = scope.binders.clone();
    for name in binder_names(Some(list)) {
        if !binders.contains(&name) {
            binders.push(name);
        }
    }

    binders
}

struct UnboundDefaultReference<'context> {
    unavailable: &'context [String],
    found: Option<(String, Span)>,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for UnboundDefaultReference<'_> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        if self.found.is_some() {
            return Flow::Skip;
        }
        if let Node::NamedType(named) = node
            && let Identifier::Local(local) = &named.identifier
            && self
                .unavailable
                .iter()
                .any(|parameter| parameter == local.value)
        {
            self.found = Some((local.value.to_string(), named.span()));
            return Flow::Skip;
        }

        Flow::Descend
    }
}

/// Finds a reference to a parameter that has no concrete binding when a
/// default is evaluated: the parameter declaring the default or one after it.
fn unbound_default_reference(source: &Type<'_>, unavailable: &[String]) -> Option<(String, Span)> {
    let mut visitor = UnboundDefaultReference {
        unavailable,
        found: None,
    };
    walk(Node::Type(source), &mut visitor);
    visitor.found
}

pub(in crate::compiler) fn check_type_parameters(
    heap: &Heap,
    scope: &Scope<'_>,
    aliases: &[CompiledTypeAlias],
    list: &TypeParameterList<'_>,
) -> Result<(), CompileError> {
    check_sequence(
        CompileErrorKind::TooManyTypeParameters,
        "a declaration may bind",
        "type parameters",
        list.parameters,
    )?;

    let mut names = Vec::with_capacity(list.parameters.len());
    for parameter in list.parameters {
        if names.contains(&parameter.name.value) {
            return Err(CompileError::new(
                CompileErrorKind::DuplicateTypeParameter,
                format!(
                    "the type parameter `{}` is declared more than once in the same parameter list",
                    parameter.name.value
                ),
                parameter.name.span(),
            ));
        }
        names.push(parameter.name.value);
    }

    let binders = binders_within(scope, list);
    let type_scope = TypeScope {
        heap,
        resolver: scope.resolver,
        class: scope.class,
        aliases,
        binders: &binders,
        forbidden_binders: &scope.forbidden_binders,
        generics: scope.generics,
    };

    let mut default_seen = false;
    let own_binders = binder_names(Some(list));
    for (index, parameter) in list.parameters.iter().enumerate() {
        if parameter.default.is_some() {
            default_seen = true;
        } else if default_seen {
            return Err(CompileError::new(
                CompileErrorKind::NonTrailingTypeParameterDefault,
                format!(
                    "the type parameter `{}` has no default, but a preceding one does; only \
                     trailing type parameters may declare a default",
                    parameter.name.value
                ),
                parameter.span(),
            ));
        }

        if let Some(bound) = &parameter.bound {
            check_sequence(
                CompileErrorKind::TooManyTypeCompositionMembers,
                "a type parameter bound may name",
                "types",
                bound.types,
            )?;

            for r#type in bound.types {
                render_annotation(&type_scope, r#type)?;
            }
        }

        if let Some(default) = &parameter.default {
            if let Some((referenced, span)) =
                unbound_default_reference(default.r#type, &own_binders[index..])
            {
                return Err(CompileError::new(
                    CompileErrorKind::UnboundTypeParameterDefault,
                    format!(
                        "the default for type parameter `{}` references `{referenced}` before it \
                         has a concrete binding; a default may reference only enclosing and \
                         preceding type parameters",
                        parameter.name.value
                    ),
                    span,
                ));
            }
            render_annotation(&type_scope, default.r#type)?;
        }
    }

    Ok(())
}

pub(in crate::compiler) fn compile_type_parameters(
    heap: &Heap,
    scope: &Scope<'_>,
    aliases: &[CompiledTypeAlias],
    type_parameters: Option<&TypeParameterList<'_>>,
) -> Result<Vec<CompiledTypeParameter>, CompileError> {
    let Some(list) = type_parameters else {
        return Ok(Vec::new());
    };

    check_type_parameters(heap, scope, aliases, list)?;
    let binders = binders_within(scope, list);
    let type_scope = TypeScope {
        heap,
        resolver: scope.resolver,
        class: scope.class,
        aliases,
        binders: &binders,
        forbidden_binders: &scope.forbidden_binders,
        generics: scope.generics,
    };

    let mut result = Vec::new();
    for parameter in list.parameters {
        let variance = match parameter.variance {
            Some(TypeVariance::In(_)) => Variance::Contravariant,
            Some(TypeVariance::Out(_)) => Variance::Covariant,
            None => Variance::Invariant,
        };

        let mut bounds = Vec::new();
        if let Some(bound) = &parameter.bound {
            for r#type in bound.types {
                types::lowering::reject_return_only_annotation(r#type, "type-parameter bound")?;
                bounds.push(types::lowering::lower_type_argument(&type_scope, r#type)?);
            }
        }

        let default = match &parameter.default {
            Some(default) => {
                types::lowering::reject_return_only_annotation(
                    default.r#type,
                    "type-parameter default",
                )?;
                Some(types::lowering::lower_type_argument(
                    &type_scope,
                    default.r#type,
                )?)
            }
            None => None,
        };

        result.push(CompiledTypeParameter {
            name: heap.intern(parameter.name.value.as_bytes()),
            span: parameter.span(),
            variance,
            bounds,
            default,
        });
    }

    Ok(result)
}
