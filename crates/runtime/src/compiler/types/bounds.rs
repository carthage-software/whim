//! Conservative same-unit validation of generic construction sites.

use hashbrown::HashMap;
use whim_span::Span;

use crate::bytecode::aliases::TypeAliasIndex;
use crate::bytecode::aliases::expand_aliases_using;
use crate::bytecode::aliases::substitute;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::limits::MAX_TYPE_DEPTH;
use crate::optimizer::descriptor_proves;
use crate::optimizer::descriptors_equal;
use crate::value::atom::Atom;

struct UnitIndexes<'unit> {
    classes: HashMap<Atom, &'unit CompiledClassLike>,
    aliases: TypeAliasIndex<'unit>,
}

impl<'unit> UnitIndexes<'unit> {
    fn new(unit: &'unit CompiledUnit) -> Self {
        Self {
            classes: unit
                .classes
                .iter()
                .map(|class| (class.name.clone(), class))
                .collect(),
            aliases: TypeAliasIndex::new(&unit.type_aliases),
        }
    }

    fn class(&self, name: &Atom) -> Option<&'unit CompiledClassLike> {
        self.classes.get(name).copied()
    }
}

pub(in crate::compiler) fn validate_static_type_argument_bounds(
    unit: &CompiledUnit,
) -> Result<(), CompileError> {
    let indexes = UnitIndexes::new(unit);
    validate_chunk(&indexes, &unit.main)?;
    for function in &unit.functions {
        validate_chunk(&indexes, &function.chunk)?;
    }

    for class in &unit.classes {
        for method in &class.methods {
            validate_chunk(&indexes, &method.function.chunk)?;
        }
    }

    Ok(())
}

fn validate_chunk(indexes: &UnitIndexes<'_>, chunk: &Chunk) -> Result<(), CompileError> {
    for (index, instruction) in chunk.code.iter().enumerate() {
        let Instruction::NewStatic { cache, .. } = instruction else {
            continue;
        };

        let Some(IcDescriptor::Member {
            name,
            type_arguments: Some(arguments),
        }) = chunk.ic_descriptors.get(usize::from(cache.index()))
        else {
            continue;
        };

        let Some(class) = indexes.class(name) else {
            continue;
        };

        validate_arguments(
            indexes,
            &class.name,
            &class.type_parameters,
            arguments,
            chunk.spans[index],
        )?;
    }

    Ok(())
}

fn validate_arguments(
    indexes: &UnitIndexes<'_>,
    subject: &Atom,
    parameters: &[CompiledTypeParameter],
    arguments: &[TypeDescriptor],
    span: Span,
) -> Result<(), CompileError> {
    if arguments.len() > parameters.len() {
        return Ok(());
    }

    let mut bindings = Vec::with_capacity(parameters.len());
    for (position, parameter) in parameters.iter().enumerate() {
        let argument = if let Some(argument) = arguments.get(position) {
            argument.clone()
        } else {
            let Some(default) = &parameter.default else {
                return Ok(());
            };
            substitute(default, &bindings, 0)
        };

        let argument = expand_aliases_using(&argument, &indexes.aliases);
        bindings.push((parameter.name.clone(), argument));
    }

    for (position, (parameter, (_, argument))) in parameters.iter().zip(&bindings).enumerate() {
        for bound in &parameter.bounds {
            let bound = expand_aliases_using(&substitute(bound, &bindings, 0), &indexes.aliases);
            if statically_is_subtype(indexes, argument, &bound, 0) != Some(false) {
                continue;
            }

            return Err(CompileError::new(
                CompileErrorKind::TypeArgumentBoundViolation,
                format!(
                    "type argument {} supplied to `{}` does not satisfy the bound of `{}`",
                    position + 1,
                    subject.to_string_lossy(),
                    parameter.name.to_string_lossy(),
                ),
                span,
            ));
        }
    }

    Ok(())
}

fn statically_is_subtype(
    indexes: &UnitIndexes<'_>,
    actual: &TypeDescriptor,
    expected: &TypeDescriptor,
    depth: usize,
) -> Option<bool> {
    if depth > MAX_TYPE_DEPTH {
        return None;
    }

    if descriptor_proves(actual, expected, None, depth + 1) {
        return Some(true);
    }

    if let TypeDescriptor::Union(members) = actual {
        return all_relations(
            members
                .iter()
                .map(|member| statically_is_subtype(indexes, member, expected, depth + 1)),
        );
    }

    if let TypeDescriptor::Union(members) = expected {
        return any_relation(
            members
                .iter()
                .map(|member| statically_is_subtype(indexes, actual, member, depth + 1)),
        );
    }

    if let TypeDescriptor::Intersection(members) = expected {
        return all_relations(
            members
                .iter()
                .map(|member| statically_is_subtype(indexes, actual, member, depth + 1)),
        );
    }

    if let TypeDescriptor::Intersection(members) = actual {
        return any_relation(
            members
                .iter()
                .map(|member| statically_is_subtype(indexes, member, expected, depth + 1)),
        );
    }

    match (actual, expected) {
        (
            TypeDescriptor::Named {
                name: actual_name,
                arguments: actual_arguments,
                ..
            },
            TypeDescriptor::Named {
                name: expected_name,
                arguments: expected_arguments,
                ..
            },
        ) => nominal_is_subtype(
            indexes,
            actual_name,
            actual_arguments.as_deref(),
            expected_name,
            expected_arguments.as_deref(),
            depth + 1,
        ),
        (TypeDescriptor::Named { name, .. }, TypeDescriptor::Object) => {
            indexes.class(name).map(|_| true)
        }
        (TypeDescriptor::Parameter(_) | TypeDescriptor::StaticClass, _)
        | (_, TypeDescriptor::Parameter(_) | TypeDescriptor::StaticClass) => None,
        (TypeDescriptor::Named { .. }, _) => Some(false),
        (_, TypeDescriptor::Named { name, .. }) => indexes.class(name).map(|_| false),
        _ if closed_scalar(actual) && closed_scalar(expected) => Some(false),
        _ => None,
    }
}

fn nominal_is_subtype(
    indexes: &UnitIndexes<'_>,
    actual_name: &Atom,
    actual_arguments: Option<&[TypeDescriptor]>,
    expected_name: &Atom,
    expected_arguments: Option<&[TypeDescriptor]>,
    depth: usize,
) -> Option<bool> {
    if depth > MAX_TYPE_DEPTH {
        return None;
    }
    if actual_name == expected_name {
        return match (actual_arguments, expected_arguments) {
            (_, None) => Some(true),
            (Some(actual), Some(expected))
                if actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(actual, expected)| {
                        descriptors_equal(actual, expected, depth + 1)
                    }) =>
            {
                Some(true)
            }
            _ => None,
        };
    }

    indexes.class(expected_name)?;
    let actual = indexes.class(actual_name)?;
    let mut unresolved = false;
    for base in actual.parent.iter().chain(actual.interfaces.iter()) {
        match nominal_is_subtype(
            indexes,
            &base.name,
            base.type_arguments.as_deref(),
            expected_name,
            expected_arguments,
            depth + 1,
        ) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unresolved = true,
        }
    }

    if unresolved { None } else { Some(false) }
}

fn all_relations(relations: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let mut unknown = false;
    for relation in relations {
        match relation {
            Some(true) => {}
            Some(false) => return Some(false),
            None => unknown = true,
        }
    }

    if unknown { None } else { Some(true) }
}

fn any_relation(relations: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let mut unknown = false;
    for relation in relations {
        match relation {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }

    if unknown { None } else { Some(false) }
}

const fn closed_scalar(descriptor: &TypeDescriptor) -> bool {
    matches!(
        descriptor,
        TypeDescriptor::Void
            | TypeDescriptor::Never
            | TypeDescriptor::Null
            | TypeDescriptor::Bool
            | TypeDescriptor::Int
            | TypeDescriptor::Float
            | TypeDescriptor::String
            | TypeDescriptor::TrueLiteral
            | TypeDescriptor::FalseLiteral
            | TypeDescriptor::IntLiteral(_)
            | TypeDescriptor::IntRange { .. }
            | TypeDescriptor::FloatLiteral(_)
            | TypeDescriptor::StringLiteral(_)
    )
}
