//! Whether tracked facts prove declared types: the query side of the
//! analysis.

use crate::limits::MAX_TYPE_DEPTH;
use crate::optimizer::cfg::dominates;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::type_flow::ALL;
use crate::optimizer::type_flow::ALWAYS_REFERENCE_COUNTED;
use crate::optimizer::type_flow::Atom;
use crate::optimizer::type_flow::BOOL;
use crate::optimizer::type_flow::CALLABLE;
use crate::optimizer::type_flow::CompiledParameter;
use crate::optimizer::type_flow::CompiledTypeParameter;
use crate::optimizer::type_flow::DICTIONARY;
use crate::optimizer::type_flow::DictionaryTypeDescriptor;
use crate::optimizer::type_flow::FLOAT;
use crate::optimizer::type_flow::Fact;
use crate::optimizer::type_flow::INT;
use crate::optimizer::type_flow::Instruction;
use crate::optimizer::type_flow::Literal;
use crate::optimizer::type_flow::MAY_BE_REFERENCE_COUNTED;
use crate::optimizer::type_flow::NO_ORIGIN;
use crate::optimizer::type_flow::NULL;
use crate::optimizer::type_flow::OBJECT;
use crate::optimizer::type_flow::Register;
use crate::optimizer::type_flow::STRING;
use crate::optimizer::type_flow::THIS_ORIGIN;
use crate::optimizer::type_flow::TUPLE;
use crate::optimizer::type_flow::TypeDescriptor;
use crate::optimizer::type_flow::TypeFlow;
use crate::optimizer::type_flow::VECTOR;
use crate::optimizer::type_flow::callable_signature;
use crate::optimizer::type_flow::descriptors::descriptor_mask;
use crate::optimizer::type_flow::descriptors::descriptor_proves;
use crate::optimizer::type_flow::descriptors::descriptor_slices_equal;
use crate::optimizer::type_flow::descriptors::descriptors_disjoint;
use crate::optimizer::type_flow::descriptors::exact_descriptor_mask;
use crate::optimizer::type_flow::descriptors::literal_descriptor_disjoint;
use crate::optimizer::type_flow::descriptors::literal_descriptor_matches;
use crate::optimizer::type_flow::descriptors::substitute_parameters;
use crate::optimizer::type_flow::fact_bits;
use crate::optimizer::type_flow::instruction_index;
use crate::optimizer::type_flow::same_atom;

impl TypeFlow<'_> {
    pub(in crate::optimizer) fn callable_arguments_proven(
        &self,
        index: usize,
        callee: Register,
        first: usize,
        count: usize,
    ) -> bool {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return false;
        }
        let callee = usize::from(callee.index());
        if callee >= usize::from(self.chunk.register_count) {
            return false;
        }
        let Some(descriptor) =
            self.origin_type(self.fact(index, Register::new(callee as u16)).origin, 0)
        else {
            return false;
        };
        let descriptor = self.expand_aliases_owned(descriptor);
        let Some(signature) = callable_signature(&descriptor) else {
            return false;
        };
        let required = signature
            .parameters
            .iter()
            .filter(|parameter| !parameter.optional)
            .count();
        if count < required || count > signature.parameters.len() {
            return false;
        }

        signature
            .parameters
            .iter()
            .take(count)
            .enumerate()
            .all(|(offset, parameter)| {
                let register = first + offset;
                if register >= usize::from(self.chunk.register_count) {
                    return false;
                }
                let register = Register::new(register as u16);
                if self.argument_proves(index, register, &parameter.r#type) {
                    return true;
                }
                let Some(actual) = self.register_type_at(index, register, 0) else {
                    return false;
                };
                let actual = self.expand_aliases_owned(actual);
                self.descriptor_proves(&actual, &parameter.r#type, 0)
            })
    }

    pub(in crate::optimizer) fn method_arguments_proven(
        &self,
        index: usize,
        first: usize,
        count: usize,
    ) -> bool {
        if self.resolved_method_at(index, 0).is_some_and(|method| {
            self.arguments_proven(index, &method.function.parameters, first, count)
        }) {
            return true;
        }
        let Instruction::CallMethod {
            first_argument,
            cache,
            ..
        } = self.chunk.code[index]
        else {
            return false;
        };
        let Some(TypeDescriptor::Named {
            name, arguments, ..
        }) = self.origin_type(self.fact(index, first_argument).origin, 0)
        else {
            return false;
        };
        let Some(class) = self.class_like(&name) else {
            return false;
        };
        let Some((method_name, method_arguments)) = self.member_descriptor(cache) else {
            return false;
        };
        let Some(method) = class
            .methods
            .iter()
            .find(|method| !method.is_static && same_atom(&method.name, method_name))
        else {
            return false;
        };
        if count > method.function.parameters.len()
            || method.function.parameters[count..]
                .iter()
                .any(|parameter| !parameter.has_default)
        {
            return false;
        }
        if method.function.type_parameters.is_empty() {
            if method_arguments.is_some_and(|arguments| !arguments.is_empty()) {
                return false;
            }
        } else if method_arguments
            .is_none_or(|arguments| arguments.len() != method.function.type_parameters.len())
        {
            return false;
        }
        method
            .function
            .parameters
            .iter()
            .take(count)
            .enumerate()
            .all(|(offset, parameter)| {
                let register = first + offset;
                register < usize::from(self.chunk.register_count)
                    && parameter.declared_type.as_ref().is_none_or(|expected| {
                        let expected = substitute_parameters(
                            expected,
                            &method.function.type_parameters,
                            method_arguments,
                            0,
                        );
                        let expected = substitute_parameters(
                            &expected,
                            &class.type_parameters,
                            arguments.as_deref(),
                            0,
                        );
                        self.argument_proves(index, Register::new(register as u16), &expected)
                    })
            })
    }

    pub(in crate::optimizer) fn arguments_proven(
        &self,
        index: usize,
        parameters: &[CompiledParameter],
        first: usize,
        count: usize,
    ) -> bool {
        count <= parameters.len()
            && parameters[count..]
                .iter()
                .all(|parameter| parameter.has_default)
            && parameters
                .iter()
                .take(count)
                .enumerate()
                .all(|(offset, parameter)| {
                    let register = first + offset;
                    register < usize::from(self.chunk.register_count)
                        && parameter.declared_type.as_ref().is_none_or(|expected| {
                            self.argument_proves(index, Register::new(register as u16), expected)
                        })
                })
    }

    pub(in crate::optimizer) fn function_arguments_proven(
        &self,
        index: usize,
        first: usize,
        count: usize,
    ) -> bool {
        let type_arguments = match self.chunk.code[index] {
            Instruction::CallNamed { cache, .. }
            | Instruction::CallNamedUnchecked { cache, .. } => {
                let Some((_, type_arguments)) = self.member_descriptor(cache) else {
                    return false;
                };
                type_arguments
            }
            _ => None,
        };
        if let Some(function) = self.resolved_function(index) {
            return self.callee_arguments_proven(
                index,
                first,
                count,
                &function.parameters,
                &function.type_parameters,
                type_arguments,
            );
        }
        let Some(function) = self.resolved_built_in_function(index) else {
            return false;
        };
        self.callee_arguments_proven(
            index,
            first,
            count,
            &function.parameters,
            &function.type_parameters,
            type_arguments,
        )
    }

    /// Whether every argument at a call site is proven against `callee`'s
    /// declared parameter types, with `type_arguments` substituted for the
    /// callee's own type parameters.
    pub(in crate::optimizer) fn callee_arguments_proven(
        &self,
        index: usize,
        first: usize,
        count: usize,
        parameters: &[CompiledParameter],
        callee_type_parameters: &[CompiledTypeParameter],
        type_arguments: Option<&[TypeDescriptor]>,
    ) -> bool {
        if count > parameters.len()
            || parameters[count..]
                .iter()
                .any(|parameter| !parameter.has_default)
        {
            return false;
        }
        if !callee_type_parameters.is_empty() && type_arguments.is_none() {
            return false;
        }
        if callee_type_parameters.is_empty() && type_arguments.is_some() {
            return false;
        }
        parameters
            .iter()
            .take(count)
            .enumerate()
            .all(|(offset, parameter)| {
                let register = first + offset;
                register < usize::from(self.chunk.register_count)
                    && parameter.declared_type.as_ref().is_none_or(|expected| {
                        let expected = substitute_parameters(
                            expected,
                            callee_type_parameters,
                            type_arguments,
                            0,
                        );
                        let register = Register::new(register as u16);
                        self.argument_proves(index, register, &expected)
                    })
            })
    }

    pub(in crate::optimizer) fn proves(
        &self,
        index: usize,
        register: Register,
        expected: &TypeDescriptor,
    ) -> bool {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return false;
        }
        let register_index = usize::from(register.index());
        if register_index >= usize::from(self.chunk.register_count) {
            return false;
        }
        let fact = self.fact(index, register);
        if self.fact_proves(fact, expected, 0) {
            return true;
        }

        let expected = self.expanded_aliases(expected);
        if self.fact_proves(fact, &expected, 0) {
            return true;
        }

        let Some(actual) = self.register_type_at(index, register, 0) else {
            return false;
        };
        let actual = self.expand_aliases_owned(actual);

        self.descriptor_proves(&actual, &expected, 0)
    }

    fn argument_proves(&self, index: usize, register: Register, expected: &TypeDescriptor) -> bool {
        self.proves(index, register, expected)
            || self.proves_constructed_array(index, register, expected)
    }

    pub(in crate::optimizer) fn destructure_proven(
        &self,
        index: usize,
        subject: Register,
        required: usize,
        arity: usize,
        rest: bool,
    ) -> bool {
        let Some(descriptor) = self.register_type_at(index, subject, 0) else {
            return false;
        };
        let descriptor = self.expand_aliases_owned(descriptor);

        descriptor_proves_destructure(&descriptor, required, arity, rest, 0)
    }

    /// Whether a fresh vec or dict is populated exclusively with values that
    /// satisfy its declared return specialization.
    pub(in crate::optimizer) fn proves_constructed_array(
        &self,
        index: usize,
        register: Register,
        expected: &TypeDescriptor,
    ) -> bool {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return false;
        }

        let expected = self.expanded_aliases(expected);

        match expected.as_ref() {
            TypeDescriptor::Union(members) => members
                .iter()
                .any(|member| self.proves_constructed_array(index, register, member)),
            TypeDescriptor::Intersection(members) => members
                .iter()
                .all(|member| self.proves_constructed_array(index, register, member)),
            TypeDescriptor::Array(Some((key, value))) => {
                self.descriptor_proves(&TypeDescriptor::integer_range(Some(0), None), key, 0)
                    && self.proves_constructed_vector(index, register, value)
                    || self.proves_constructed_dictionary(index, register, key, value)
            }
            TypeDescriptor::Dictionary(Some((key, value))) => {
                self.proves_constructed_dictionary(index, register, key, value)
            }
            TypeDescriptor::Vector(Some(element)) => {
                self.proves_constructed_vector(index, register, element)
            }
            _ => false,
        }
    }

    fn proves_constructed_dictionary(
        &self,
        index: usize,
        register: Register,
        expected_key: &TypeDescriptor,
        expected_value: &TypeDescriptor,
    ) -> bool {
        let expected = TypeDescriptor::Dictionary(Some((
            Box::new(expected_key.clone()),
            Box::new(expected_value.clone()),
        )));
        let Some((initializer, fresh)) =
            self.typed_array_initializer(index, register, &expected, |instruction| {
                matches!(
                    instruction,
                    Instruction::NewDict { destination, .. } if destination == register
                )
            })
        else {
            return false;
        };
        if fresh {
            let Instruction::NewDict {
                pair_count,
                first_pair,
                ..
            } = self.chunk.code[initializer]
            else {
                return false;
            };
            for pair in 0..usize::from(pair_count.value()) {
                let key = Register::new(first_pair.index() + (pair * 2) as u16);
                let value = Register::new(key.index() + 1);
                if !self.register_dictionary_key_descriptor_proves(initializer, key, expected_key)
                    || !self.register_descriptor_proves(initializer, value, expected_value)
                {
                    return false;
                }
            }
        }

        for position in initializer + 1..index {
            let instruction = self.chunk.code[position];
            match instruction {
                Instruction::IndexSet {
                    container,
                    index: key,
                    value,
                }
                | Instruction::DictIndexSet {
                    container,
                    index: key,
                    value,
                }
                | Instruction::DictIndexSetIntKey {
                    container,
                    index: key,
                    value,
                }
                | Instruction::DictIndexSetStringKey {
                    container,
                    index: key,
                    value,
                } if container == register => {
                    if !self.register_dictionary_key_descriptor_proves(position, key, expected_key)
                        || !self.register_descriptor_proves(position, value, expected_value)
                    {
                        return false;
                    }
                    continue;
                }
                Instruction::Append { container, .. }
                | Instruction::Spread { container, .. }
                | Instruction::IndexAddAssign { container, .. }
                    if container == register =>
                {
                    return false;
                }
                _ => {}
            }

            if effect_on(self.chunk, instruction, register).writes() {
                return false;
            }
        }

        true
    }

    fn proves_constructed_vector(
        &self,
        index: usize,
        register: Register,
        expected_element: &TypeDescriptor,
    ) -> bool {
        let expected = TypeDescriptor::Vector(Some(Box::new(expected_element.clone())));
        let Some((initializer, fresh)) =
            self.typed_array_initializer(index, register, &expected, |instruction| {
                matches!(
                    instruction,
                    Instruction::NewVec { destination, .. } if destination == register
                )
            })
        else {
            return false;
        };
        if fresh {
            let Instruction::NewVec {
                element_count,
                first_element,
                ..
            } = self.chunk.code[initializer]
            else {
                return false;
            };
            for element in 0..usize::from(element_count.value()) {
                let value = Register::new(first_element.index() + element as u16);
                if !self.register_descriptor_proves(initializer, value, expected_element) {
                    return false;
                }
            }
        }

        for position in initializer + 1..index {
            let instruction = self.chunk.code[position];
            match instruction {
                Instruction::Append { container, value }
                | Instruction::VecAppend { container, value }
                    if container == register =>
                {
                    if !self.register_descriptor_proves(position, value, expected_element) {
                        return false;
                    }
                    continue;
                }
                Instruction::IndexSet {
                    container, value, ..
                }
                | Instruction::VecIndexSet {
                    container, value, ..
                } if container == register => {
                    if !self.register_descriptor_proves(position, value, expected_element) {
                        return false;
                    }
                    continue;
                }
                Instruction::Spread { container, .. }
                | Instruction::IndexAddAssign { container, .. }
                    if container == register =>
                {
                    return false;
                }
                _ => {}
            }

            if effect_on(self.chunk, instruction, register).writes() {
                return false;
            }
        }

        true
    }

    fn typed_array_initializer(
        &self,
        index: usize,
        register: Register,
        expected: &TypeDescriptor,
        fresh: impl Fn(Instruction) -> bool,
    ) -> Option<(usize, bool)> {
        for candidate in (0..index).rev() {
            if !dominates(self.chunk, candidate, index) {
                continue;
            }
            let instruction = self.chunk.code[candidate];
            if fresh(instruction) {
                return Some((candidate, true));
            }
            if !effect_on(self.chunk, instruction, register).writes() {
                continue;
            }

            return (candidate + 1 < self.chunk.code.len()
                && self.register_descriptor_proves(candidate + 1, register, expected))
            .then_some((candidate, false));
        }

        None
    }

    fn register_descriptor_proves(
        &self,
        index: usize,
        register: Register,
        expected: &TypeDescriptor,
    ) -> bool {
        if self.proves(index, register, expected) {
            return true;
        }
        let Some(actual) = self.register_type_at(index, register, 0) else {
            return false;
        };
        let actual = self.expand_aliases_owned(actual);
        let expected = self.expanded_aliases(expected);

        self.descriptor_proves(&actual, &expected, 0)
    }

    fn register_dictionary_key_descriptor_proves(
        &self,
        index: usize,
        register: Register,
        expected: &TypeDescriptor,
    ) -> bool {
        let Some(actual) = self.register_type_at(index, register, 0) else {
            return false;
        };
        let Some(actual) = self.stored_dictionary_key_descriptor(&actual, 0) else {
            return false;
        };
        let expected = self.expanded_aliases(expected);

        self.descriptor_proves(&actual, &expected, 0)
    }

    fn stored_dictionary_key_descriptor(
        &self,
        descriptor: &TypeDescriptor,
        depth: usize,
    ) -> Option<TypeDescriptor> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        let descriptor = self.expanded_aliases(descriptor);

        match descriptor.as_ref() {
            TypeDescriptor::Named {
                name, arguments, ..
            } => {
                let Some(unit) = self.unit else {
                    return Some(descriptor.as_ref().clone());
                };
                let Some(newtype) = unit.newtype_by_name(name) else {
                    return Some(descriptor.as_ref().clone());
                };
                let backing = substitute_parameters(
                    &newtype.backing,
                    &newtype.type_parameters,
                    arguments.as_deref(),
                    depth + 1,
                );
                self.stored_dictionary_key_descriptor(&backing, depth + 1)
            }
            TypeDescriptor::Parameter(_) => None,
            TypeDescriptor::Union(members) => members
                .iter()
                .map(|member| self.stored_dictionary_key_descriptor(member, depth + 1))
                .collect::<Option<Vec<_>>>()
                .map(TypeDescriptor::Union),
            TypeDescriptor::Intersection(members) => members
                .iter()
                .map(|member| self.stored_dictionary_key_descriptor(member, depth + 1))
                .collect::<Option<Vec<_>>>()
                .map(TypeDescriptor::Intersection),
            _ => Some(descriptor.as_ref().clone()),
        }
    }

    pub(in crate::optimizer) fn proves_array_element(
        &self,
        index: usize,
        register: Register,
        expected: &TypeDescriptor,
    ) -> bool {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return false;
        }
        let register = usize::from(register.index());
        if register >= usize::from(self.chunk.register_count) {
            return false;
        }
        let collection = self.fact(index, Register::new(register as u16)).array;
        if collection == NO_ORIGIN {
            return false;
        }
        let Some(expected) = exact_descriptor_mask(expected) else {
            return false;
        };
        let elements = self
            .array_elements
            .get(collection as usize)
            .copied()
            .unwrap_or(0);
        elements != 0 && elements & !expected == 0
    }

    pub(in crate::optimizer) fn proves_reference_counted(
        &self,
        index: usize,
        register: Register,
    ) -> bool {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return false;
        }
        let register = usize::from(register.index());
        if register >= usize::from(self.chunk.register_count) {
            return false;
        }
        let fact = self.fact(index, Register::new(register as u16));
        let mask = fact.mask;
        if mask != 0 && mask & !ALWAYS_REFERENCE_COUNTED == 0 {
            return true;
        }

        mask == STRING && self.origin_is_boxed_string(fact.origin)
    }

    /// Constant-pool strings are materialized as managed strings by the VM.
    /// Other string values may be inline, so only this exact origin is safe to
    /// classify as reference-counted without a runtime tag check.
    fn origin_is_boxed_string(&self, origin: u32) -> bool {
        let Some(index) = origin
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
        else {
            return false;
        };
        let Some(Instruction::LoadConstant { constant, .. }) = self.chunk.code.get(index) else {
            return false;
        };
        matches!(
            self.chunk.constants.get(usize::from(constant.index())),
            Some(Literal::String(_))
        )
    }

    pub(in crate::optimizer) fn proves_scalar(&self, index: usize, register: Register) -> bool {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return false;
        }
        let register = usize::from(register.index());
        if register >= usize::from(self.chunk.register_count) {
            return false;
        }
        let mask = self.fact(index, Register::new(register as u16)).mask;
        mask != 0 && mask & MAY_BE_REFERENCE_COUNTED == 0
    }

    pub(in crate::optimizer::type_flow) fn fact_proves(
        &self,
        fact: Fact,
        expected: &TypeDescriptor,
        depth: usize,
    ) -> bool {
        if depth > MAX_TYPE_DEPTH {
            return false;
        }
        if matches!(expected, TypeDescriptor::Wildcard | TypeDescriptor::Mixed) {
            return true;
        }

        if self.new_static_proves(fact, expected, depth + 1) {
            return true;
        }
        if let Some(actual) = self.origin_type(fact.origin, depth + 1)
            && self.descriptor_proves(&actual, expected, depth + 1)
        {
            return true;
        }
        if fact.mask == 0 || fact.mask == ALL {
            return false;
        }

        match expected {
            TypeDescriptor::Void | TypeDescriptor::Null => fact.mask & !NULL == 0,
            TypeDescriptor::Bool => fact.mask & !BOOL == 0,
            TypeDescriptor::Int => fact.mask & !INT == 0,
            TypeDescriptor::Float => fact.mask & !FLOAT == 0,
            TypeDescriptor::String => fact.mask & !STRING == 0,
            TypeDescriptor::Object => fact.mask & !OBJECT == 0,
            TypeDescriptor::IntRange { min, max }
                if fact.positive && min.is_none_or(|min| min <= 1) && max.is_none() =>
            {
                true
            }
            TypeDescriptor::IntRange { min, max }
                if fact.non_negative && min.is_none_or(|min| min <= 0) && max.is_none() =>
            {
                true
            }
            TypeDescriptor::TrueLiteral
            | TypeDescriptor::FalseLiteral
            | TypeDescriptor::IntLiteral(_)
            | TypeDescriptor::IntRange { .. }
            | TypeDescriptor::FloatLiteral(_)
            | TypeDescriptor::StringLiteral(_) => self.literal_matches(fact, expected),
            TypeDescriptor::Named {
                name, arguments, ..
            } => {
                fact.origin == THIS_ORIGIN
                    && self.class_name.is_some_and(|class| same_atom(class, name))
                    && self.this_arguments_match(arguments.as_deref())
            }
            TypeDescriptor::StaticClass => fact.origin == THIS_ORIGIN,
            TypeDescriptor::Array(arguments) => match fact.mask {
                VECTOR => arguments.as_ref().is_none_or(|(key, value)| {
                    self.descriptor_proves(&TypeDescriptor::Int, key, depth + 1)
                        && self.array_proves(fact, Some(value), false, depth + 1)
                }),
                DICTIONARY => self.dictionary_proves(fact, arguments.as_ref(), depth + 1),
                TUPLE => arguments.as_ref().is_none_or(|(_, value)| {
                    let Some(index) = instruction_index(fact.origin) else {
                        return false;
                    };
                    let Instruction::NewTuple {
                        element_count,
                        first_element,
                        ..
                    } = self.chunk.code[index]
                    else {
                        return false;
                    };
                    (0..usize::from(element_count.value())).all(|position| {
                        let register = usize::from(first_element.index()) + position;
                        register < usize::from(self.chunk.register_count)
                            && self.fact_proves(
                                self.fact(index, Register::new(register as u16)),
                                value,
                                depth + 1,
                            )
                    })
                }),
                _ => false,
            },
            TypeDescriptor::Vector(element) => {
                fact.mask == VECTOR && self.array_proves(fact, element.as_deref(), false, depth)
            }
            TypeDescriptor::Dictionary(arguments) => {
                fact.mask == DICTIONARY && self.dictionary_proves(fact, arguments.as_ref(), depth)
            }
            TypeDescriptor::Callable(None) => fact.mask & !CALLABLE == 0,
            TypeDescriptor::Callable(Some(expected)) => {
                let Some(actual) = self.origin_type(fact.origin, depth + 1) else {
                    return false;
                };
                let Some(actual) = callable_signature(&actual) else {
                    return false;
                };
                actual.parameters.len() == expected.parameters.len()
                    && actual.parameters.iter().zip(&expected.parameters).all(
                        |(actual, expected)| {
                            actual.optional == expected.optional
                                && self.descriptor_proves(
                                    &actual.r#type,
                                    &expected.r#type,
                                    depth + 1,
                                )
                        },
                    )
                    && self.descriptor_proves(&actual.return_type, &expected.return_type, depth + 1)
            }
            TypeDescriptor::Tuple(members) => {
                fact.mask == TUPLE && self.tuple_proves(fact, members, depth)
            }
            TypeDescriptor::TupleAny => fact.mask & !TUPLE == 0,
            TypeDescriptor::Union(members) => fact_bits(fact).all(|part| {
                members
                    .iter()
                    .any(|member| self.fact_proves(part, member, depth + 1))
            }),
            TypeDescriptor::Intersection(members) => members
                .iter()
                .all(|member| self.fact_proves(fact, member, depth + 1)),
            TypeDescriptor::Negated(inner) => {
                descriptor_mask(inner).is_some_and(|excluded| fact.mask & excluded == 0)
                    || self.literal_disjoint(fact, inner)
            }
            TypeDescriptor::Never
            | TypeDescriptor::Member { .. }
            | TypeDescriptor::Parameter(_)
            | TypeDescriptor::VectorShape { .. }
            | TypeDescriptor::DictionaryShape { .. }
            | TypeDescriptor::Classname(_)
            | TypeDescriptor::TupleRest { .. } => false,
            TypeDescriptor::Wildcard | TypeDescriptor::Mixed => true,
        }
    }

    pub(in crate::optimizer::type_flow) fn array_proves(
        &self,
        fact: Fact,
        element: Option<&TypeDescriptor>,
        dictionary_values: bool,
        depth: usize,
    ) -> bool {
        let Some(element) = element else {
            return true;
        };

        if fact.array != NO_ORIGIN {
            let elements = self
                .array_elements
                .get(fact.array as usize)
                .copied()
                .unwrap_or(0);
            if elements == 0
                || exact_descriptor_mask(element).is_some_and(|expected| elements & !expected == 0)
            {
                return true;
            }
        }

        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        let instruction = self.chunk.code[index];
        let (count, first, stride, offset) = match instruction {
            Instruction::NewVec {
                element_count,
                first_element,
                ..
            } if !dictionary_values => (usize::from(element_count.value()), first_element, 1, 0),
            Instruction::NewDict {
                pair_count,
                first_pair,
                ..
            } if dictionary_values => (usize::from(pair_count.value()), first_pair, 2, 1),
            _ => return false,
        };
        (0..count).all(|position| {
            let register = usize::from(first.index()) + position * stride + offset;
            register < usize::from(self.chunk.register_count)
                && self.fact_proves(
                    self.fact(index, Register::new(register as u16)),
                    element,
                    depth + 1,
                )
        })
    }

    pub(in crate::optimizer::type_flow) fn dictionary_proves(
        &self,
        fact: Fact,
        arguments: Option<&DictionaryTypeDescriptor>,
        depth: usize,
    ) -> bool {
        let Some((key, value)) = arguments else {
            return true;
        };
        self.dictionary_side_proves(fact, key, 0, depth)
            && self.dictionary_side_proves(fact, value, 1, depth)
    }

    pub(in crate::optimizer::type_flow) fn dictionary_side_proves(
        &self,
        fact: Fact,
        expected: &TypeDescriptor,
        offset: usize,
        depth: usize,
    ) -> bool {
        if fact.array != NO_ORIGIN {
            let observed = if offset == 0 {
                &self.array_keys
            } else {
                &self.array_elements
            }
            .get(fact.array as usize)
            .copied()
            .unwrap_or(0);
            if observed == 0
                || exact_descriptor_mask(expected).is_some_and(|expected| observed & !expected == 0)
            {
                return true;
            }
        }

        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        let Instruction::NewDict {
            pair_count,
            first_pair,
            ..
        } = self.chunk.code[index]
        else {
            return false;
        };
        (0..usize::from(pair_count.value())).all(|position| {
            let register = usize::from(first_pair.index()) + position * 2 + offset;
            register < usize::from(self.chunk.register_count)
                && self.fact_proves(
                    self.fact(index, Register::new(register as u16)),
                    expected,
                    depth + 1,
                )
        })
    }

    pub(in crate::optimizer::type_flow) fn tuple_proves(
        &self,
        fact: Fact,
        members: &[TypeDescriptor],
        depth: usize,
    ) -> bool {
        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        let Instruction::NewTuple {
            element_count,
            first_element,
            ..
        } = self.chunk.code[index]
        else {
            return false;
        };
        if usize::from(element_count.value()) != members.len() {
            return false;
        }
        members.iter().enumerate().all(|(position, expected)| {
            let register = usize::from(first_element.index()) + position;
            register < usize::from(self.chunk.register_count)
                && self.fact_proves(
                    self.fact(index, Register::new(register as u16)),
                    expected,
                    depth + 1,
                )
        })
    }

    pub(in crate::optimizer::type_flow) fn literal_matches(
        &self,
        fact: Fact,
        expected: &TypeDescriptor,
    ) -> bool {
        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        match self.chunk.code[index] {
            Instruction::LoadTrue { .. } => matches!(expected, TypeDescriptor::TrueLiteral),
            Instruction::LoadFalse { .. } => matches!(expected, TypeDescriptor::FalseLiteral),
            Instruction::LoadInt { immediate, .. } => descriptor_proves(
                &TypeDescriptor::IntLiteral(i64::from(immediate.value())),
                expected,
                self.unit,
                0,
            ),
            Instruction::LoadConstant { constant, .. } => literal_descriptor_matches(
                &self.chunk.constants[usize::from(constant.index())],
                expected,
            ),
            _ => false,
        }
    }

    fn literal_disjoint(&self, fact: Fact, excluded: &TypeDescriptor) -> bool {
        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        match self.chunk.code[index] {
            Instruction::LoadTrue { .. } => {
                descriptors_disjoint(&TypeDescriptor::TrueLiteral, excluded, 0)
            }
            Instruction::LoadFalse { .. } => {
                descriptors_disjoint(&TypeDescriptor::FalseLiteral, excluded, 0)
            }
            Instruction::LoadInt { immediate, .. } => descriptors_disjoint(
                &TypeDescriptor::IntLiteral(i64::from(immediate.value())),
                excluded,
                0,
            ),
            Instruction::LoadConstant { constant, .. } => literal_descriptor_disjoint(
                &self.chunk.constants[usize::from(constant.index())],
                excluded,
            ),
            _ => false,
        }
    }

    pub(in crate::optimizer::type_flow) fn new_static_proves(
        &self,
        fact: Fact,
        expected: &TypeDescriptor,
        depth: usize,
    ) -> bool {
        if depth > MAX_TYPE_DEPTH {
            return false;
        }
        let TypeDescriptor::Named {
            name: expected_name,
            arguments: expected_arguments,
            ..
        } = expected
        else {
            return false;
        };
        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        let Instruction::NewStatic { cache, .. } = self.chunk.code[index] else {
            return false;
        };
        let Some((name, type_arguments)) = self.member_descriptor(cache) else {
            return false;
        };
        self.nominal_named_proves(
            name,
            type_arguments,
            expected_name,
            expected_arguments.as_deref(),
            depth + 1,
        )
    }

    pub(in crate::optimizer) fn descriptor_proves(
        &self,
        actual: &TypeDescriptor,
        expected: &TypeDescriptor,
        depth: usize,
    ) -> bool {
        descriptor_proves(actual, expected, self.unit, depth)
            || self.nominal_descriptor_proves(actual, expected, depth)
    }

    fn nominal_descriptor_proves(
        &self,
        actual: &TypeDescriptor,
        expected: &TypeDescriptor,
        depth: usize,
    ) -> bool {
        let (
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
        ) = (actual, expected)
        else {
            return false;
        };
        self.nominal_named_proves(
            actual_name,
            actual_arguments.as_deref(),
            expected_name,
            expected_arguments.as_deref(),
            depth + 1,
        )
    }

    pub(in crate::optimizer) fn nominal_named_proves(
        &self,
        actual_name: &Atom,
        actual_arguments: Option<&[TypeDescriptor]>,
        expected_name: &Atom,
        expected_arguments: Option<&[TypeDescriptor]>,
        depth: usize,
    ) -> bool {
        if depth > MAX_TYPE_DEPTH {
            return false;
        }
        if same_atom(actual_name, expected_name) {
            return match (actual_arguments, expected_arguments) {
                (None, None) => true,
                (Some(actual), Some(expected)) => {
                    descriptor_slices_equal(actual, expected, depth + 1)
                }
                _ => false,
            };
        }
        let Some(actual) = self.class_like(actual_name) else {
            return false;
        };
        actual.parent.iter().chain(&actual.interfaces).any(|base| {
            let arguments = base.type_arguments.as_ref().map(|arguments| {
                arguments
                    .iter()
                    .map(|argument| {
                        substitute_parameters(
                            argument,
                            &actual.type_parameters,
                            actual_arguments,
                            depth + 1,
                        )
                    })
                    .collect::<Vec<_>>()
            });
            self.nominal_named_proves(
                &base.name,
                arguments.as_deref(),
                expected_name,
                expected_arguments,
                depth + 1,
            )
        })
    }
}

fn descriptor_proves_destructure(
    descriptor: &TypeDescriptor,
    required: usize,
    arity: usize,
    rest: bool,
    depth: usize,
) -> bool {
    if depth > MAX_TYPE_DEPTH {
        return false;
    }

    match descriptor {
        TypeDescriptor::Tuple(members) => {
            destructure_length_satisfies(members.len(), required, arity, rest)
        }
        TypeDescriptor::TupleRest { elements, .. }
        | TypeDescriptor::VectorShape {
            elements,
            rest: Some(_),
        } => rest && elements.len() >= required,
        TypeDescriptor::VectorShape {
            elements,
            rest: None,
        } => destructure_length_satisfies(elements.len(), required, arity, rest),
        TypeDescriptor::Vector(Some(element))
            if matches!(element.as_ref(), TypeDescriptor::Never) =>
        {
            destructure_length_satisfies(0, required, arity, rest)
        }
        TypeDescriptor::Union(members) => members
            .iter()
            .all(|member| descriptor_proves_destructure(member, required, arity, rest, depth + 1)),
        TypeDescriptor::Intersection(members) => members
            .iter()
            .any(|member| descriptor_proves_destructure(member, required, arity, rest, depth + 1)),
        _ => false,
    }
}

fn destructure_length_satisfies(length: usize, required: usize, arity: usize, rest: bool) -> bool {
    if rest {
        length >= required
    } else {
        length >= required && length <= arity
    }
}
