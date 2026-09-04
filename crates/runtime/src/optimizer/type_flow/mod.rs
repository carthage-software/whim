//! Conservative forward type facts shared by optimization passes.

use std::borrow::Cow;
use std::cell::Cell;
use std::cell::RefCell;

use std::cmp::Ordering;
use std::ptr;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::DictionaryTypeDescriptor;
use crate::bytecode::chunk::descriptors::FunctionTypeDescriptor;
use crate::bytecode::chunk::descriptors::FunctionTypeParameterDescriptor;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::Comparison as BytecodeComparison;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledMethod;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledProperty;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::optimizer::cfg::branches_or_terminates;
use crate::optimizer::cfg::successors;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::operands::for_each_register;
use crate::optimizer::operands::for_each_write_register;
use crate::optimizer::type_flow::descriptors::descriptor_mask;
use crate::optimizer::type_flow::descriptors::descriptor_may_release_observably;
use crate::optimizer::type_flow::transfer::numeric_result;
use crate::optimizer::type_flow::transfer::transfer;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

mod constants;
mod descriptors;
mod proofs;
mod resolve;
mod string_lengths;
mod transfer;
mod world;

pub(in crate::optimizer) use descriptors::descriptor_options_equal;
pub(crate) use descriptors::descriptor_proves;
pub(crate) use descriptors::descriptors_equal;
pub(in crate::optimizer) use world::IndexedUnit;
pub(crate) use world::World;
pub(crate) use world::WorldCache;

const NULL: u16 = 1 << 0;
const BOOL: u16 = 1 << 1;
const INT: u16 = 1 << 2;
const FLOAT: u16 = 1 << 3;
const STRING: u16 = 1 << 4;
const OBJECT: u16 = 1 << 5;
const VECTOR: u16 = 1 << 6;
const DICTIONARY: u16 = 1 << 7;
const TUPLE: u16 = 1 << 8;
const CALLABLE: u16 = 1 << 9;
const ALL: u16 = u16::MAX;
const NUMERIC: u16 = INT | FLOAT;
const ALWAYS_REFERENCE_COUNTED: u16 = OBJECT | VECTOR | DICTIONARY | TUPLE | CALLABLE;
const MAY_BE_REFERENCE_COUNTED: u16 = STRING | OBJECT | VECTOR | DICTIONARY | TUPLE | CALLABLE;
const NO_ORIGIN: u32 = 0;
const CAPTURE_ORIGIN: u32 = 1 << 30;
const PARAMETER_ORIGIN: u32 = 1 << 31;
const EXTERNAL_ORIGIN: u32 = CAPTURE_ORIGIN | PARAMETER_ORIGIN;
const THIS_ORIGIN: u32 = u32::MAX;

/// One register's facts before one instruction.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Fact {
    mask: u16,
    observable_release: bool,
    non_negative: bool,
    positive: bool,
    origin: u32,
    array: u32,
}

#[derive(Clone)]
pub(in crate::optimizer) enum ConstantValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Atom),
}

impl Fact {
    const UNKNOWN: Self = Self {
        mask: ALL,
        origin: NO_ORIGIN,
        array: NO_ORIGIN,
        observable_release: true,
        non_negative: false,
        positive: false,
    };

    const fn known(mask: u16) -> Self {
        Self {
            mask,
            origin: NO_ORIGIN,
            array: NO_ORIGIN,
            observable_release: mask & (OBJECT | VECTOR | DICTIONARY | TUPLE | CALLABLE) != 0,
            non_negative: false,
            positive: false,
        }
    }

    const fn with_origin(mask: u16, origin: u32) -> Self {
        Self {
            mask,
            origin,
            array: NO_ORIGIN,
            observable_release: mask & (OBJECT | VECTOR | DICTIONARY | TUPLE | CALLABLE) != 0,
            non_negative: false,
            positive: false,
        }
    }

    const fn array(mask: u16, identity: u32, observable_release: bool) -> Self {
        Self {
            mask,
            origin: identity,
            array: identity,
            observable_release,
            non_negative: false,
            positive: false,
        }
    }

    const fn integer(value: i64, origin: u32) -> Self {
        Self {
            mask: INT,
            origin,
            array: NO_ORIGIN,
            observable_release: false,
            non_negative: value >= 0,
            positive: value > 0,
        }
    }

    fn from_value(value: &Value) -> Self {
        match value.transparent() {
            ValueView::Uninitialized => Self::known(0),
            ValueView::Null => Self::known(NULL),
            ValueView::Bool(_) => Self::known(BOOL),
            ValueView::Int(value) => Self::integer(*value, NO_ORIGIN),
            ValueView::Float(_) => Self::known(FLOAT),
            ValueView::String(_) | ValueView::ShortString(_) => Self::known(STRING),
            ValueView::Object(_) => Self::known(OBJECT),
            ValueView::Vec(_) => Self::known(VECTOR),
            ValueView::Dict(_) => Self::known(DICTIONARY),
            ValueView::Tuple(_) => Self::known(TUPLE),
            ValueView::Function(_) => Self::known(CALLABLE),
            ValueView::Iter(_) => Self::UNKNOWN,
        }
    }

    const fn without_origin(self) -> Self {
        Self {
            mask: self.mask,
            origin: NO_ORIGIN,
            array: self.array,
            observable_release: self.observable_release,
            non_negative: self.non_negative,
            positive: self.positive,
        }
    }

    const fn release_is_unobservable(mut self) -> Self {
        self.observable_release = false;
        self
    }

    fn merge(self, other: Self) -> Self {
        Self {
            mask: self.mask | other.mask,
            origin: if self.origin == other.origin {
                self.origin
            } else {
                NO_ORIGIN
            },
            array: if self.array == other.array {
                self.array
            } else {
                NO_ORIGIN
            },
            observable_release: self.observable_release || other.observable_release,
            non_negative: self.non_negative && other.non_negative,
            positive: self.positive && other.positive,
        }
    }
}

/// The largest control-flow state table an analysis will build: basic-block
/// entries times registers. Instruction facts are stored only for operands.
const MAXIMUM_FLOW_STATES: usize = 1 << 21;

type MemoizedConstant = Option<Option<(Register, ConstantValue)>>;

pub(in crate::optimizer) struct TypeFlow<'a> {
    chunk: &'a Chunk,
    parameters: &'a [CompiledParameter],
    capture_types: Vec<Option<TypeDescriptor>>,
    class_name: Option<&'a Atom>,
    class_type_parameters: &'a [CompiledTypeParameter],
    unit: Option<&'a IndexedUnit<'a>>,
    allocator: &'a Heap,
    facts: Vec<Fact>,
    fact_offsets: Vec<usize>,
    fact_registers: Vec<Register>,
    block_states: Vec<Fact>,
    blocks: Vec<FlowBlock>,
    block_reachable: Vec<bool>,
    declined: bool,
    linear: bool,
    reachable: Vec<bool>,
    array_elements: Vec<u16>,
    array_keys: Vec<u16>,
    settled: Cell<bool>,
    constants: RefCell<Vec<MemoizedConstant>>,
}

#[derive(Clone, Copy)]
struct FlowBlock {
    start: usize,
    end: usize,
}

pub(in crate::optimizer) struct TypeFlowOptions<'a> {
    pub(in crate::optimizer) has_receiver: bool,
    pub(in crate::optimizer) class_name: Option<&'a Atom>,
    pub(in crate::optimizer) class_type_parameters: &'a [CompiledTypeParameter],
    pub(in crate::optimizer) track_array_elements: bool,
    pub(in crate::optimizer) cache_constants: bool,
    pub(in crate::optimizer) capture_types: Vec<Option<TypeDescriptor>>,
}

pub(in crate::optimizer) struct ResolvedProperty<'a> {
    pub(crate) class: &'a CompiledClassLike,
    pub(crate) property: &'a CompiledProperty,
    pub(crate) slot: u16,
}

struct ExactClass<'a> {
    class: &'a CompiledClassLike,
    arguments: Option<Vec<TypeDescriptor>>,
}

impl<'a> TypeFlow<'a> {
    fn expanded_aliases<'descriptor>(
        &self,
        descriptor: &'descriptor TypeDescriptor,
    ) -> Cow<'descriptor, TypeDescriptor> {
        match self.unit {
            Some(unit) => Cow::Owned(unit.expand_aliases(descriptor)),
            None => Cow::Borrowed(descriptor),
        }
    }

    fn expand_aliases_owned(&self, descriptor: TypeDescriptor) -> TypeDescriptor {
        match self.unit {
            Some(unit) => unit.expand_aliases(&descriptor),
            None => descriptor,
        }
    }

    fn descriptor_fact(&self, descriptor: &TypeDescriptor, origin: u32) -> Fact {
        Fact {
            mask: self
                .unit
                .and_then(|unit| unit.descriptor_mask(descriptor, 0))
                .or_else(|| descriptor_mask(descriptor))
                .unwrap_or(ALL),
            origin,
            array: NO_ORIGIN,
            observable_release: descriptor_may_release_observably(descriptor),
            non_negative: matches!(
                descriptor,
                TypeDescriptor::IntLiteral(value) if *value >= 0
            ) || matches!(
                descriptor,
                TypeDescriptor::IntRange { min: Some(min), .. } if *min >= 0
            ),
            positive: matches!(
                descriptor,
                TypeDescriptor::IntLiteral(value) if *value > 0
            ) || matches!(
                descriptor,
                TypeDescriptor::IntRange { min: Some(min), .. } if *min > 0
            ),
        }
    }

    pub(in crate::optimizer) fn chunk(&self) -> &'a Chunk {
        self.chunk
    }

    pub(in crate::optimizer) fn capture_types(&self) -> &[Option<TypeDescriptor>] {
        &self.capture_types
    }

    pub(in crate::optimizer) fn analyze(
        chunk: &'a Chunk,
        parameters: &'a [CompiledParameter],
        has_receiver: bool,
        class_name: Option<&'a Atom>,
        class_type_parameters: &'a [CompiledTypeParameter],
        allocator: &'a Heap,
    ) -> Self {
        Self::analyze_optional_unit(
            chunk,
            parameters,
            None,
            allocator,
            TypeFlowOptions {
                has_receiver,
                class_name,
                class_type_parameters,
                track_array_elements: true,
                cache_constants: true,
                capture_types: Vec::new(),
            },
            None,
        )
    }

    pub(in crate::optimizer) fn analyze_with_unit(
        chunk: &'a Chunk,
        parameters: &'a [CompiledParameter],
        has_receiver: bool,
        class_name: Option<&'a Atom>,
        class_type_parameters: &'a [CompiledTypeParameter],
        unit: &'a IndexedUnit<'a>,
        allocator: &'a Heap,
    ) -> Self {
        Self::analyze_with_unit_options(
            chunk,
            parameters,
            unit,
            allocator,
            TypeFlowOptions {
                has_receiver,
                class_name,
                class_type_parameters,
                track_array_elements: true,
                cache_constants: true,
                capture_types: Vec::new(),
            },
        )
    }

    pub(in crate::optimizer) fn analyze_with_unit_options(
        chunk: &'a Chunk,
        parameters: &'a [CompiledParameter],
        unit: &'a IndexedUnit<'a>,
        allocator: &'a Heap,
        options: TypeFlowOptions<'a>,
    ) -> Self {
        Self::analyze_optional_unit(chunk, parameters, Some(unit), allocator, options, None)
    }

    pub(in crate::optimizer) fn analyze_live_with_unit(
        chunk: &'a Chunk,
        registers: &[Value],
        unit: &'a IndexedUnit<'a>,
        allocator: &'a Heap,
    ) -> Self {
        Self::analyze_optional_unit(
            chunk,
            &[],
            Some(unit),
            allocator,
            TypeFlowOptions {
                has_receiver: false,
                class_name: None,
                class_type_parameters: &[],
                track_array_elements: false,
                cache_constants: true,
                capture_types: Vec::new(),
            },
            Some(registers),
        )
    }

    fn analyze_optional_unit(
        chunk: &'a Chunk,
        parameters: &'a [CompiledParameter],
        unit: Option<&'a IndexedUnit<'a>>,
        allocator: &'a Heap,
        options: TypeFlowOptions<'a>,
        entry_values: Option<&[Value]>,
    ) -> Self {
        let TypeFlowOptions {
            has_receiver,
            class_name,
            class_type_parameters,
            track_array_elements,
            cache_constants,
            capture_types,
        } = options;

        let register_count = usize::from(chunk.register_count);
        let capture_count = capture_types.len();
        let linear = chunk_is_linear(chunk);
        let blocks = if linear {
            Vec::new()
        } else {
            flow_blocks(chunk)
        };

        let state_count = blocks.len().saturating_mul(register_count);
        let declined = !linear && state_count > MAXIMUM_FLOW_STATES;
        let mut flow = Self {
            chunk,
            parameters,
            capture_types,
            class_name,
            class_type_parameters,
            unit,
            allocator,
            facts: Vec::new(),
            fact_offsets: vec![0; chunk.code.len() + 1],
            fact_registers: Vec::new(),
            block_states: if declined {
                Vec::new()
            } else {
                vec![Fact::UNKNOWN; state_count]
            },
            block_reachable: vec![false; blocks.len()],
            blocks,
            declined,
            linear,
            reachable: vec![false; chunk.code.len()],
            array_elements: if track_array_elements {
                vec![0; chunk.code.len() + parameters.len() + capture_count + 1]
            } else {
                Vec::new()
            },
            array_keys: if track_array_elements {
                vec![0; chunk.code.len() + parameters.len() + capture_count + 1]
            } else {
                Vec::new()
            },
            settled: Cell::new(false),
            constants: RefCell::new(if cache_constants {
                vec![None; chunk.code.len()]
            } else {
                Vec::new()
            }),
        };

        if !declined {
            flow.initialize_fact_layout();
        }

        if chunk.code.is_empty() || declined {
            flow.settled.set(true);
            return flow;
        }

        if let Some(values) = entry_values {
            flow.run_from_values(values, false);
        } else if track_array_elements {
            flow.seed_parameter_array_facts();
            flow.seed_capture_array_facts();
            flow.run_from_unknown(has_receiver, true);
            while flow.infer_array_facts() && flow.array_facts_affect_flow() {
                flow.run(has_receiver, true);
            }
        } else {
            flow.run_from_unknown(has_receiver, false);
        }

        flow.settled.set(true);
        flow
    }

    fn run_from_unknown(&mut self, has_receiver: bool, arrays_ready: bool) {
        self.run_over(has_receiver, arrays_ready, None);
    }

    fn run_from_values(&mut self, values: &[Value], arrays_ready: bool) {
        self.run_over(false, arrays_ready, Some(values));
    }

    fn run(&mut self, has_receiver: bool, arrays_ready: bool) {
        self.facts.fill(Fact::UNKNOWN);
        self.block_states.fill(Fact::UNKNOWN);
        self.block_reachable.fill(false);
        self.reachable.fill(false);
        self.run_over(has_receiver, arrays_ready, None);
    }

    fn run_over(&mut self, has_receiver: bool, arrays_ready: bool, entry_values: Option<&[Value]>) {
        let register_count = usize::from(self.chunk.register_count);
        let mut entry: Vec<_> = entry_values
            .into_iter()
            .flatten()
            .take(register_count)
            .map(Fact::from_value)
            .collect();
        entry.resize(register_count, Fact::UNKNOWN);
        if entry_values.is_some() {
            self.propagate(entry, arrays_ready);
            return;
        }

        for fact in &mut entry[usize::from(self.chunk.local_register_count)..] {
            *fact = Fact::known(0);
        }

        for register in &self.chunk.uninitialized_registers {
            entry[usize::from(register.index())] = Fact::known(0);
        }

        let first_parameter = usize::from(has_receiver);
        if has_receiver && register_count != 0 {
            entry[0] = Fact::with_origin(OBJECT, THIS_ORIGIN);
        }

        for (index, parameter) in self.parameters.iter().enumerate() {
            let register = first_parameter + index;
            if register >= register_count {
                continue;
            }

            if !parameter.has_default
                && let Some(descriptor) = &parameter.declared_type
            {
                let descriptor = self.expanded_aliases(descriptor);
                let mut fact = self.descriptor_fact(&descriptor, PARAMETER_ORIGIN | index as u32);
                let array = self.parameter_array_identity(index);
                if !self.array_elements.is_empty()
                    && (self.array_elements[array] != 0 || self.array_keys[array] != 0)
                {
                    fact.array = array as u32;
                }

                entry[register] = fact;
            }

            if let Some(target) = self.chunk.trace_argument_registers.get(index)
                && *target != Register::NONE
            {
                entry[usize::from(target.index())] = entry[register];
            }
        }

        let first_capture = first_parameter + self.parameters.len();
        for (index, descriptor) in self.capture_types.iter().enumerate() {
            let register = first_capture + index;
            if register >= register_count {
                continue;
            }

            let Some(descriptor) = descriptor else {
                continue;
            };
            let descriptor = self.expanded_aliases(descriptor);
            let mut fact = self.descriptor_fact(&descriptor, CAPTURE_ORIGIN | index as u32);
            let array = self.capture_array_identity(index);
            if !self.array_elements.is_empty()
                && (self.array_elements[array] != 0 || self.array_keys[array] != 0)
            {
                fact.array = array as u32;
            }
            entry[register] = fact;
        }

        self.propagate(entry, arrays_ready);
    }

    fn propagate(&mut self, entry: Vec<Fact>, arrays_ready: bool) {
        let register_count = usize::from(self.chunk.register_count);
        if self.linear {
            self.run_linear(&entry, arrays_ready);
            return;
        }

        let mut work = Vec::new();
        let mut queued = vec![false; self.blocks.len()];
        let mut scratch = vec![Fact::UNKNOWN; register_count];
        let mut exceptional = vec![Fact::UNKNOWN; register_count];
        let mut next = Vec::new();
        let mut refinements = Vec::new();
        self.seed_block(0, &entry, &mut work, &mut queued);

        while let Some(block_index) = work.pop() {
            queued[block_index] = false;
            let block = self.blocks[block_index];
            scratch.copy_from_slice(self.block_state(block_index));
            for index in block.start..block.end {
                self.reachable[index] = true;
                self.record_facts(index, &scratch);
                self.merge_exceptional_successors(
                    index,
                    &scratch,
                    &mut exceptional,
                    &mut work,
                    &mut queued,
                );
                let precise_result = self.precise_result(index);
                transfer(
                    self.chunk,
                    index,
                    &mut scratch,
                    arrays_ready.then_some(self.array_elements.as_slice()),
                    arrays_ready.then_some(self.array_keys.as_slice()),
                );

                if let Some((destination, fact)) = precise_result {
                    scratch[usize::from(destination.index())] = fact;
                }
            }

            let index = block.end - 1;
            next.clear();
            successors(self.chunk, index, &mut next);
            for successor in next.iter().copied() {
                if successor >= self.chunk.code.len() {
                    continue;
                }

                refinements.clear();
                if let Some((register, fact)) = self.is_true_edge(index, successor, &scratch) {
                    refine_aliases(&mut scratch, register, fact, &mut refinements);
                }
                if let Some((register, lower_bound)) =
                    self.integer_lower_bound_edge(index, successor, &scratch)
                {
                    let mut fact = scratch[usize::from(register.index())];
                    fact.non_negative |= lower_bound >= 0;
                    fact.positive |= lower_bound > 0;
                    refine_aliases(&mut scratch, register, fact, &mut refinements);
                }

                let successor_block = self.block_at(successor);
                let changed = self.merge_into_block(successor_block, &scratch);
                while let Some((register, previous)) = refinements.pop() {
                    scratch[register] = previous;
                }

                if changed && !queued[successor_block] {
                    work.push(successor_block);
                    queued[successor_block] = true;
                }
            }
        }
    }

    /// Propagates facts through a chunk whose only path is straight through.
    fn run_linear(&mut self, entry: &[Fact], arrays_ready: bool) {
        let mut state = entry.to_vec();
        for index in 0..self.chunk.code.len() {
            self.reachable[index] = true;
            self.record_facts(index, &state);
            let precise_result = self.precise_result(index);
            transfer(
                self.chunk,
                index,
                &mut state,
                arrays_ready.then_some(self.array_elements.as_slice()),
                arrays_ready.then_some(self.array_keys.as_slice()),
            );
            if let Some((destination, fact)) = precise_result {
                state[usize::from(destination.index())] = fact;
            }
        }
    }

    fn is_true_edge(
        &self,
        index: usize,
        successor: usize,
        state: &[Fact],
    ) -> Option<(Register, Fact)> {
        let (subject, is_null) = match self.chunk.code[index] {
            Instruction::JumpIfNull { subject, .. } => (subject, successor != index + 1),
            Instruction::JumpIfNotNull { subject, .. } => (subject, successor == index + 1),
            _ => (Register::NONE, false),
        };
        if subject != Register::NONE {
            let mut fact = state[usize::from(subject.index())];
            if is_null {
                if fact.mask & NULL == 0 {
                    return None;
                }
                return Some((subject, Fact::with_origin(NULL, fact.origin)));
            }

            fact.mask &= !NULL;
            if fact.mask == 0 {
                return None;
            }
            return Some((subject, fact));
        }

        let (condition, truth) = match self.chunk.code[index] {
            Instruction::JumpIfFalse { condition, .. } => (condition, successor == index + 1),
            Instruction::JumpIfTrue { condition, .. } => (condition, successor != index + 1),
            _ => return None,
        };
        if !truth {
            return None;
        }

        let origin = state[usize::from(condition.index())].origin;
        let producer = instruction_index(origin)?;
        let Instruction::Is {
            destination,
            source,
            descriptor,
        } = self.chunk.code[producer]
        else {
            return None;
        };
        if destination != condition {
            return None;
        }

        let descriptor = &self.chunk.type_descriptors[usize::from(descriptor.index())];
        let descriptor = self.expanded_aliases(descriptor);
        Some((source, self.descriptor_fact(&descriptor, origin)))
    }

    fn integer_lower_bound_edge(
        &self,
        index: usize,
        successor: usize,
        state: &[Fact],
    ) -> Option<(Register, i64)> {
        let truth = successor == index + 1;
        let (comparison, subject, constant) = match self.chunk.code[index] {
            Instruction::IntJumpUnlessImmediate {
                comparison,
                source,
                immediate,
                ..
            } => (comparison, source, i64::from(immediate.value())),
            Instruction::JumpUnlessConstant {
                comparison,
                source,
                constant,
                ..
            } => {
                let Literal::Int(constant) = self.chunk.constants[usize::from(constant.index())]
                else {
                    return None;
                };
                (comparison, source, constant)
            }
            Instruction::JumpUnless {
                comparison,
                left,
                right,
                ..
            }
            | Instruction::IntJumpUnless {
                comparison,
                left,
                right,
                ..
            } => {
                if let Some(ConstantValue::Int(constant)) =
                    self.constant_value_fact(state[usize::from(right.index())], 0)
                {
                    (comparison, left, constant)
                } else if let Some(ConstantValue::Int(constant)) =
                    self.constant_value_fact(state[usize::from(left.index())], 0)
                {
                    (comparison.reversed(), right, constant)
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        let fact = state[usize::from(subject.index())];
        if fact.mask & !INT != 0 {
            return None;
        }

        comparison_lower_bound(comparison, constant, truth).map(|bound| (subject, bound))
    }

    fn infer_array_facts(&mut self) -> bool {
        let mut changed = false;
        for index in 0..self.chunk.code.len() {
            if !self.reachable[index] {
                continue;
            }

            let (array, elements, keys) = match self.chunk.code[index] {
                Instruction::PropertyGet { .. }
                | Instruction::PropertyGetUnchecked { .. }
                | Instruction::ConstantGet { .. }
                | Instruction::ClassConstantGet { .. }
                | Instruction::CallValue { .. }
                | Instruction::CallValueUnchecked { .. }
                | Instruction::CallMethod { .. }
                | Instruction::CallMethodUnchecked { .. }
                | Instruction::CallMethodDirect { .. }
                | Instruction::CallNamed { .. }
                | Instruction::CallNamedUnchecked { .. }
                | Instruction::CallNamedConstantUnchecked { .. }
                | Instruction::CallSelfUnchecked { .. } => {
                    let array = index as u32 + 1;
                    let Some(descriptor) = self.origin_type(array, 0) else {
                        continue;
                    };
                    let Some((keys, element)) = array_shape(&descriptor) else {
                        continue;
                    };
                    (array, descriptor_mask(element).unwrap_or(ALL), keys)
                }
                Instruction::NewVec {
                    element_count,
                    first_element,
                    ..
                }
                | Instruction::NewTuple {
                    element_count,
                    first_element,
                    ..
                } => {
                    let first = usize::from(first_element.index());
                    let count = usize::from(element_count.value());
                    let mask = (0..count).fold(0, |mask, offset| {
                        mask | self
                            .fact(index, Register::new((first + offset) as u16))
                            .mask
                    });
                    (index as u32 + 1, mask, INT)
                }
                Instruction::NewFilledVec { value, .. } => {
                    (index as u32 + 1, self.fact(index, value).mask, INT)
                }
                Instruction::NewDict {
                    pair_count,
                    first_pair,
                    ..
                } => {
                    let first = usize::from(first_pair.index());
                    let count = usize::from(pair_count.value());
                    let (keys, elements) = (0..count).fold((0, 0), |masks, pair| {
                        (
                            masks.0
                                | self
                                    .fact(index, Register::new((first + pair * 2) as u16))
                                    .mask,
                            masks.1
                                | self
                                    .fact(index, Register::new((first + pair * 2 + 1) as u16))
                                    .mask,
                        )
                    });
                    (index as u32 + 1, elements, keys)
                }
                Instruction::IndexSet {
                    container,
                    index: subscript,
                    value,
                }
                | Instruction::DictIndexSet {
                    container,
                    index: subscript,
                    value,
                } => (
                    self.fact(index, container).array,
                    self.fact(index, value).mask,
                    self.fact(index, subscript).mask,
                ),
                Instruction::VecIndexSet {
                    container, value, ..
                }
                | Instruction::Append { container, value }
                | Instruction::VecAppend { container, value }
                | Instruction::DictIndexSetIntKey {
                    container, value, ..
                } => (
                    self.fact(index, container).array,
                    self.fact(index, value).mask,
                    INT,
                ),
                Instruction::DictIndexSetStringKey {
                    container, value, ..
                } => (
                    self.fact(index, container).array,
                    self.fact(index, value).mask,
                    STRING,
                ),
                Instruction::IndexAddAssign {
                    container,
                    value,
                    mode,
                    ..
                } => {
                    let array = self.fact(index, container).array;
                    let current = if array != NO_ORIGIN {
                        self.array_elements[array as usize]
                    } else {
                        0
                    };
                    let elements = match mode {
                        IndexAddMode::DictAnyKeyIntValue | IndexAddMode::DictStringKeyIntValue => {
                            INT
                        }
                        IndexAddMode::Generic if current == 0 => ALL,
                        IndexAddMode::Generic => {
                            numeric_result(Fact::known(current), self.fact(index, value)).mask
                        }
                    };
                    (array, elements, 0)
                }
                Instruction::Spread { container, .. } => {
                    (self.fact(index, container).array, ALL, ALL)
                }
                _ => continue,
            };

            if array != NO_ORIGIN {
                let array = array as usize;
                let previous_elements = self.array_elements[array];
                let previous_keys = self.array_keys[array];
                self.array_elements[array] |= elements;
                self.array_keys[array] |= keys;
                changed |= previous_elements != self.array_elements[array]
                    || previous_keys != self.array_keys[array];
            }
        }

        changed
    }

    fn array_facts_affect_flow(&self) -> bool {
        self.chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::IndexGet { .. }
                    | Instruction::VecIndexGet { .. }
                    | Instruction::DictIndexGetIntKey { .. }
                    | Instruction::DictIndexGetStringKey { .. }
                    | Instruction::ForeachNext { .. }
                    | Instruction::VecForeachNext {
                        value_mode: ArrayValueMode::Generic,
                        ..
                    }
                    | Instruction::DictForeachNext {
                        value_mode: ArrayValueMode::Generic,
                        ..
                    }
                    | Instruction::PropertyGet { .. }
                    | Instruction::PropertyGetUnchecked { .. }
            )
        })
    }

    fn seed_parameter_array_facts(&mut self) {
        for (index, parameter) in self.parameters.iter().enumerate() {
            let Some(descriptor) = &parameter.declared_type else {
                continue;
            };

            let descriptor = self.expanded_aliases(descriptor);

            let Some((keys, element)) = array_shape(&descriptor) else {
                continue;
            };

            let identity = self.parameter_array_identity(index);
            self.array_keys[identity] = keys;
            if let Some(mask) = descriptor_mask(element) {
                self.array_elements[identity] = mask;
            }
        }
    }

    fn seed_capture_array_facts(&mut self) {
        for index in 0..self.capture_types.len() {
            let Some(descriptor) = &self.capture_types[index] else {
                continue;
            };

            let descriptor = self.expanded_aliases(descriptor);
            let Some((keys, element)) = array_shape(&descriptor) else {
                continue;
            };

            let identity = self.capture_array_identity(index);
            self.array_keys[identity] = keys;
            if let Some(mask) = descriptor_mask(element) {
                self.array_elements[identity] = mask;
            }
        }
    }

    fn parameter_array_identity(&self, index: usize) -> usize {
        self.chunk.code.len() + index + 1
    }

    fn capture_array_identity(&self, index: usize) -> usize {
        self.chunk.code.len() + self.parameters.len() + index + 1
    }

    fn initialize_fact_layout(&mut self) {
        let register_count = usize::from(self.chunk.register_count);
        let mut registers = Vec::new();
        for (index, instruction) in self.chunk.code.iter().copied().enumerate() {
            self.fact_offsets[index] = self.fact_registers.len();
            registers.clear();
            if !for_each_register(instruction, |register| {
                if usize::from(register.index()) < register_count {
                    registers.push(register);
                }
            }) {
                for register in 0..self.chunk.register_count {
                    let register = Register::new(register);
                    if !effect_on(self.chunk, instruction, register).is_none() {
                        registers.push(register);
                    }
                }
            }
            registers.sort_unstable_by_key(|register| register.index());
            registers.dedup();
            self.fact_registers.extend_from_slice(&registers);
            self.fact_offsets[index + 1] = self.fact_registers.len();
        }
        self.facts.resize(self.fact_registers.len(), Fact::UNKNOWN);
    }

    fn record_facts(&mut self, index: usize, state: &[Fact]) {
        let start = self.fact_offsets[index];
        let end = self.fact_offsets[index + 1];
        for (fact, register) in self.facts[start..end]
            .iter_mut()
            .zip(&self.fact_registers[start..end])
        {
            *fact = state[usize::from(register.index())];
        }
    }

    fn block_at(&self, instruction: usize) -> usize {
        self.blocks
            .partition_point(|block| block.start <= instruction)
            .saturating_sub(1)
    }

    fn seed_block(
        &mut self,
        block: usize,
        state: &[Fact],
        work: &mut Vec<usize>,
        queued: &mut [bool],
    ) {
        if self.block_reachable[block] {
            return;
        }

        self.block_reachable[block] = true;
        self.block_state_mut(block).copy_from_slice(state);
        work.push(block);
        queued[block] = true;
    }

    fn merge_into_block(&mut self, block: usize, incoming: &[Fact]) -> bool {
        if !self.block_reachable[block] {
            self.block_reachable[block] = true;
            self.block_state_mut(block).copy_from_slice(incoming);
            return true;
        }

        let register_count = usize::from(self.chunk.register_count);
        let start = block * register_count;
        let mut changed = false;
        for (position, incoming) in incoming.iter().copied().enumerate() {
            let current = self.block_states[start + position];
            let mut merged = current.merge(incoming);
            if current.origin != incoming.origin
                && (self.equivalent_class_constant_origins(current.origin, incoming.origin)
                    || self.equivalent_final_class_origins(current.origin, incoming.origin))
            {
                merged.origin = current.origin;
            }
            changed |= merged != current;
            self.block_states[start + position] = merged;
        }

        changed
    }

    fn merge_exceptional_successors(
        &mut self,
        index: usize,
        state: &[Fact],
        exceptional: &mut [Fact],
        work: &mut Vec<usize>,
        queued: &mut [bool],
    ) {
        let instruction = self.chunk.code[index];
        for position in 0..self.chunk.catch_table.len() {
            let entry = self.chunk.catch_table[position];
            if index < entry.start as usize
                || index >= entry.end as usize
                || entry.handler as usize >= self.chunk.code.len()
            {
                continue;
            }

            exceptional.copy_from_slice(state);
            if !for_each_write_register(instruction, |register| {
                exceptional[usize::from(register.index())] = Fact::UNKNOWN;
            }) {
                for (register, fact) in exceptional.iter_mut().enumerate() {
                    if effect_on(self.chunk, instruction, Register::new(register as u16)).writes() {
                        *fact = Fact::UNKNOWN;
                    }
                }
            }

            exceptional[usize::from(entry.temporary_floor)..].fill(Fact::known(0));
            if let Some(binding) = entry.binding {
                exceptional[usize::from(binding.index())] = Fact::known(OBJECT);
            }

            let block = self.block_at(entry.handler as usize);
            let changed = self.merge_into_block(block, exceptional);
            if changed && !queued[block] {
                work.push(block);
                queued[block] = true;
            }
        }
    }

    fn block_state(&self, block: usize) -> &[Fact] {
        let register_count = usize::from(self.chunk.register_count);
        let start = block * register_count;
        &self.block_states[start..start + register_count]
    }

    fn block_state_mut(&mut self, block: usize) -> &mut [Fact] {
        let register_count = usize::from(self.chunk.register_count);
        let start = block * register_count;
        &mut self.block_states[start..start + register_count]
    }

    fn fact(&self, index: usize, register: Register) -> Fact {
        let register_index = usize::from(register.index());
        if self.declined || register_index >= usize::from(self.chunk.register_count) {
            return Fact::UNKNOWN;
        }

        let start = self.fact_offsets[index];
        let end = self.fact_offsets[index + 1];
        let registers = &self.fact_registers[start..end];
        let Ok(position) =
            registers.binary_search_by_key(&register.index(), |register| register.index())
        else {
            return Fact::UNKNOWN;
        };
        self.facts[start + position]
    }

    /// Whether releasing the value held before `index` may be observed through
    /// a destructor, captured object, or object weak reference.
    pub(in crate::optimizer) fn register_may_release_observably(
        &self,
        index: usize,
        register: Register,
    ) -> bool {
        let fact = self.fact(index, register);
        fact.observable_release && (register.index() == 0 || fact.origin != THIS_ORIGIN)
    }
}

fn array_shape(descriptor: &TypeDescriptor) -> Option<(u16, &TypeDescriptor)> {
    match descriptor {
        TypeDescriptor::Array(Some((key, value)))
        | TypeDescriptor::Dictionary(Some((key, value))) => {
            Some((descriptor_mask(key).unwrap_or(INT | STRING), value.as_ref()))
        }
        TypeDescriptor::Vector(Some(element)) => Some((INT, element.as_ref())),
        _ => None,
    }
}

fn chunk_is_linear(chunk: &Chunk) -> bool {
    if chunk.code.is_empty()
        || !chunk.catch_table.is_empty()
        || chunk.code[..chunk.code.len() - 1]
            .iter()
            .any(|instruction| branches_or_terminates(*instruction))
    {
        return false;
    }

    let mut next = Vec::with_capacity(2);
    successors(chunk, chunk.code.len() - 1, &mut next);
    next.iter().all(|successor| *successor >= chunk.code.len())
}

fn flow_blocks(chunk: &Chunk) -> Vec<FlowBlock> {
    if chunk.code.is_empty() {
        return Vec::new();
    }

    let mut starts = vec![0usize];
    let mut edges = Vec::new();
    for (index, instruction) in chunk.code.iter().copied().enumerate() {
        if !branches_or_terminates(instruction) {
            continue;
        }

        if index + 1 < chunk.code.len() {
            starts.push(index + 1);
        }

        edges.clear();
        successors(chunk, index, &mut edges);
        starts.extend(
            edges
                .iter()
                .copied()
                .filter(|target| *target < chunk.code.len()),
        );
    }

    for entry in &chunk.catch_table {
        let handler = entry.handler as usize;
        if handler < chunk.code.len() {
            starts.push(handler);
        }
    }

    starts.sort_unstable();
    starts.dedup();
    let mut blocks = Vec::with_capacity(starts.len());
    for (position, start) in starts.iter().copied().enumerate() {
        let end = starts
            .get(position + 1)
            .copied()
            .unwrap_or(chunk.code.len());
        blocks.push(FlowBlock { start, end });
    }

    blocks
}

fn comparison_lower_bound(
    comparison: BytecodeComparison,
    constant: i64,
    truth: bool,
) -> Option<i64> {
    match (comparison, truth) {
        (BytecodeComparison::Equal, true) | (BytecodeComparison::NotEqual, false) => Some(constant),
        (BytecodeComparison::LessThan, false) | (BytecodeComparison::GreaterThanOrEqual, true) => {
            Some(constant)
        }
        (BytecodeComparison::LessThanOrEqual, false) | (BytecodeComparison::GreaterThan, true) => {
            constant.checked_add(1)
        }
        _ => None,
    }
}

fn unary_numeric_result(source: Fact) -> Fact {
    if source.mask & !INT == 0 {
        Fact::known(INT)
    } else if source.mask & !FLOAT == 0 {
        Fact::known(FLOAT)
    } else {
        Fact::known(NUMERIC)
    }
}

fn with_origin(mut fact: Fact, origin: u32) -> Fact {
    fact.origin = origin;
    fact
}

fn refine_aliases(
    state: &mut [Fact],
    register: Register,
    refinement: Fact,
    previous: &mut Vec<(usize, Fact)>,
) {
    let register = usize::from(register.index());
    let origin = state[register].origin;
    for (index, fact) in state.iter_mut().enumerate() {
        if index != register && (origin == NO_ORIGIN || fact.origin != origin) {
            continue;
        }

        previous.push((index, *fact));
        fact.mask &= refinement.mask;
        fact.non_negative |= refinement.non_negative;
        fact.positive |= refinement.positive;
        if fact.mask & MAY_BE_REFERENCE_COUNTED == 0 {
            fact.observable_release = false;
        }
    }
}

fn append_constant_text(bytes: &mut Vec<u8>, value: ConstantValue) -> Option<()> {
    match value {
        ConstantValue::String(value) => bytes.extend_from_slice(value.as_bytes()),
        ConstantValue::Int(value) => {
            let mut buffer = itoa::Buffer::new();
            bytes.extend_from_slice(buffer.format(value).as_bytes());
        }
        ConstantValue::Float(value) if value.is_nan() => bytes.extend_from_slice(b"NAN"),
        ConstantValue::Float(value) if value.is_infinite() => {
            bytes.extend_from_slice(if value.is_sign_negative() {
                b"-INF"
            } else {
                b"INF"
            })
        }
        ConstantValue::Float(value) => {
            let mut buffer = ryu::Buffer::new();
            bytes.extend_from_slice(buffer.format(value).as_bytes());
        }
        ConstantValue::Null | ConstantValue::Bool(_) => return None,
    }

    Some(())
}

fn same_atom(left: &Atom, right: &Atom) -> bool {
    left.as_bytes() == right.as_bytes()
}

fn callable_signature(descriptor: &TypeDescriptor) -> Option<FunctionTypeDescriptor> {
    match descriptor {
        TypeDescriptor::Callable(Some(signature)) => Some(signature.clone()),
        TypeDescriptor::Union(members) => {
            let mut signature = None;
            for member in members {
                if matches!(member, TypeDescriptor::Null) {
                    continue;
                }
                let TypeDescriptor::Callable(Some(candidate)) = member else {
                    return None;
                };
                if signature.is_some() {
                    return None;
                }
                signature = Some(candidate.clone());
            }
            signature
        }
        _ => None,
    }
}

fn instruction_index(origin: u32) -> Option<usize> {
    (origin != NO_ORIGIN && origin & EXTERNAL_ORIGIN == 0 && origin != THIS_ORIGIN)
        .then(|| origin as usize - 1)
}

fn fact_bits(fact: Fact) -> impl Iterator<Item = Fact> {
    (0..u16::BITS).filter_map(move |bit| {
        let mask = 1u16 << bit;
        (fact.mask & mask != 0).then_some(Fact {
            mask,
            origin: fact.origin,
            array: fact.array,
            observable_release: fact.observable_release,
            non_negative: fact.non_negative,
            positive: fact.positive,
        })
    })
}
