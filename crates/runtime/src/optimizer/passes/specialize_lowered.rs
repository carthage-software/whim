//! Cheap specialization from exact kinds visible in freshly lowered bytecode.

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ArrayValueMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::cfg::for_each_control_flow_target;
use crate::optimizer::operands::for_each_write_register;
use crate::value::atom::Atom;

#[derive(Clone, Copy, PartialEq, Eq)]
enum KnownKind {
    Unknown,
    Null,
    Bool,
    Int,
    Float,
    String,
    Object,
    Vec,
    Dict,
    Tuple,
    Callable,
}

struct Facts {
    kinds: Vec<KnownKind>,
    generations: Vec<u32>,
    generation: u32,
}

impl Facts {
    fn new(register_count: usize, entry: &[(Register, KnownKind)]) -> Self {
        let mut facts = Self {
            kinds: vec![KnownKind::Unknown; register_count],
            generations: vec![0; register_count],
            generation: 1,
        };
        facts.seed(entry);
        facts
    }

    fn get(&self, register: Register) -> KnownKind {
        let index = usize::from(register.index());
        if self.generations[index] == self.generation {
            self.kinds[index]
        } else {
            KnownKind::Unknown
        }
    }

    fn set(&mut self, register: Register, kind: KnownKind) {
        let index = usize::from(register.index());
        self.kinds[index] = kind;
        self.generations[index] = self.generation;
    }

    fn clear(&mut self, register: Register) {
        self.generations[usize::from(register.index())] = 0;
    }

    fn reset(&mut self, stable: &[(Register, KnownKind)]) {
        self.generation += 1;
        self.seed(stable);
    }

    fn seed(&mut self, facts: &[(Register, KnownKind)]) {
        for (register, kind) in facts {
            self.set(*register, *kind);
        }
    }
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk(
        &mut unit.main,
        &[],
        None,
        None,
        false,
        false,
        configuration,
        statistics,
    );

    let function_floor = configuration.function_floor(unit.functions.len());
    for function in &mut unit.functions[function_floor..] {
        optimize_function(
            function,
            None,
            function.captures_this,
            configuration,
            statistics,
        );
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for class in &mut unit.classes[class_floor..] {
        for method in &mut class.methods {
            let has_receiver = !method.is_static || method.function.captures_this;
            optimize_function(
                &mut method.function,
                Some(&class.name),
                has_receiver,
                configuration,
                statistics,
            );
        }
    }
}

fn optimize_function(
    function: &mut CompiledFunction,
    class_name: Option<&Atom>,
    has_receiver: bool,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    optimize_chunk(
        &mut function.chunk,
        &function.parameters,
        function.return_type.as_ref(),
        class_name,
        has_receiver,
        true,
        configuration,
        statistics,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "lowered specialization receives the callable's complete type context"
)]
fn optimize_chunk(
    chunk: &mut Chunk,
    parameters: &[CompiledParameter],
    return_type: Option<&TypeDescriptor>,
    class_name: Option<&Atom>,
    has_receiver: bool,
    check_returns: bool,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if chunk.code.is_empty() {
        return;
    }

    let candidates = CandidateSet::of(chunk, configuration);
    if !candidates.contains(CandidateSet::ARITHMETIC)
        && !candidates.contains(CandidateSet::COLLECTION)
        && !candidates.contains(CandidateSet::COMPARISON)
        && !candidates.contains(CandidateSet::COUNTER_LOOP)
        && !candidates.contains(CandidateSet::TYPE_CHECK)
    {
        return;
    }

    let entry = entry_facts(chunk, parameters, has_receiver);
    let stable = stable_entry_facts(chunk, &entry);
    let mut facts = Facts::new(usize::from(chunk.register_count), &entry);
    let mut targets = vec![false; chunk.code.len()];
    for_each_control_flow_target(chunk, |target| {
        if let Some(is_target) = targets.get_mut(target) {
            *is_target = true;
        }
    });

    for (index, is_target) in targets.iter().copied().enumerate() {
        if index != 0 && is_target {
            facts.reset(&stable);
        }

        let instruction = chunk.code[index];
        let (replacement, category) = specialize(instruction, &facts, configuration);
        if let Some(replacement) = replacement {
            chunk.code[index] = replacement;
            match category {
                Specialization::Operation => statistics.operations_specialized += 1,
                Specialization::Array => {
                    statistics.array_operations_specialized += 1;
                }
            }
        }

        if check_returns
            && configuration.elide_type_checks
            && let Some(replacement) = specialize_return(
                chunk.code[index],
                &facts,
                return_type,
                class_name,
                has_receiver,
            )
        {
            chunk.code[index] = replacement;
            statistics.type_checks_elided += 1;
        }

        transfer(chunk, chunk.code[index], &mut facts, &stable);
        if has_no_fallthrough(chunk.code[index]) {
            facts.reset(&stable);
        }
    }
}

fn entry_facts(
    chunk: &Chunk,
    parameters: &[CompiledParameter],
    has_receiver: bool,
) -> Vec<(Register, KnownKind)> {
    let mut facts = Vec::with_capacity(parameters.len() + usize::from(has_receiver));
    if has_receiver && chunk.register_count != 0 {
        facts.push((Register::new(0), KnownKind::Object));
    }

    for (position, parameter) in parameters.iter().enumerate() {
        if parameter.has_default {
            continue;
        }
        let Some(descriptor) = parameter.declared_type.as_ref() else {
            continue;
        };
        let kind = descriptor_kind(descriptor);
        let Ok(position) = u16::try_from(position) else {
            continue;
        };
        let Some(index) = chunk.parameter_register_start.checked_add(position) else {
            continue;
        };
        if kind != KnownKind::Unknown && index < chunk.register_count {
            facts.push((Register::new(index), kind));
        }
    }

    facts
}

fn stable_entry_facts(
    chunk: &Chunk,
    entry: &[(Register, KnownKind)],
) -> Vec<(Register, KnownKind)> {
    let mut stable = entry.to_vec();
    for instruction in &chunk.code {
        if !for_each_write_register(*instruction, |written| {
            stable.retain(|(register, _)| *register != written);
        }) {
            stable.clear();
            break;
        }
    }

    stable
}

fn specialize_return(
    instruction: Instruction,
    facts: &Facts,
    return_type: Option<&TypeDescriptor>,
    class_name: Option<&Atom>,
    has_receiver: bool,
) -> Option<Instruction> {
    match instruction {
        Instruction::Return { source }
            if return_type.is_none_or(|expected| {
                return_kind_satisfies(
                    facts.get(source),
                    source,
                    expected,
                    class_name,
                    has_receiver,
                )
            }) =>
        {
            Some(match facts.get(source) {
                KnownKind::Null | KnownKind::Bool | KnownKind::Int | KnownKind::Float => {
                    Instruction::ReturnScalarUnchecked { source }
                }
                KnownKind::Object
                | KnownKind::Vec
                | KnownKind::Dict
                | KnownKind::Tuple
                | KnownKind::Callable => Instruction::ReturnReferenceUnchecked { source },
                KnownKind::String | KnownKind::Unknown => Instruction::ReturnUnchecked { source },
            })
        }
        Instruction::ReturnNull if return_type.is_none_or(return_null_satisfies) => {
            Some(Instruction::ReturnNullUnchecked)
        }
        _ => None,
    }
}

fn return_kind_satisfies(
    kind: KnownKind,
    source: Register,
    expected: &TypeDescriptor,
    class_name: Option<&Atom>,
    has_receiver: bool,
) -> bool {
    match expected {
        TypeDescriptor::Mixed => true,
        TypeDescriptor::Null => kind == KnownKind::Null,
        TypeDescriptor::Bool => kind == KnownKind::Bool,
        TypeDescriptor::Int => kind == KnownKind::Int,
        TypeDescriptor::Float => kind == KnownKind::Float,
        TypeDescriptor::String => kind == KnownKind::String,
        TypeDescriptor::Object => kind == KnownKind::Object,
        TypeDescriptor::Named { name, .. } => {
            has_receiver
                && source == Register::new(0)
                && class_name.is_some_and(|class_name| class_name == name)
        }
        TypeDescriptor::StaticClass => has_receiver && source == Register::new(0),
        TypeDescriptor::Array(None) => {
            matches!(kind, KnownKind::Vec | KnownKind::Dict | KnownKind::Tuple)
        }
        TypeDescriptor::Vector(None) => kind == KnownKind::Vec,
        TypeDescriptor::Dictionary(None) => kind == KnownKind::Dict,
        TypeDescriptor::Callable(None) => kind == KnownKind::Callable,
        TypeDescriptor::TupleAny => kind == KnownKind::Tuple,
        TypeDescriptor::Union(members) => members
            .iter()
            .any(|member| return_kind_satisfies(kind, source, member, class_name, has_receiver)),
        TypeDescriptor::Intersection(members) => members
            .iter()
            .all(|member| return_kind_satisfies(kind, source, member, class_name, has_receiver)),
        TypeDescriptor::Wildcard
        | TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::Array(Some(_))
        | TypeDescriptor::Vector(Some(_))
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::Dictionary(Some(_))
        | TypeDescriptor::DictionaryShape { .. }
        | TypeDescriptor::Callable(Some(_))
        | TypeDescriptor::Classname(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::Negated(_) => false,
    }
}

fn return_null_satisfies(expected: &TypeDescriptor) -> bool {
    match expected {
        TypeDescriptor::Void | TypeDescriptor::Mixed | TypeDescriptor::Null => true,
        TypeDescriptor::Union(members) => members.iter().any(return_null_satisfies),
        TypeDescriptor::Intersection(members) => members.iter().all(return_null_satisfies),
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum Specialization {
    Operation,
    Array,
}

fn specialize(
    instruction: Instruction,
    facts: &Facts,
    configuration: OptimizationConfiguration,
) -> (Option<Instruction>, Specialization) {
    if configuration.specialize_arithmetic
        && let Some(replacement) = specialize_arithmetic(instruction, facts)
    {
        return (Some(replacement), Specialization::Operation);
    }
    if configuration.specialize_arrays
        && let Some(replacement) = specialize_array(instruction, facts)
    {
        return (Some(replacement), Specialization::Array);
    }
    if configuration.specialize_comparison
        && let Some(replacement) = specialize_comparison(instruction, facts)
    {
        return (Some(replacement), Specialization::Operation);
    }
    if configuration.specialize_counter_loop
        && let Some(replacement) = specialize_counter_loop(instruction, facts)
    {
        return (Some(replacement), Specialization::Operation);
    }

    (None, Specialization::Operation)
}

fn specialize_arithmetic(instruction: Instruction, facts: &Facts) -> Option<Instruction> {
    super::specialize_arithmetic::specialize_with(
        instruction,
        |register| facts.get(register) == KnownKind::Int,
        |register| facts.get(register) == KnownKind::Float,
    )
}

fn specialize_array(instruction: Instruction, facts: &Facts) -> Option<Instruction> {
    super::specialize_arrays::specialize_with(
        instruction,
        |register| facts.get(register) == KnownKind::String,
        |register| facts.get(register) == KnownKind::Int,
        |register| facts.get(register) == KnownKind::Vec,
        |register| facts.get(register) == KnownKind::Dict,
        |_, _| ArrayValueMode::Generic,
    )
}

fn specialize_comparison(instruction: Instruction, facts: &Facts) -> Option<Instruction> {
    super::specialize_comparison::specialize_with(
        instruction,
        |register| facts.get(register) == KnownKind::Int,
        |register| facts.get(register) == KnownKind::String,
    )
}

fn specialize_counter_loop(instruction: Instruction, facts: &Facts) -> Option<Instruction> {
    super::specialize_counter_loop::specialize_with(instruction, |register| {
        facts.get(register) == KnownKind::Int
    })
}

fn transfer(
    chunk: &Chunk,
    instruction: Instruction,
    facts: &mut Facts,
    stable: &[(Register, KnownKind)],
) {
    let moved = match instruction {
        Instruction::Move { source, .. } | Instruction::MoveOwned { source, .. } => {
            facts.get(source)
        }
        Instruction::Negate { source, .. }
        | Instruction::UnaryPlus { source, .. }
        | Instruction::AddImmediate { source, .. }
        | Instruction::SubtractImmediate { source, .. }
        | Instruction::IncrementJump { target: source, .. }
        | Instruction::CounterLoop {
            counter: source, ..
        } => facts.get(source),
        _ => KnownKind::Unknown,
    };

    if !for_each_write_register(instruction, |register| facts.clear(register)) {
        facts.reset(stable);
        return;
    }

    let (destination, kind) = match instruction {
        Instruction::Move { destination, .. } | Instruction::MoveOwned { destination, .. } => {
            (destination, moved)
        }
        Instruction::LoadConstant {
            destination,
            constant,
        } => (
            destination,
            literal_kind(&chunk.constants[usize::from(constant.index())]),
        ),
        Instruction::LoadNull { destination } => (destination, KnownKind::Null),
        Instruction::LoadTrue { destination }
        | Instruction::LoadFalse { destination }
        | Instruction::Equal { destination, .. }
        | Instruction::NotEqual { destination, .. }
        | Instruction::LessThan { destination, .. }
        | Instruction::LessThanOrEqual { destination, .. }
        | Instruction::GreaterThan { destination, .. }
        | Instruction::GreaterThanOrEqual { destination, .. }
        | Instruction::Not { destination, .. }
        | Instruction::Is { destination, .. }
        | Instruction::Contains { destination, .. }
        | Instruction::ContainsKey { destination, .. }
        | Instruction::StringByteEqual { destination, .. }
        | Instruction::StringByteNotEqual { destination, .. }
        | Instruction::StringByteLessThan { destination, .. }
        | Instruction::StringByteLessThanOrEqual { destination, .. }
        | Instruction::StringByteGreaterThan { destination, .. }
        | Instruction::StringByteGreaterThanOrEqual { destination, .. } => {
            (destination, KnownKind::Bool)
        }
        Instruction::LoadInt { destination, .. }
        | Instruction::IntAdd { destination, .. }
        | Instruction::IntSubtract { destination, .. }
        | Instruction::IntMultiply { destination, .. }
        | Instruction::IntModulo { destination, .. }
        | Instruction::IntBitwiseAnd { destination, .. }
        | Instruction::IntBitwiseOr { destination, .. }
        | Instruction::IntBitwiseXor { destination, .. }
        | Instruction::IntBitwiseNot { destination, .. }
        | Instruction::IntShiftLeft { destination, .. }
        | Instruction::IntShiftRight { destination, .. }
        | Instruction::IntMultiplyImmediate { destination, .. }
        | Instruction::IntModuloImmediate { destination, .. }
        | Instruction::Length { destination, .. }
        | Instruction::StringLength { destination, .. }
        | Instruction::Modulo { destination, .. }
        | Instruction::BitwiseAnd { destination, .. }
        | Instruction::BitwiseOr { destination, .. }
        | Instruction::BitwiseXor { destination, .. }
        | Instruction::BitwiseNot { destination, .. }
        | Instruction::ShiftLeft { destination, .. }
        | Instruction::ShiftRight { destination, .. }
        | Instruction::Compare { destination, .. } => (destination, KnownKind::Int),
        Instruction::FloatAdd { destination, .. }
        | Instruction::FloatSubtract { destination, .. }
        | Instruction::FloatMultiply { destination, .. }
        | Instruction::FloatMultiplyConstant { destination, .. }
        | Instruction::Divide { destination, .. } => (destination, KnownKind::Float),
        Instruction::Add {
            destination,
            left,
            right,
        }
        | Instruction::Subtract {
            destination,
            left,
            right,
        }
        | Instruction::Multiply {
            destination,
            left,
            right,
        }
        | Instruction::Power {
            destination,
            left,
            right,
        } => (
            destination,
            same_numeric_kind(facts.get(left), facts.get(right)),
        ),
        Instruction::Negate { destination, .. }
        | Instruction::UnaryPlus { destination, .. }
        | Instruction::AddImmediate { destination, .. }
        | Instruction::SubtractImmediate { destination, .. } => (destination, moved),
        Instruction::Concatenate { destination, .. }
        | Instruction::StringIndexGet { destination, .. } => (destination, KnownKind::String),
        Instruction::NewVec { destination, .. }
        | Instruction::NewFilledVec { destination, .. }
        | Instruction::Rest { destination, .. } => (destination, KnownKind::Vec),
        Instruction::NewDict { destination, .. } => (destination, KnownKind::Dict),
        Instruction::NewTuple { destination, .. } => (destination, KnownKind::Tuple),
        Instruction::NewStatic { destination, .. }
        | Instruction::NewDynamic { destination, .. }
        | Instruction::NewTyped { destination, .. }
        | Instruction::CloneObject { destination, .. } => (destination, KnownKind::Object),
        Instruction::MakeClosure { destination, .. }
        | Instruction::MakeBound { destination, .. } => (destination, KnownKind::Callable),
        Instruction::AsCheck {
            destination,
            descriptor,
            ..
        } => (
            destination,
            descriptor_kind(&chunk.type_descriptors[usize::from(descriptor.index())]),
        ),
        Instruction::IndexGet {
            destination,
            container,
            index,
        } if facts.get(container) == KnownKind::String && facts.get(index) == KnownKind::Int => {
            (destination, KnownKind::String)
        }
        Instruction::VecIndexGet {
            destination,
            value_mode,
            ..
        }
        | Instruction::DictIndexGetIntKey {
            destination,
            value_mode,
            ..
        }
        | Instruction::DictIndexGetStringKey {
            destination,
            value_mode,
            ..
        } => (destination, array_value_kind(value_mode)),
        _ => return,
    };

    if kind != KnownKind::Unknown {
        facts.set(destination, kind);
    }
}

fn literal_kind(literal: &Literal) -> KnownKind {
    match literal {
        Literal::Null => KnownKind::Null,
        Literal::Bool(_) => KnownKind::Bool,
        Literal::Int(_) => KnownKind::Int,
        Literal::Float(_) => KnownKind::Float,
        Literal::String(_) => KnownKind::String,
    }
}

fn array_value_kind(mode: ArrayValueMode) -> KnownKind {
    match mode {
        ArrayValueMode::Int => KnownKind::Int,
        ArrayValueMode::Float => KnownKind::Float,
        ArrayValueMode::Generic => KnownKind::Unknown,
    }
}

fn same_numeric_kind(left: KnownKind, right: KnownKind) -> KnownKind {
    match (left, right) {
        (KnownKind::Int, KnownKind::Int) => KnownKind::Int,
        (KnownKind::Float, KnownKind::Float) => KnownKind::Float,
        _ => KnownKind::Unknown,
    }
}

fn descriptor_kind(descriptor: &TypeDescriptor) -> KnownKind {
    match descriptor {
        TypeDescriptor::Null => KnownKind::Null,
        TypeDescriptor::Bool | TypeDescriptor::TrueLiteral | TypeDescriptor::FalseLiteral => {
            KnownKind::Bool
        }
        TypeDescriptor::Int | TypeDescriptor::IntLiteral(_) | TypeDescriptor::IntRange { .. } => {
            KnownKind::Int
        }
        TypeDescriptor::Float | TypeDescriptor::FloatLiteral(_) => KnownKind::Float,
        TypeDescriptor::String
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Classname(_) => KnownKind::String,
        TypeDescriptor::Object | TypeDescriptor::StaticClass => KnownKind::Object,
        TypeDescriptor::Vector(_) | TypeDescriptor::VectorShape { .. } => KnownKind::Vec,
        TypeDescriptor::Dictionary(_) | TypeDescriptor::DictionaryShape { .. } => KnownKind::Dict,
        TypeDescriptor::Tuple(_) | TypeDescriptor::TupleRest { .. } | TypeDescriptor::TupleAny => {
            KnownKind::Tuple
        }
        TypeDescriptor::Callable(_) => KnownKind::Callable,
        TypeDescriptor::Union(members) => common_union_kind(members),
        TypeDescriptor::Intersection(members) => common_intersection_kind(members),
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Named { .. }
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::Array(_)
        | TypeDescriptor::Negated(_) => KnownKind::Unknown,
    }
}

fn common_union_kind(members: &[TypeDescriptor]) -> KnownKind {
    let mut members = members.iter();
    let Some(first) = members.next() else {
        return KnownKind::Unknown;
    };
    let first = descriptor_kind(first);
    if first == KnownKind::Unknown || members.any(|member| descriptor_kind(member) != first) {
        KnownKind::Unknown
    } else {
        first
    }
}

fn common_intersection_kind(members: &[TypeDescriptor]) -> KnownKind {
    let mut kind = KnownKind::Unknown;
    for member in members {
        let member = descriptor_kind(member);
        if member == KnownKind::Unknown {
            continue;
        }
        if kind != KnownKind::Unknown && member != kind {
            return KnownKind::Unknown;
        }
        kind = member;
    }

    kind
}

fn has_no_fallthrough(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Jump { .. }
            | Instruction::NumericRegionJump { .. }
            | Instruction::SwitchInt { .. }
            | Instruction::SwitchString { .. }
            | Instruction::SwitchBool { .. }
            | Instruction::SwitchFloat { .. }
            | Instruction::SwitchPattern { .. }
            | Instruction::SwitchTuplePattern { .. }
            | Instruction::IntRangeJumpIf { .. }
            | Instruction::IntRangeJumpUnless { .. }
            | Instruction::BoolPatternBranch { .. }
            | Instruction::Return { .. }
            | Instruction::ReturnUnchecked { .. }
            | Instruction::ReturnReferenceUnchecked { .. }
            | Instruction::ReturnPairUnchecked { .. }
            | Instruction::ReturnScalarUnchecked { .. }
            | Instruction::ReturnNull
            | Instruction::ReturnNullUnchecked
            | Instruction::ReturnIntUnchecked { .. }
            | Instruction::Throw { .. }
            | Instruction::Rethrow
            | Instruction::ThrowUnhandledMatch { .. }
            | Instruction::Exit { .. }
            | Instruction::Panic { .. }
    )
}
