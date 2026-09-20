//! Structural expansion of compiled type aliases.

use std::rc::Rc;

use hashbrown::HashMap;
use whim_base::limits::MAX_TYPE_DEPTH;
use whim_value::atom::Atom;

use crate::chunk::descriptors::TypeDescriptor;
use crate::unit::CompiledBaseReference;
use crate::unit::CompiledClassLike;
use crate::unit::CompiledFunction;
use crate::unit::CompiledTypeAlias;
use crate::unit::CompiledTypeParameter;
use crate::unit::CompiledUnit;
use crate::unit::is_external;

pub trait TypeAliasLookup {
    fn find_alias(&self, name: &Atom) -> Option<&CompiledTypeAlias>;
}

pub struct TypeAliasIndex<'alias> {
    aliases: HashMap<Atom, &'alias CompiledTypeAlias>,
}

impl<'alias> TypeAliasIndex<'alias> {
    #[must_use]
    pub fn new(aliases: &'alias [CompiledTypeAlias]) -> Self {
        let mut index = HashMap::with_capacity(aliases.len());
        for alias in aliases {
            index.entry(alias.name.clone()).or_insert(alias);
        }

        Self { aliases: index }
    }
}

impl TypeAliasLookup for TypeAliasIndex<'_> {
    fn find_alias(&self, name: &Atom) -> Option<&CompiledTypeAlias> {
        self.aliases.get(name).copied()
    }
}

impl TypeAliasLookup for [CompiledTypeAlias] {
    fn find_alias(&self, name: &Atom) -> Option<&CompiledTypeAlias> {
        self.iter().find(|alias| alias.name == *name)
    }
}

impl TypeAliasLookup for [Rc<CompiledTypeAlias>] {
    fn find_alias(&self, name: &Atom) -> Option<&CompiledTypeAlias> {
        self.iter()
            .find(|alias| alias.name == *name)
            .map(Rc::as_ref)
    }
}

/// Expands known aliases in the runtime-checked declarations of `unit`.
/// Alias definitions stay canonical so recursive aliases expand only once.
pub fn expand_unit_declarations(unit: &mut CompiledUnit, aliases: &[CompiledTypeAlias]) {
    let aliases = TypeAliasIndex::new(aliases);
    for function in &mut unit.functions {
        if is_external(&function.attributes) {
            continue;
        }

        expand_function(function, &aliases);
    }

    for class in &mut unit.classes {
        if is_external(&class.attributes) {
            continue;
        }

        expand_class(class, &aliases);
    }

    for newtype in &mut unit.newtypes {
        if is_external(&newtype.attributes) {
            continue;
        }

        expand_parameters(&mut newtype.type_parameters, &aliases);
        newtype.backing = expand_aliases_using(&newtype.backing, &aliases);
    }
}

fn expand_function(function: &mut CompiledFunction, aliases: &impl TypeAliasLookup) {
    expand_parameters(&mut function.type_parameters, aliases);
    for parameter in &mut function.parameters {
        if let Some(descriptor) = &mut parameter.declared_type {
            *descriptor = expand_aliases_using(descriptor, aliases);
        }
    }

    if let Some(descriptor) = &mut function.return_type {
        *descriptor = expand_aliases_using(descriptor, aliases);
    }
}

fn expand_class(class: &mut CompiledClassLike, aliases: &impl TypeAliasLookup) {
    expand_parameters(&mut class.type_parameters, aliases);
    if let Some(parent) = &mut class.parent {
        expand_base(parent, aliases);
    }

    for interface in &mut class.interfaces {
        expand_base(interface, aliases);
    }

    for constant in &mut class.constants {
        if let Some(descriptor) = &mut constant.declared_type {
            *descriptor = expand_aliases_using(descriptor, aliases);
        }
    }

    for property in &mut class.properties {
        if let Some(descriptor) = &mut property.declared_type {
            *descriptor = expand_aliases_using(descriptor, aliases);
        }
    }

    for method in &mut class.methods {
        expand_function(&mut method.function, aliases);
    }
}

fn expand_base(base: &mut CompiledBaseReference, aliases: &impl TypeAliasLookup) {
    if let Some(arguments) = &mut base.type_arguments {
        for argument in arguments {
            *argument = expand_aliases_using(argument, aliases);
        }
    }
}

fn expand_parameters(parameters: &mut [CompiledTypeParameter], aliases: &impl TypeAliasLookup) {
    for parameter in parameters {
        for bound in &mut parameter.bounds {
            *bound = expand_aliases_using(bound, aliases);
        }

        if let Some(default) = &mut parameter.default {
            *default = expand_aliases_using(default, aliases);
        }
    }
}

#[must_use]
pub fn expand_aliases(
    descriptor: &TypeDescriptor,
    aliases: &[CompiledTypeAlias],
) -> TypeDescriptor {
    expand_aliases_using(descriptor, aliases)
}

pub fn expand_aliases_using(
    descriptor: &TypeDescriptor,
    aliases: &(impl TypeAliasLookup + ?Sized),
) -> TypeDescriptor {
    expand_descriptor(descriptor, aliases, 0, &mut Vec::new())
}

fn expand_descriptor<L: TypeAliasLookup + ?Sized>(
    descriptor: &TypeDescriptor,
    aliases: &L,
    depth: usize,
    expanding_aliases: &mut Vec<Atom>,
) -> TypeDescriptor {
    if depth > MAX_TYPE_DEPTH {
        return descriptor.clone();
    }

    match descriptor {
        TypeDescriptor::Named {
            name,
            arguments,
            recursive,
        } => {
            let expanded_arguments = arguments.as_ref().map(|arguments| {
                arguments
                    .iter()
                    .map(|argument| {
                        expand_descriptor(argument, aliases, depth + 1, expanding_aliases)
                    })
                    .collect::<Vec<_>>()
            });

            if *recursive {
                return TypeDescriptor::Named {
                    name: name.clone(),
                    arguments: expanded_arguments,
                    recursive: true,
                };
            }

            let Some(alias) = aliases.find_alias(name) else {
                return TypeDescriptor::Named {
                    name: name.clone(),
                    arguments: expanded_arguments,
                    recursive: false,
                };
            };

            if expanding_aliases.contains(name) {
                return TypeDescriptor::Named {
                    name: name.clone(),
                    arguments: expanded_arguments,
                    recursive: true,
                };
            }
            if alias
                .type_parameters
                .iter()
                .any(|parameter| !parameter.bounds.is_empty())
            {
                return TypeDescriptor::Named {
                    name: name.clone(),
                    arguments: expanded_arguments,
                    recursive: false,
                };
            }

            let Some(bindings) =
                alias_bindings(&alias.type_parameters, expanded_arguments.as_deref())
            else {
                return TypeDescriptor::Named {
                    name: name.clone(),
                    arguments: expanded_arguments,
                    recursive: false,
                };
            };

            let substituted = substitute(&alias.descriptor, &bindings, depth + 1);
            expanding_aliases.push(name.clone());
            let expanded = expand_descriptor(&substituted, aliases, depth + 1, expanding_aliases);
            expanding_aliases.pop();
            expanded
        }
        TypeDescriptor::Intersection(members) => {
            let mut expanded = Vec::with_capacity(members.len());
            for member in members {
                let member = expand_descriptor(member, aliases, depth + 1, expanding_aliases);
                if !matches!(member, TypeDescriptor::Mixed) {
                    expanded.push(member);
                }
            }

            match expanded.len() {
                0 => TypeDescriptor::Mixed,
                1 => expanded.remove(0),
                _ => TypeDescriptor::intersection(expanded),
            }
        }
        _ => descriptor
            .map_children(|child| expand_descriptor(child, aliases, depth + 1, expanding_aliases)),
    }
}

#[must_use]
pub fn alias_bindings(
    parameters: &[CompiledTypeParameter],
    arguments: Option<&[TypeDescriptor]>,
) -> Option<Vec<(Atom, TypeDescriptor)>> {
    let arguments = arguments.unwrap_or_default();
    if arguments.len() > parameters.len() {
        return None;
    }

    let mut bindings = Vec::with_capacity(parameters.len());
    for (position, parameter) in parameters.iter().enumerate() {
        let argument = match arguments.get(position) {
            Some(argument) => argument.clone(),
            None => substitute(parameter.default.as_ref()?, &bindings, 0),
        };

        bindings.push((parameter.name.clone(), argument));
    }

    Some(bindings)
}

#[must_use]
pub fn substitute(
    descriptor: &TypeDescriptor,
    bindings: &[(Atom, TypeDescriptor)],
    depth: usize,
) -> TypeDescriptor {
    if depth > MAX_TYPE_DEPTH {
        return descriptor.clone();
    }

    match descriptor {
        TypeDescriptor::Parameter(name) => bindings
            .iter()
            .find(|(parameter, _)| parameter == name)
            .map_or_else(|| descriptor.clone(), |(_, argument)| argument.clone()),
        _ => descriptor.map_children(|child| substitute(child, bindings, depth + 1)),
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use whim_span::Span;
    use whim_value::heap::Heap;

    use crate::aliases::expand_aliases;
    use crate::aliases::expand_unit_declarations;
    use crate::chunk::Chunk;
    use crate::chunk::descriptors::ShapeKey;
    use crate::chunk::descriptors::TypeDescriptor;
    use crate::unit::CompiledTypeAlias;
    use crate::unit::CompiledTypeParameter;
    use crate::unit::CompiledUnit;
    use crate::unit::Variance;

    #[test]
    fn expands_aliases_nested_in_array_shapes() {
        let heap = Heap::new();
        let alias_name = heap.intern(b"Scalar");
        let alias = CompiledTypeAlias {
            name: alias_name.clone(),
            span: Span::zero(),
            attributes: Vec::new(),
            type_parameters: Vec::new(),
            descriptor: TypeDescriptor::Int,
            rendered: heap.intern(b"int"),
        };
        let reference = || TypeDescriptor::Named {
            name: alias_name.clone(),
            arguments: None,
            recursive: false,
        };
        let descriptor = TypeDescriptor::Intersection(vec![
            TypeDescriptor::VectorShape {
                elements: vec![reference()],
                rest: Some(Box::new(reference())),
            },
            TypeDescriptor::DictionaryShape {
                entries: vec![(ShapeKey::Int(0), reference())],
                rest: Some((Box::new(reference()), Box::new(reference()))),
            },
        ]);

        let expanded = expand_aliases(&descriptor, &[alias]);
        let TypeDescriptor::Intersection(members) = expanded else {
            panic!("the intersection shape changed")
        };
        let TypeDescriptor::VectorShape { elements, rest } = &members[0] else {
            panic!("the vec shape changed")
        };
        assert!(matches!(elements.as_slice(), [TypeDescriptor::Int]));
        assert!(matches!(rest.as_deref(), Some(TypeDescriptor::Int)));
        let TypeDescriptor::DictionaryShape { entries, rest } = &members[1] else {
            panic!("the dict shape changed")
        };
        assert!(matches!(
            entries.as_slice(),
            [(ShapeKey::Int(0), TypeDescriptor::Int)]
        ));
        assert!(matches!(
            rest.as_ref()
                .map(|(key, value)| (key.as_ref(), value.as_ref())),
            Some((TypeDescriptor::Int, TypeDescriptor::Int))
        ));
    }

    #[test]
    fn removes_mixed_from_an_expanded_intersection() {
        let heap = Heap::new();
        let alias_name = heap.intern(b"Erased");
        let parameter_name = heap.intern(b"T");
        let alias = CompiledTypeAlias {
            name: alias_name.clone(),
            span: Span::zero(),
            attributes: Vec::new(),
            type_parameters: vec![CompiledTypeParameter {
                name: parameter_name,
                span: Span::zero(),
                variance: Variance::Invariant,
                bounds: Vec::new(),
                default: None,
            }],
            descriptor: TypeDescriptor::Mixed,
            rendered: heap.intern(b"mixed"),
        };
        let tuple = TypeDescriptor::TupleRest {
            elements: vec![TypeDescriptor::Int],
            rest: Box::new(TypeDescriptor::Mixed),
        };
        let descriptor = TypeDescriptor::Intersection(vec![
            tuple,
            TypeDescriptor::Named {
                name: alias_name,
                arguments: Some(vec![TypeDescriptor::String]),
                recursive: false,
            },
        ]);

        let expanded = expand_aliases(&descriptor, &[alias]);
        assert!(matches!(
            expanded,
            TypeDescriptor::TupleRest { elements, rest }
                if matches!(elements.as_slice(), [TypeDescriptor::Int])
                    && matches!(rest.as_ref(), TypeDescriptor::Mixed)
        ));
    }

    #[test]
    fn declaration_expansion_keeps_alias_definitions_canonical() {
        let heap = Heap::new();
        let datum_name = heap.intern(b"Datum");
        let data_name = heap.intern(b"Data");
        let datum = CompiledTypeAlias {
            name: datum_name.clone(),
            span: Span::zero(),
            attributes: Vec::new(),
            type_parameters: Vec::new(),
            descriptor: TypeDescriptor::Union(vec![
                TypeDescriptor::Int,
                TypeDescriptor::Vector(Some(Box::new(TypeDescriptor::Named {
                    name: datum_name.clone(),
                    arguments: None,
                    recursive: false,
                }))),
            ]),
            rendered: heap.intern(b"int|vec<Datum>"),
        };
        let data = CompiledTypeAlias {
            name: data_name,
            span: Span::zero(),
            attributes: Vec::new(),
            type_parameters: Vec::new(),
            descriptor: TypeDescriptor::Dictionary(Some((
                Box::new(TypeDescriptor::String),
                Box::new(TypeDescriptor::Named {
                    name: datum_name.clone(),
                    arguments: None,
                    recursive: false,
                }),
            ))),
            rendered: heap.intern(b"dict<string, Datum>"),
        };
        let mut unit = CompiledUnit {
            path: heap.intern(b"aliases.whim"),
            files: Vec::new(),
            main: Chunk::new(),
            functions: Vec::new(),
            classes: Vec::new(),
            constants: Vec::new(),
            type_aliases: vec![datum, data],
            newtypes: Vec::new(),
        };
        let aliases = unit.type_aliases.clone();

        expand_unit_declarations(&mut unit, &aliases);

        assert!(matches!(
            &unit.type_aliases[1].descriptor,
            TypeDescriptor::Dictionary(Some((key, value)))
                if matches!(key.as_ref(), TypeDescriptor::String)
                    && matches!(
                        value.as_ref(),
                        TypeDescriptor::Named {
                            name,
                            arguments: None,
                            ..
                        }
                            if name == &datum_name
                    )
        ));
    }

    #[test]
    fn recursive_alias_expansion_is_idempotent() {
        let heap = Heap::new();
        let name = heap.intern(b"Datum");
        let alias = CompiledTypeAlias {
            name: name.clone(),
            span: Span::zero(),
            attributes: Vec::new(),
            type_parameters: Vec::new(),
            descriptor: TypeDescriptor::Union(vec![
                TypeDescriptor::Int,
                TypeDescriptor::Vector(Some(Box::new(TypeDescriptor::Named {
                    name: name.clone(),
                    arguments: None,
                    recursive: false,
                }))),
            ]),
            rendered: heap.intern(b"int|vec<Datum>"),
        };
        let reference = TypeDescriptor::Named {
            name,
            arguments: None,
            recursive: false,
        };

        let once = expand_aliases(&reference, slice::from_ref(&alias));
        let twice = expand_aliases(&once, &[alias]);

        for descriptor in [&once, &twice] {
            assert!(matches!(
                descriptor,
                TypeDescriptor::Union(members)
                    if matches!(
                        members.as_slice(),
                        [
                            TypeDescriptor::Int,
                            TypeDescriptor::Vector(Some(recursive)),
                        ] if matches!(
                            recursive.as_ref(),
                            TypeDescriptor::Named {
                                recursive: true,
                                ..
                            }
                        )
                    )
            ));
        }
    }
}
