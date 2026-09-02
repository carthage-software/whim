//! Resolving facts back to declarations: origins, exact classes, and
//! callee lookups.

use crate::bytecode::unit::CompiledBuiltInFunction;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::ConstantInitializer;
use crate::limits::MAX_TYPE_DEPTH;
use crate::linker::SlotPlacement;
use crate::linker::slot_placement;
use crate::optimizer::cfg::dominates;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::type_flow::Atom;
use crate::optimizer::type_flow::CAPTURE_ORIGIN;
use crate::optimizer::type_flow::ClassLikeKind;
use crate::optimizer::type_flow::CompiledClassLike;
use crate::optimizer::type_flow::CompiledMethod;
use crate::optimizer::type_flow::CompiledProperty;
use crate::optimizer::type_flow::ConstantValue;
use crate::optimizer::type_flow::ExactClass;
use crate::optimizer::type_flow::Fact;
use crate::optimizer::type_flow::FunctionTypeDescriptor;
use crate::optimizer::type_flow::FunctionTypeParameterDescriptor;
use crate::optimizer::type_flow::IcDescriptor;
use crate::optimizer::type_flow::IcSlot;
use crate::optimizer::type_flow::Instruction;
use crate::optimizer::type_flow::Literal;
use crate::optimizer::type_flow::NULL;
use crate::optimizer::type_flow::PARAMETER_ORIGIN;
use crate::optimizer::type_flow::Register;
use crate::optimizer::type_flow::ResolvedProperty;
use crate::optimizer::type_flow::STRING;
use crate::optimizer::type_flow::THIS_ORIGIN;
use crate::optimizer::type_flow::TypeDescriptor;
use crate::optimizer::type_flow::TypeFlow;
use crate::optimizer::type_flow::callable_signature;
use crate::optimizer::type_flow::descriptors::descriptor_mask;
use crate::optimizer::type_flow::descriptors::descriptors_equal;
use crate::optimizer::type_flow::descriptors::substitute_parameters;
use crate::optimizer::type_flow::instruction_index;
use crate::optimizer::type_flow::ptr;
use crate::optimizer::type_flow::same_atom;

impl<'a> TypeFlow<'a> {
    pub(in crate::optimizer) fn register_type_at(
        &self,
        index: usize,
        register: Register,
        depth: usize,
    ) -> Option<TypeDescriptor> {
        if depth > MAX_TYPE_DEPTH || index >= self.chunk.code.len() || !self.reachable[index] {
            return None;
        }
        let fact = self.fact(index, register);
        if let Some(producer) = instruction_index(fact.origin) {
            let (iterator, key_destination, value_destination) = match self.chunk.code[producer] {
                Instruction::ForeachNext {
                    iterator,
                    key_destination,
                    value_destination,
                }
                | Instruction::VecForeachNext {
                    iterator,
                    key_destination,
                    value_destination,
                    ..
                }
                | Instruction::DictForeachNext {
                    iterator,
                    key_destination,
                    value_destination,
                    ..
                } => (iterator, key_destination, value_destination),
                _ => {
                    return self.origin_type_matching_mask(fact, depth + 1);
                }
            };
            let key = register == key_destination;
            if !key && register != value_destination {
                let Some((position, source)) = self.moved_register_source(index, register) else {
                    return self.origin_type(fact.origin, depth + 1);
                };
                return self.register_type_at(position, source, depth + 1);
            }
            let iterator_fact = self.fact(producer, iterator);
            let descriptor = self
                .origin_type(iterator_fact.origin, depth + 1)
                .or_else(|| self.foreach_iterator_type(producer, iterator, depth + 1))?;
            let descriptor = self.expand_aliases_owned(descriptor);
            return traversed_component(&descriptor, key, depth + 1);
        }

        if let Some((position, source)) = self.moved_register_source(index, register) {
            return self.register_type_at(position, source, depth + 1);
        }

        self.origin_type_matching_mask(fact, depth + 1)
    }

    fn origin_type_matching_mask(&self, fact: Fact, depth: usize) -> Option<TypeDescriptor> {
        let descriptor = self.origin_type(fact.origin, depth + 1)?;
        let descriptor = self.expand_aliases_owned(descriptor);

        self.descriptor_matching_mask(descriptor, fact.mask)
    }

    fn descriptor_matching_mask(
        &self,
        descriptor: TypeDescriptor,
        mask: u16,
    ) -> Option<TypeDescriptor> {
        let TypeDescriptor::Union(members) = descriptor else {
            return self
                .descriptor_mask_for_refinement(&descriptor)
                .is_none_or(|descriptor| descriptor & mask != 0)
                .then_some(descriptor);
        };
        let members = members
            .into_iter()
            .filter(|member| {
                self.descriptor_mask_for_refinement(member)
                    .is_none_or(|member| member & mask != 0)
            })
            .collect();
        Some(union_or_never(members))
    }

    fn descriptor_mask_for_refinement(&self, descriptor: &TypeDescriptor) -> Option<u16> {
        self.unit
            .and_then(|unit| unit.descriptor_mask(descriptor, 0))
            .or_else(|| descriptor_mask(descriptor))
    }

    fn moved_register_source(&self, index: usize, register: Register) -> Option<(usize, Register)> {
        for previous in (0..index).rev() {
            let instruction = self.chunk.code[previous];
            match instruction {
                Instruction::Move {
                    destination,
                    source,
                }
                | Instruction::MoveOwned {
                    destination,
                    source,
                } if destination == register => {
                    return dominates(self.chunk, previous, index).then_some((previous, source));
                }
                _ if effect_on(self.chunk, instruction, register).writes() => return None,
                _ => {}
            }
        }

        None
    }

    fn foreach_iterator_type(
        &self,
        index: usize,
        iterator: Register,
        depth: usize,
    ) -> Option<TypeDescriptor> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        for previous in (0..index).rev() {
            let instruction = self.chunk.code[previous];
            if let Instruction::ForeachInit {
                iterator: candidate,
                subject,
                ..
            } = instruction
                && candidate == iterator
            {
                return dominates(self.chunk, previous, index)
                    .then(|| self.register_type_at(previous, subject, depth + 1))?;
            }
            if effect_on(self.chunk, instruction, iterator).writes() {
                return None;
            }
        }

        None
    }

    pub(in crate::optimizer::type_flow) fn precise_result(
        &self,
        index: usize,
    ) -> Option<(Register, Fact)> {
        let origin = index as u32 + 1;
        let (destination, descriptor) = match self.chunk.code.get(index)? {
            Instruction::AsCheck {
                destination,
                descriptor,
                ..
            } => (
                *destination,
                &self.chunk.type_descriptors[usize::from(descriptor.index())],
            ),
            Instruction::AsOrNull {
                destination,
                descriptor,
                ..
            } => {
                let descriptor = &self.chunk.type_descriptors[usize::from(descriptor.index())];
                let descriptor = self.expanded_aliases(descriptor);
                let mut fact = self.descriptor_fact(&descriptor, origin);
                fact.mask |= NULL;
                return Some((*destination, fact));
            }
            Instruction::PropertyGet {
                destination,
                object,
                cache,
            } => {
                let property = self.resolved_property(index, *object, *cache)?.property;
                (*destination, property.declared_type.as_ref()?)
            }
            Instruction::PropertyGetUnchecked {
                destination,
                object,
                slot,
                ..
            } => {
                let resolved = self.property_class_specialization(self.fact(index, *object), 0)?;
                let (_, property) = *self
                    .flattened_layout(resolved.class)?
                    .get(usize::from(slot.index()))?;
                (*destination, property.declared_type.as_ref()?)
            }
            Instruction::ConstantGet { destination, .. } => {
                let descriptor = self.origin_type(origin, 0)?;
                return Some((*destination, self.descriptor_fact(&descriptor, origin)));
            }
            Instruction::ClassConstantGet { destination, cache } => {
                let descriptor = self.class_constant_type(*cache)?;
                return Some((*destination, self.descriptor_fact(&descriptor, origin)));
            }
            Instruction::CallValue {
                destination,
                callee,
                ..
            }
            | Instruction::CallValueUnchecked {
                destination,
                callee,
                ..
            } => {
                let fact = self.fact(index, *callee);
                let descriptor = self.origin_type(fact.origin, 0)?;
                let descriptor = self.expand_aliases_owned(descriptor);
                let signature = callable_signature(&descriptor)?;
                return Some((
                    *destination,
                    self.descriptor_fact(&signature.return_type, origin),
                ));
            }
            Instruction::CallMethod { destination, .. }
            | Instruction::CallMethodUnchecked { destination, .. }
            | Instruction::CallMethodDirect { destination, .. } => {
                let descriptor = self.method_return_type_at(index, 0)?;
                return Some((*destination, self.descriptor_fact(&descriptor, origin)));
            }
            Instruction::CallNamed { destination, .. }
            | Instruction::CallNamedUnchecked { destination, .. }
            | Instruction::CallNamedConstantUnchecked { destination, .. }
            | Instruction::CallSelfUnchecked { destination, .. } => {
                if let Some(function) = self.resolved_function(index) {
                    (*destination, function.return_type.as_ref()?)
                } else if let Some(function) = self.resolved_built_in_function(index) {
                    let descriptor = substitute_parameters(
                        &function.return_type,
                        &function.type_parameters,
                        self.call_type_arguments(index),
                        0,
                    );
                    let descriptor = self.expand_aliases_owned(descriptor);

                    return Some((*destination, self.descriptor_fact(&descriptor, origin)));
                } else {
                    let descriptor = self.newtype_call_type(index)?;
                    return Some((*destination, self.descriptor_fact(&descriptor, origin)));
                }
            }
            Instruction::StringIndexGet { destination, .. } => {
                return Some((*destination, Fact::with_origin(STRING, origin)));
            }
            Instruction::IndexGet {
                destination,
                container,
                ..
            }
            | Instruction::VecIndexGet {
                destination,
                container,
                ..
            }
            | Instruction::DictIndexGetIntKey {
                destination,
                container,
                ..
            }
            | Instruction::DictIndexGetStringKey {
                destination,
                container,
                ..
            } => {
                let fact = self.fact(index, *container);
                if let Some(container) = self.origin_type(fact.origin, 0) {
                    let descriptor = match container {
                        TypeDescriptor::Array(Some((_, value)))
                        | TypeDescriptor::Dictionary(Some((_, value))) => *value,
                        TypeDescriptor::Vector(Some(element)) => *element,
                        _ => return None,
                    };
                    return Some((*destination, self.descriptor_fact(&descriptor, origin)));
                }
                if fact.array != 0 {
                    let mask = self
                        .array_elements
                        .get(fact.array as usize)
                        .copied()
                        .unwrap_or(0);
                    if mask != 0 {
                        return Some((*destination, Fact::with_origin(mask, origin)));
                    }
                }
                return None;
            }
            _ => return None,
        };
        let descriptor = self.expanded_aliases(descriptor);
        let mut fact = self.descriptor_fact(&descriptor, origin);
        if self
            .array_elements
            .get(origin as usize)
            .is_some_and(|elements| *elements != 0)
            || self
                .array_keys
                .get(origin as usize)
                .is_some_and(|keys| *keys != 0)
        {
            fact.array = origin;
        }
        Some((destination, fact))
    }

    pub(in crate::optimizer) fn resolved_function(
        &self,
        index: usize,
    ) -> Option<&'a CompiledFunction> {
        let instruction = *self.chunk.code.get(index)?;
        if matches!(instruction, Instruction::CallSelfUnchecked { .. }) {
            return self
                .unit?
                .functions
                .iter()
                .find(|function| ptr::eq(&raw const function.chunk, self.chunk));
        }

        let cache = named_call_cache(instruction)?;
        let (name, _) = self.member_descriptor(cache)?;
        self.unit?.function_by_name(name)
    }

    pub(in crate::optimizer) fn resolved_built_in_function(
        &self,
        index: usize,
    ) -> Option<&'a CompiledBuiltInFunction> {
        let cache = named_call_cache(*self.chunk.code.get(index)?)?;
        let name = self.member_name(cache)?;
        self.unit?.built_in_function_by_name(name)
    }

    fn call_type_arguments(&self, index: usize) -> Option<&'a [TypeDescriptor]> {
        let cache = named_call_cache(*self.chunk.code.get(index)?)?;
        self.member_descriptor(cache)?.1
    }

    pub(in crate::optimizer) fn resolved_property(
        &self,
        index: usize,
        object: Register,
        cache: IcSlot,
    ) -> Option<ResolvedProperty<'a>> {
        if index >= self.chunk.code.len() || !self.reachable[index] {
            return None;
        }
        let class = self
            .property_class_specialization(self.fact(index, object), 0)?
            .class;
        let name = self.member_name(cache)?;
        self.instance_slot_of(class, name)
    }

    fn property_class_specialization(&self, fact: Fact, depth: usize) -> Option<ExactClass<'a>> {
        if fact.origin != THIS_ORIGIN {
            return self.exact_class_specialization(fact, depth);
        }

        let class = self.class(self.class_name?)?;
        Some(ExactClass {
            class,
            arguments: (!self.class_type_parameters.is_empty()).then(|| {
                self.class_type_parameters
                    .iter()
                    .map(|parameter| TypeDescriptor::Parameter(parameter.name.clone()))
                    .collect()
            }),
        })
    }

    /// Resolves a property name to its slot in `class`'s flattened instance
    /// layout.
    fn instance_slot_of(
        &self,
        class: &'a CompiledClassLike,
        name: &Atom,
    ) -> Option<ResolvedProperty<'a>> {
        let layout = self.flattened_layout(class)?;
        // Last match wins: a private redeclaration appends and takes the name.
        let slot = layout
            .iter()
            .rposition(|(_, property)| same_atom(&property.name, name))?;
        let (owner, property) = layout[slot];
        Some(ResolvedProperty {
            class: owner,
            property,
            slot: u16::try_from(slot).ok()?,
        })
    }

    /// The class and property occupying each slot of `class`'s flattened
    /// instance layout, in slot order.
    fn flattened_layout(
        &self,
        class: &'a CompiledClassLike,
    ) -> Option<Vec<(&'a CompiledClassLike, &'a CompiledProperty)>> {
        let mut chain = Vec::new();
        let mut current = class;
        loop {
            if chain.len() > MAX_TYPE_DEPTH {
                return None;
            }
            chain.push(current);
            let Some(parent) = &current.parent else {
                break;
            };
            current = self.class(&parent.name)?;
        }
        chain.reverse();

        let mut names: Vec<(&Atom, u32)> = Vec::new();
        let mut occupants: Vec<(&'a CompiledClassLike, &'a CompiledProperty)> = Vec::new();
        for owner in chain {
            for property in &owner.properties {
                if property.is_static {
                    continue;
                }
                let inherited = names
                    .iter()
                    .find(|(existing, _)| same_atom(existing, &property.name))
                    .and_then(|(_, slot)| {
                        occupants
                            .get(*slot as usize)
                            .map(|(_, inherited)| (*slot, inherited.visibility))
                    });
                match slot_placement(inherited, property.visibility) {
                    SlotPlacement::Inherited(slot) => {
                        *occupants.get_mut(slot as usize)? = (owner, property);
                    }
                    SlotPlacement::Appended => {
                        let slot = u32::try_from(occupants.len()).ok()?;
                        occupants.push((owner, property));
                        match names
                            .iter_mut()
                            .find(|(existing, _)| same_atom(existing, &property.name))
                        {
                            Some(entry) => entry.1 = slot,
                            None => names.push((&property.name, slot)),
                        }
                    }
                }
            }
        }
        Some(occupants)
    }

    pub(in crate::optimizer) fn origin_type(
        &self,
        origin: u32,
        depth: usize,
    ) -> Option<TypeDescriptor> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        if origin & PARAMETER_ORIGIN != 0 && origin != THIS_ORIGIN {
            let index = (origin & !PARAMETER_ORIGIN) as usize;
            return self.parameters.get(index)?.declared_type.clone();
        }
        if origin & CAPTURE_ORIGIN != 0 && origin != THIS_ORIGIN {
            let index = (origin & !CAPTURE_ORIGIN) as usize;
            return self.capture_types.get(index)?.clone();
        }
        if origin == THIS_ORIGIN {
            return Some(TypeDescriptor::Named {
                name: self.class_name?.clone(),
                arguments: (!self.class_type_parameters.is_empty()).then(|| {
                    self.class_type_parameters
                        .iter()
                        .map(|parameter| TypeDescriptor::Parameter(parameter.name.clone()))
                        .collect()
                }),
                recursive: false,
            });
        }
        let index = instruction_index(origin)?;
        match self.chunk.code[index] {
            Instruction::NewStatic { cache, .. } => self.member_type_descriptor(cache),
            Instruction::InitializeProperties {
                cache, descriptor, ..
            } if self
                .chunk
                .property_initialization_descriptor(descriptor)
                .allocates =>
            {
                self.member_type_descriptor(cache)
            }
            Instruction::MakeClosure { prototype, .. } => {
                let Literal::String(name) =
                    self.chunk.constants.get(usize::from(prototype.index()))?
                else {
                    return None;
                };
                let function = self.unit?.function_by_name(name)?;
                Some(TypeDescriptor::Callable(Some(FunctionTypeDescriptor {
                    parameters: function
                        .parameters
                        .iter()
                        .map(|parameter| FunctionTypeParameterDescriptor {
                            r#type: parameter
                                .declared_type
                                .clone()
                                .unwrap_or(TypeDescriptor::Mixed),
                            optional: parameter.has_default,
                        })
                        .collect(),
                    return_type: Box::new(
                        function
                            .return_type
                            .clone()
                            .unwrap_or(TypeDescriptor::Mixed),
                    ),
                })))
            }
            Instruction::ConstantGet { cache, .. } => {
                let constant = self.unit?.constant_by_name(self.member_name(cache)?)?;
                let ConstantInitializer::Literal(literal) = &constant.initializer else {
                    return None;
                };
                Some(literal_type(literal))
            }
            Instruction::ClassConstantGet { cache, .. } => self.class_constant_type(cache),
            Instruction::IndexGet {
                container,
                index: key,
                ..
            } => {
                let container = self.register_type_at(index, container, depth + 1)?;
                let ConstantValue::Int(key) =
                    self.constant_value_fact(self.fact(index, key), depth + 1)?
                else {
                    return None;
                };
                Self::indexed_descriptor(&container, key).cloned()
            }
            Instruction::ElementGet {
                subject,
                index: key,
                ..
            } => {
                let container = self.register_type_at(index, subject, depth + 1)?;
                Self::indexed_descriptor(&container, i64::from(key.value())).cloned()
            }
            Instruction::PropertyGet { object, cache, .. } => {
                let resolved =
                    self.property_class_specialization(self.fact(index, object), depth + 1)?;
                let name = self.member_name(cache)?;
                let property = self.instance_slot_of(resolved.class, name)?.property;
                Some(substitute_parameters(
                    property.declared_type.as_ref()?,
                    &resolved.class.type_parameters,
                    resolved.arguments.as_deref(),
                    depth + 1,
                ))
            }
            Instruction::PropertyGetUnchecked { object, slot, .. } => {
                let resolved =
                    self.exact_class_specialization(self.fact(index, object), depth + 1)?;
                // The slot indexes the flattened layout, not the class's own
                // declarations.
                let layout = self.flattened_layout(resolved.class)?;
                let (_, property) = *layout.get(usize::from(slot.index()))?;
                Some(substitute_parameters(
                    property.declared_type.as_ref()?,
                    &resolved.class.type_parameters,
                    resolved.arguments.as_deref(),
                    depth + 1,
                ))
            }
            Instruction::CallValue { callee, .. }
            | Instruction::CallValueDiscarded { callee, .. }
            | Instruction::CallValueUnchecked { callee, .. } => {
                let descriptor = self.origin_type(self.fact(index, callee).origin, depth + 1)?;
                let descriptor = self.expand_aliases_owned(descriptor);

                Some(*callable_signature(&descriptor)?.return_type)
            }
            Instruction::CallMethod { .. }
            | Instruction::CallMethodDiscarded { .. }
            | Instruction::CallMethodUnchecked { .. }
            | Instruction::CallMethodDirect { .. } => self.method_return_type_at(index, depth + 1),
            Instruction::CallNamed { .. }
            | Instruction::CallNamedDiscarded { .. }
            | Instruction::CallNamedUnchecked { .. }
            | Instruction::CallNamedConstantUnchecked { .. }
            | Instruction::CallSelfUnchecked { .. } => {
                if let Some(function) = self.resolved_function(index) {
                    Some(substitute_parameters(
                        function.return_type.as_ref()?,
                        &function.type_parameters,
                        self.call_type_arguments(index),
                        depth + 1,
                    ))
                } else if let Some(function) = self.resolved_built_in_function(index) {
                    Some(substitute_parameters(
                        &function.return_type,
                        &function.type_parameters,
                        self.call_type_arguments(index),
                        depth + 1,
                    ))
                } else {
                    self.newtype_call_type(index)
                }
            }
            Instruction::CloneObject { source, .. } => {
                self.origin_type(self.fact(index, source).origin, depth + 1)
            }
            _ => self.origin_descriptor(origin, depth + 1).cloned(),
        }
    }

    pub(in crate::optimizer) fn origin_descriptor(
        &self,
        origin: u32,
        depth: usize,
    ) -> Option<&TypeDescriptor> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        if origin & PARAMETER_ORIGIN != 0 && origin != THIS_ORIGIN {
            let index = (origin & !PARAMETER_ORIGIN) as usize;
            return self.parameters.get(index)?.declared_type.as_ref();
        }
        if origin & CAPTURE_ORIGIN != 0 && origin != THIS_ORIGIN {
            let index = (origin & !CAPTURE_ORIGIN) as usize;
            return self.capture_types.get(index)?.as_ref();
        }
        let index = instruction_index(origin)?;
        match self.chunk.code[index] {
            Instruction::AsCheck { descriptor, .. } | Instruction::NewTyped { descriptor, .. } => {
                self.chunk
                    .type_descriptors
                    .get(usize::from(descriptor.index()))
            }
            Instruction::PropertyGet { object, cache, .. } => self
                .resolved_property(index, object, cache)?
                .property
                .declared_type
                .as_ref(),
            Instruction::PropertyGetUnchecked { object, slot, .. } => {
                let resolved =
                    self.property_class_specialization(self.fact(index, object), depth)?;
                self.flattened_layout(resolved.class)?
                    .get(usize::from(slot.index()))?
                    .1
                    .declared_type
                    .as_ref()
            }
            Instruction::CallMethod { .. }
            | Instruction::CallMethodDiscarded { .. }
            | Instruction::CallMethodUnchecked { .. }
            | Instruction::CallMethodDirect { .. } => self
                .resolved_method_at(index, depth)?
                .function
                .return_type
                .as_ref(),
            Instruction::IndexGet {
                container,
                index: key,
                ..
            } => {
                let container =
                    self.origin_descriptor(self.fact(index, container).origin, depth + 1)?;
                let ConstantValue::Int(key) =
                    self.constant_value_fact(self.fact(index, key), depth + 1)?
                else {
                    return None;
                };
                Self::indexed_descriptor(container, key)
            }
            Instruction::VecIndexGet { container, .. }
            | Instruction::DictIndexGetIntKey { container, .. }
            | Instruction::DictIndexGetStringKey { container, .. } => {
                let container =
                    self.origin_descriptor(self.fact(index, container).origin, depth + 1)?;
                match container {
                    TypeDescriptor::Vector(Some(element)) => Some(element),
                    TypeDescriptor::Dictionary(Some((_, value))) => Some(value),
                    _ => None,
                }
            }
            Instruction::ElementGet {
                subject,
                index: key,
                ..
            } => {
                let container =
                    self.origin_descriptor(self.fact(index, subject).origin, depth + 1)?;
                Self::indexed_descriptor(container, i64::from(key.value()))
            }
            Instruction::CloneObject { source, .. } => {
                self.origin_descriptor(self.fact(index, source).origin, depth + 1)
            }
            _ => None,
        }
    }

    fn newtype_call_type(&self, index: usize) -> Option<TypeDescriptor> {
        let cache = named_call_cache(*self.chunk.code.get(index)?)?;
        let name = self.member_name(cache)?;
        self.unit?.newtype_by_name(name)?;
        Some(TypeDescriptor::Named {
            name: name.clone(),
            arguments: self.call_type_arguments(index).map(<[_]>::to_vec),
            recursive: false,
        })
    }

    pub(in crate::optimizer) fn indexed_descriptor(
        container: &TypeDescriptor,
        key: i64,
    ) -> Option<&TypeDescriptor> {
        match container {
            TypeDescriptor::Tuple(members) => members.get(usize::try_from(key).ok()?),
            TypeDescriptor::TupleRest { elements, rest } => {
                let key = usize::try_from(key).ok()?;
                Some(elements.get(key).unwrap_or(rest))
            }
            TypeDescriptor::Array(Some((_, value)))
            | TypeDescriptor::Dictionary(Some((_, value))) => Some(value),
            TypeDescriptor::Vector(Some(element)) => Some(element),
            _ => None,
        }
    }

    pub(in crate::optimizer::type_flow) fn exact_class(
        &self,
        fact: Fact,
        depth: usize,
    ) -> Option<&'a CompiledClassLike> {
        Some(self.exact_class_specialization(fact, depth)?.class)
    }

    pub(in crate::optimizer::type_flow) fn exact_class_specialization(
        &self,
        fact: Fact,
        depth: usize,
    ) -> Option<ExactClass<'a>> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        if fact.origin == THIS_ORIGIN {
            return Some(ExactClass {
                class: self.final_class(self.class_name?)?,
                arguments: None,
            });
        }
        if let Some(index) = instruction_index(fact.origin) {
            if let Instruction::NewStatic { cache, .. } = self.chunk.code[index]
                && let Some((name, type_arguments)) = self.member_descriptor(cache)
            {
                return Some(ExactClass {
                    class: self.class(name)?,
                    arguments: type_arguments.map(<[TypeDescriptor]>::to_vec),
                });
            }
            if let Some((first_argument, _)) = method_call_site(self.chunk.code[index])
                && self
                    .resolved_method_at(index, depth + 1)
                    .and_then(|method| method.function.return_type.as_ref())
                    .is_some_and(|descriptor| matches!(descriptor, TypeDescriptor::StaticClass))
            {
                return self
                    .exact_class_specialization(self.fact(index, first_argument), depth + 1);
            }
        }
        let descriptor = self.origin_type_matching_mask(fact, depth + 1)?;
        match descriptor {
            TypeDescriptor::Named {
                name, arguments, ..
            } => Some(ExactClass {
                class: self.final_class(&name)?,
                arguments,
            }),
            TypeDescriptor::StaticClass => Some(ExactClass {
                class: self.final_class(self.class_name?)?,
                arguments: None,
            }),
            _ => None,
        }
    }

    pub(in crate::optimizer) fn class(&self, name: &Atom) -> Option<&'a CompiledClassLike> {
        self.class_like(name)
            .filter(|class| class.kind == ClassLikeKind::Class)
    }

    pub(in crate::optimizer) fn class_like(&self, name: &Atom) -> Option<&'a CompiledClassLike> {
        self.unit?.class_by_name(name)
    }

    pub(in crate::optimizer) fn final_class(&self, name: &Atom) -> Option<&'a CompiledClassLike> {
        self.class(name).filter(|class| class.is_final)
    }

    pub(in crate::optimizer) fn resolved_method_at(
        &self,
        index: usize,
        depth: usize,
    ) -> Option<&'a CompiledMethod> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        let (first_argument, cache) = method_call_site(*self.chunk.code.get(index)?)?;
        let class = self.exact_class(self.fact(index, first_argument), depth + 1)?;
        let name = self.member_name(cache)?;
        class.methods.iter().find(|method| {
            !method.is_static
                && !method.is_abstract
                && method.function.type_parameters.is_empty()
                && same_atom(&method.name, name)
        })
    }

    pub(in crate::optimizer) fn method_return_type_at(
        &self,
        index: usize,
        depth: usize,
    ) -> Option<TypeDescriptor> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        let (first_argument, cache) = method_call_site(*self.chunk.code.get(index)?)?;
        let receiver = self.register_type_at(index, first_argument, depth + 1)?;
        let receiver = self.expand_aliases_owned(receiver);
        let TypeDescriptor::Named {
            name, arguments, ..
        } = receiver
        else {
            return None;
        };
        let class = self.class_like(&name)?;
        let (member, type_arguments) = self.member_descriptor(cache)?;
        let method = class
            .methods
            .iter()
            .find(|method| !method.is_static && same_atom(&method.name, member))?;
        if method.function.type_parameters.is_empty() {
            if type_arguments.is_some_and(|arguments| !arguments.is_empty()) {
                return None;
            }
        } else if type_arguments
            .is_none_or(|arguments| arguments.len() != method.function.type_parameters.len())
        {
            return None;
        }
        let descriptor = substitute_parameters(
            method.function.return_type.as_ref()?,
            &method.function.type_parameters,
            type_arguments,
            depth + 1,
        );
        let descriptor = substitute_parameters(
            &descriptor,
            &class.type_parameters,
            arguments.as_deref(),
            depth + 1,
        );
        Some(self.expand_aliases_owned(descriptor))
    }

    pub(in crate::optimizer) fn member_name(&self, cache: IcSlot) -> Option<&'a Atom> {
        Some(self.member_descriptor(cache)?.0)
    }

    pub(super) fn member_descriptor(
        &self,
        cache: IcSlot,
    ) -> Option<(&'a Atom, Option<&'a [TypeDescriptor]>)> {
        let IcDescriptor::Member {
            name,
            type_arguments,
        } = self.chunk.ic_descriptors.get(usize::from(cache.index()))?
        else {
            return None;
        };
        Some((name, type_arguments.as_deref()))
    }

    fn member_type_descriptor(&self, cache: IcSlot) -> Option<TypeDescriptor> {
        let (name, arguments) = self.member_descriptor(cache)?;
        Some(TypeDescriptor::Named {
            name: name.clone(),
            arguments: arguments.map(<[TypeDescriptor]>::to_vec),
            recursive: false,
        })
    }

    fn class_constant_type(&self, cache: IcSlot) -> Option<TypeDescriptor> {
        let IcDescriptor::ClassMember {
            class,
            member,
            type_arguments,
        } = self.chunk.ic_descriptors.get(usize::from(cache.index()))?
        else {
            return None;
        };
        if type_arguments
            .as_ref()
            .is_some_and(|arguments| !arguments.is_empty())
        {
            return None;
        }
        let class_like = self.class_like(class)?;
        if class_like.kind == ClassLikeKind::Enum
            && class_like
                .cases
                .iter()
                .any(|case| same_atom(&case.name, member))
        {
            return Some(TypeDescriptor::Named {
                name: class.clone(),
                arguments: None,
                recursive: false,
            });
        }
        class_like
            .constants
            .iter()
            .find(|constant| same_atom(&constant.name, member))?
            .declared_type
            .clone()
    }

    pub(in crate::optimizer::type_flow) fn equivalent_class_constant_origins(
        &self,
        left: u32,
        right: u32,
    ) -> bool {
        let left = instruction_index(left).and_then(|index| {
            let Instruction::ClassConstantGet { cache, .. } = self.chunk.code[index] else {
                return None;
            };
            self.class_constant_type(cache)
        });
        let right = instruction_index(right).and_then(|index| {
            let Instruction::ClassConstantGet { cache, .. } = self.chunk.code[index] else {
                return None;
            };
            self.class_constant_type(cache)
        });
        left.zip(right)
            .is_some_and(|(left, right)| descriptors_equal(&left, &right, 0))
    }

    pub(in crate::optimizer::type_flow) fn equivalent_final_class_origins(
        &self,
        left: u32,
        right: u32,
    ) -> bool {
        let Some(mut left) = self.origin_type(left, 0) else {
            return false;
        };
        let Some(mut right) = self.origin_type(right, 0) else {
            return false;
        };
        left = self.expand_aliases_owned(left);
        right = self.expand_aliases_owned(right);
        let TypeDescriptor::Named { name, .. } = &left else {
            return false;
        };
        self.final_class(name).is_some() && descriptors_equal(&left, &right, 0)
    }

    pub(in crate::optimizer) fn this_arguments_match(
        &self,
        arguments: Option<&[TypeDescriptor]>,
    ) -> bool {
        if self.class_type_parameters.is_empty() {
            return arguments.is_none_or(<[TypeDescriptor]>::is_empty);
        }
        let Some(arguments) = arguments else {
            return false;
        };
        arguments.len() == self.class_type_parameters.len()
            && arguments
                .iter()
                .zip(self.class_type_parameters)
                .all(|(argument, parameter)| {
                    matches!(argument, TypeDescriptor::Parameter(name) if same_atom(name, &parameter.name))
                })
    }
}

fn method_call_site(instruction: Instruction) -> Option<(Register, IcSlot)> {
    match instruction {
        Instruction::CallMethod {
            first_argument,
            cache,
            ..
        }
        | Instruction::CallMethodDiscarded {
            first_argument,
            cache,
            ..
        }
        | Instruction::CallMethodUnchecked {
            first_argument,
            cache,
            ..
        }
        | Instruction::CallMethodDirect {
            first_argument,
            cache,
            ..
        } => Some((first_argument, cache)),
        _ => None,
    }
}

fn named_call_cache(instruction: Instruction) -> Option<IcSlot> {
    match instruction {
        Instruction::CallNamed { cache, .. }
        | Instruction::CallNamedDiscarded { cache, .. }
        | Instruction::CallNamedUnchecked { cache, .. }
        | Instruction::CallNamedConstantUnchecked { cache, .. } => Some(cache),
        _ => None,
    }
}

fn traversed_component(
    descriptor: &TypeDescriptor,
    key: bool,
    depth: usize,
) -> Option<TypeDescriptor> {
    if depth > MAX_TYPE_DEPTH {
        return None;
    }
    match descriptor {
        TypeDescriptor::Array(Some((key_type, value_type)))
        | TypeDescriptor::Dictionary(Some((key_type, value_type))) => Some(if key {
            key_type.as_ref().clone()
        } else {
            value_type.as_ref().clone()
        }),
        TypeDescriptor::Vector(Some(value)) => Some(if key {
            TypeDescriptor::integer_range(Some(0), None)
        } else {
            value.as_ref().clone()
        }),
        TypeDescriptor::Tuple(members) => {
            if key {
                return Some(if members.is_empty() {
                    TypeDescriptor::Never
                } else {
                    TypeDescriptor::integer_range(Some(0), Some(members.len() as i64 - 1))
                });
            }
            Some(union_or_never(members.clone()))
        }
        TypeDescriptor::TupleRest { elements, rest } => {
            if key {
                return Some(TypeDescriptor::integer_range(Some(0), None));
            }
            let mut members = elements.clone();
            members.push(rest.as_ref().clone());
            Some(union_or_never(members))
        }
        TypeDescriptor::Named {
            name,
            arguments: Some(arguments),
            ..
        } if (name.as_bytes() == b"Whim\\Iterate\\Iterator"
            || name.as_bytes() == b"Whim\\Iterate\\ToIterator")
            && arguments.len() == 2 =>
        {
            Some(arguments[usize::from(!key)].clone())
        }
        TypeDescriptor::Union(members) => {
            let mut components = Vec::with_capacity(members.len());
            for member in members {
                components.push(traversed_component(member, key, depth + 1)?);
            }
            Some(union_or_never(components))
        }
        TypeDescriptor::Intersection(members) => members
            .iter()
            .find_map(|member| traversed_component(member, key, depth + 1)),
        _ => None,
    }
}

fn union_or_never(members: Vec<TypeDescriptor>) -> TypeDescriptor {
    let mut unique = Vec::with_capacity(members.len());
    for member in members {
        if !unique
            .iter()
            .any(|existing| descriptors_equal(existing, &member, 0))
        {
            unique.push(member);
        }
    }

    match unique.len() {
        0 => TypeDescriptor::Never,
        1 => unique.pop().unwrap_or(TypeDescriptor::Never),
        _ => TypeDescriptor::Union(unique),
    }
}

fn literal_type(literal: &Literal) -> TypeDescriptor {
    match literal {
        Literal::Null => TypeDescriptor::Null,
        Literal::Bool(true) => TypeDescriptor::TrueLiteral,
        Literal::Bool(false) => TypeDescriptor::FalseLiteral,
        Literal::Int(value) => TypeDescriptor::IntLiteral(*value),
        Literal::Float(value) => TypeDescriptor::FloatLiteral(*value),
        Literal::String(value) => TypeDescriptor::StringLiteral(value.clone()),
    }
}
