//! Declared-variance checks over compiled type descriptors.

#![deny(clippy::nursery, clippy::pedantic)]

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::Variance;
use crate::value::atom::Atom;

#[expect(
    clippy::redundant_pub_crate,
    reason = "the compiler and linker share descriptor variance checks"
)]
pub(crate) fn incompatible_parameter<'parameter>(
    descriptor: &TypeDescriptor,
    polarity: i8,
    parameters: &'parameter [CompiledTypeParameter],
    mut named_variance: impl FnMut(&Atom, usize) -> Option<Variance>,
) -> Option<&'parameter CompiledTypeParameter> {
    let mut pending = vec![(descriptor, polarity)];
    while let Some((current, position)) = pending.pop() {
        match current {
            TypeDescriptor::Parameter(name) => {
                let Some(parameter) = parameters.iter().find(|parameter| parameter.name == *name)
                else {
                    continue;
                };
                let valid = match parameter.variance {
                    Variance::Invariant => true,
                    Variance::Covariant => position == 1,
                    Variance::Contravariant => position == -1,
                };
                if !valid {
                    return Some(parameter);
                }
            }
            TypeDescriptor::Named {
                name,
                arguments: Some(arguments),
                ..
            } => {
                for (index, argument) in arguments.iter().enumerate() {
                    let Some(variance) = named_variance(name, index) else {
                        continue;
                    };
                    pending.push((argument, nested_polarity(position, variance)));
                }
            }
            TypeDescriptor::Array(Some((key, value)))
            | TypeDescriptor::Dictionary(Some((key, value))) => {
                pending.push((key, position));
                pending.push((value, position));
            }
            TypeDescriptor::Vector(Some(element)) => pending.push((element, position)),
            TypeDescriptor::VectorShape { elements, rest } => {
                pending.extend(elements.iter().map(|element| (element, position)));
                if let Some(rest) = rest {
                    pending.push((rest, position));
                }
            }
            TypeDescriptor::DictionaryShape { entries, rest } => {
                pending.extend(entries.iter().map(|(_, value)| (value, position)));
                if let Some((key, value)) = rest {
                    pending.push((key, position));
                    pending.push((value, position));
                }
            }
            TypeDescriptor::Callable(Some(signature)) => {
                pending.push((&signature.return_type, position));
                pending.extend(
                    signature
                        .parameters
                        .iter()
                        .map(|parameter| (&parameter.r#type, -position)),
                );
            }
            TypeDescriptor::Classname(inner) => pending.push((inner, position)),
            TypeDescriptor::Negated(inner) => pending.push((inner, -position)),
            TypeDescriptor::TupleRest { elements, rest } => {
                pending.extend(elements.iter().map(|element| (element, position)));
                pending.push((rest, position));
            }
            TypeDescriptor::Tuple(members)
            | TypeDescriptor::Union(members)
            | TypeDescriptor::Intersection(members) => {
                pending.extend(members.iter().map(|member| (member, position)));
            }
            _ => {}
        }
    }

    None
}

const fn nested_polarity(polarity: i8, variance: Variance) -> i8 {
    match variance {
        Variance::Invariant => 0,
        Variance::Covariant => polarity,
        Variance::Contravariant => -polarity,
    }
}
