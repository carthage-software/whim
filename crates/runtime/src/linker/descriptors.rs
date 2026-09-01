//! Symbolic descriptor substitution and built-in type-spec conversion.

use hashbrown::HashMap;

use crate::builtin::spec::TypeSpec;
use crate::bytecode::chunk::descriptors::FunctionTypeDescriptor;
use crate::bytecode::chunk::descriptors::FunctionTypeParameterDescriptor;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

pub(in crate::linker) fn substitute_symbolic(
    descriptor: &TypeDescriptor,
    environment: &HashMap<Atom, TypeDescriptor>,
) -> TypeDescriptor {
    match descriptor {
        TypeDescriptor::Parameter(name) => environment
            .get(name)
            .cloned()
            .unwrap_or_else(|| descriptor.clone()),
        _ => descriptor.map_children(|child| substitute_symbolic(child, environment)),
    }
}

pub(crate) fn descriptor_from_built_in_spec(heap: &Heap, spec: &TypeSpec) -> TypeDescriptor {
    match spec {
        TypeSpec::Wildcard => TypeDescriptor::Wildcard,
        TypeSpec::Mixed => TypeDescriptor::Mixed,
        TypeSpec::Void => TypeDescriptor::Void,
        TypeSpec::Never => TypeDescriptor::Never,
        TypeSpec::Null => TypeDescriptor::Null,
        TypeSpec::Bool => TypeDescriptor::Bool,
        TypeSpec::Int => TypeDescriptor::Int,
        TypeSpec::IntRange(min, max) => TypeDescriptor::integer_range(*min, *max),
        TypeSpec::Float => TypeDescriptor::Float,
        TypeSpec::String => TypeDescriptor::String,
        TypeSpec::StringLiteral(value) => TypeDescriptor::StringLiteral(heap.intern(value)),
        TypeSpec::Array => TypeDescriptor::Array(None),
        TypeSpec::Vec => TypeDescriptor::Vector(None),
        TypeSpec::Dict => TypeDescriptor::Dictionary(None),
        TypeSpec::Tuple => TypeDescriptor::TupleAny,
        TypeSpec::Function => TypeDescriptor::Callable(None),
        TypeSpec::Object => TypeDescriptor::Object,
        TypeSpec::Static => TypeDescriptor::StaticClass,
        TypeSpec::Instance(name) => TypeDescriptor::Named {
            name: heap.intern(name.as_bytes()),
            arguments: None,
            recursive: false,
        },
        TypeSpec::Parameter(name) => TypeDescriptor::Parameter(heap.intern(name.as_bytes())),
        TypeSpec::GenericInstance(name, arguments) => TypeDescriptor::Named {
            name: heap.intern(name.as_bytes()),
            arguments: Some(
                arguments
                    .iter()
                    .map(|argument| descriptor_from_built_in_spec(heap, argument))
                    .collect(),
            ),
            recursive: false,
        },
        TypeSpec::TupleOf(members) => TypeDescriptor::Tuple(
            members
                .iter()
                .map(|member| descriptor_from_built_in_spec(heap, member))
                .collect(),
        ),
        TypeSpec::TupleRest(elements, rest) => TypeDescriptor::TupleRest {
            elements: elements
                .iter()
                .map(|element| descriptor_from_built_in_spec(heap, element))
                .collect(),
            rest: Box::new(descriptor_from_built_in_spec(heap, rest)),
        },
        TypeSpec::Optional(inner) => TypeDescriptor::Union(vec![
            TypeDescriptor::Null,
            descriptor_from_built_in_spec(heap, inner),
        ]),
        TypeSpec::VectorOf(element) => {
            TypeDescriptor::Vector(Some(Box::new(descriptor_from_built_in_spec(heap, element))))
        }
        TypeSpec::ArrayOf(key, value) => TypeDescriptor::Array(Some((
            Box::new(descriptor_from_built_in_spec(heap, key)),
            Box::new(descriptor_from_built_in_spec(heap, value)),
        ))),
        TypeSpec::DictionaryOf(key, value) => TypeDescriptor::Dictionary(Some((
            Box::new(descriptor_from_built_in_spec(heap, key)),
            Box::new(descriptor_from_built_in_spec(heap, value)),
        ))),
        TypeSpec::CallableOf(parameters, return_type) => {
            TypeDescriptor::Callable(Some(FunctionTypeDescriptor {
                parameters: parameters
                    .iter()
                    .map(|parameter| FunctionTypeParameterDescriptor {
                        r#type: descriptor_from_built_in_spec(heap, &parameter.type_spec),
                        optional: parameter.optional,
                    })
                    .collect(),
                return_type: Box::new(descriptor_from_built_in_spec(heap, return_type)),
            }))
        }
        TypeSpec::Classname(inner) => {
            TypeDescriptor::Classname(Box::new(descriptor_from_built_in_spec(heap, inner)))
        }
        TypeSpec::Union(members) => TypeDescriptor::Union(
            members
                .iter()
                .map(|member| descriptor_from_built_in_spec(heap, member))
                .collect(),
        ),
        TypeSpec::Intersection(members) => TypeDescriptor::Intersection(
            members
                .iter()
                .map(|member| descriptor_from_built_in_spec(heap, member))
                .collect(),
        ),
        TypeSpec::Negated(inner) => {
            TypeDescriptor::Negated(Box::new(descriptor_from_built_in_spec(heap, inner)))
        }
    }
}
