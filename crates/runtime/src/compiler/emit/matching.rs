//! Lowering for recursive `match` patterns and collection destructuring.

use std::collections::HashSet;
use std::ptr;

use whim_span::HasSpan;
use whim_syn::cst::array::TupleExpression;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::atom::LiteralInteger;
use whim_syn::cst::atom::Variable;
use whim_syn::cst::binding::BindingTarget as BindTarget;
use whim_syn::cst::binding::DictBindingTarget as BindDict;
use whim_syn::cst::binding::ElementBindingTarget as BindElement;
use whim_syn::cst::binding::TupleBindingTarget as BindTuple;
use whim_syn::cst::control_flow::Match;
use whim_syn::cst::control_flow::MatchArm;
use whim_syn::cst::pattern::DictPattern;
use whim_syn::cst::pattern::DictPatternKey;
use whim_syn::cst::pattern::Pattern;
use whim_syn::cst::pattern::TrailingPattern;
use whim_syn::cst::pattern::UnionPattern;
use whim_syn::cst::sequence::TokenSeparatedSequence;
use whim_syn::cst::r#type::NegativeLiteralType;
use whim_syn::cst::r#type::Type;

use crate::bytecode::chunk::descriptors::Literal as BytecodeLiteral;
use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::chunk::descriptors::descriptor_is_trivial;
use crate::bytecode::chunk::descriptors::string_switch_buckets;
use crate::bytecode::instruction::operands::AsMode;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::CompileErrorKind;
use crate::compiler::emit::Count;
use crate::compiler::emit::Expression;
use crate::compiler::emit::ImmediateInt;
use crate::compiler::emit::Instruction;
use crate::compiler::emit::JumpOffset;
use crate::compiler::emit::Register;
use crate::compiler::emit::Scope;
use crate::compiler::emit::Span;
use crate::compiler::emit::TupleElement;
use crate::compiler::emit::check_sequence;
use crate::compiler::emit::check_tuple_sequence;
use crate::compiler::emit::lower_pattern_type;
use crate::compiler::emit::side_table_limit;
use crate::compiler::emit::tuple_index;
use crate::compiler::emit::tuple_window_gate;
use crate::compiler::types::descriptor_is_top;
use crate::unreachable_invariant;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

#[derive(Clone)]
enum MatchKey {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Atom),
}

#[derive(Clone, Copy)]
enum MatchDispatch {
    Int(i64),
    String,
    Float,
    Pattern,
}

#[derive(Clone, Copy)]
struct SwitchLayout<'a> {
    arms: &'a [usize],
    positions: &'a [u32],
    switch: u32,
    default: u32,
}

fn collect_integer_ranges(
    descriptor: &TypeDescriptor,
    ranges: &mut Vec<(Option<i64>, Option<i64>)>,
) -> bool {
    match descriptor {
        TypeDescriptor::IntLiteral(value) => {
            ranges.push((Some(*value), Some(*value)));
            true
        }
        TypeDescriptor::IntRange { min, max } => {
            ranges.push((*min, *max));
            true
        }
        TypeDescriptor::Union(members) => members
            .iter()
            .all(|member| collect_integer_ranges(member, ranges)),
        _ => false,
    }
}

fn tuple_window_descriptor(descriptor: &TypeDescriptor, element_count: usize) -> bool {
    match descriptor {
        TypeDescriptor::Tuple(elements) => {
            elements.len() == element_count && elements.iter().all(descriptor_is_trivial)
        }
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.len() <= element_count
                && elements.iter().all(descriptor_is_trivial)
                && descriptor_is_trivial(rest)
        }
        TypeDescriptor::Union(members) => members
            .iter()
            .any(|member| tuple_window_descriptor(member, element_count)),
        TypeDescriptor::Intersection(members) => members
            .iter()
            .all(|member| tuple_window_descriptor(member, element_count)),
        _ => false,
    }
}

fn collect_bool_tuple_indices(
    descriptor: &TypeDescriptor,
    element_count: usize,
    indices: &mut Vec<usize>,
) -> bool {
    match descriptor {
        TypeDescriptor::Tuple(elements) if elements.len() == element_count => {
            let mut index = 0;
            for (position, element) in elements.iter().enumerate() {
                match element {
                    TypeDescriptor::TrueLiteral => index |= 1 << position,
                    TypeDescriptor::FalseLiteral => {}
                    _ => return false,
                }
            }
            indices.push(index);
            true
        }
        TypeDescriptor::Union(members) => {
            let mut found = false;
            for member in members {
                if !tuple_window_descriptor(member, element_count) {
                    continue;
                }
                if !collect_bool_tuple_indices(member, element_count, indices) {
                    return false;
                }
                found = true;
            }
            found
        }
        _ => false,
    }
}

fn same_shape_key(left: &ShapeKey, right: &ShapeKey) -> bool {
    match (left, right) {
        (ShapeKey::Int(left), ShapeKey::Int(right)) => left == right,
        (ShapeKey::String(left), ShapeKey::String(right)) => left.as_bytes() == right.as_bytes(),
        _ => false,
    }
}

fn dictionary_shape_table(
    descriptors: &[TypeDescriptor],
    targets: Vec<i32>,
    default: i32,
) -> Option<SwitchTable> {
    let TypeDescriptor::DictionaryShape {
        entries: first,
        rest: None,
    } = descriptors.first()?
    else {
        return None;
    };
    if first.is_empty() || first.len() > 8 {
        return None;
    }

    let keys = first.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>();
    let mut patterns = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        let TypeDescriptor::DictionaryShape {
            entries,
            rest: None,
        } = descriptor
        else {
            return None;
        };
        if entries.len() != keys.len() {
            return None;
        }
        let mut pattern = Vec::with_capacity(keys.len());
        for key in &keys {
            let (_, value) = entries
                .iter()
                .find(|(candidate, _)| same_shape_key(candidate, key))?;
            pattern.push(value.clone());
        }
        patterns.push(pattern);
    }

    Some(SwitchTable::DictionaryShape {
        keys,
        patterns,
        targets,
        default,
    })
}

fn collect_match_keys(heap: &Heap, pattern: &Pattern<'_>, keys: &mut Vec<MatchKey>) -> bool {
    match pattern {
        Pattern::Parenthesized(pattern) => collect_match_keys(heap, pattern.pattern, keys),
        Pattern::As(pattern) if pattern_only_binds(pattern.left) => {
            collect_match_keys(heap, pattern.right, keys)
        }
        Pattern::As(pattern) if pattern_only_binds(pattern.right) => {
            collect_match_keys(heap, pattern.left, keys)
        }
        Pattern::As(_)
        | Pattern::Variable(_)
        | Pattern::Vec(_)
        | Pattern::Dict(_)
        | Pattern::Tuple(_) => false,
        Pattern::Union(pattern) => {
            collect_match_keys(heap, pattern.left, keys)
                && collect_match_keys(heap, pattern.right, keys)
        }
        Pattern::Type(r#type) => match r#type.unparenthesized() {
            Type::Literal(Literal::Null(_)) => {
                keys.push(MatchKey::Null);
                true
            }
            Type::Literal(Literal::True(_)) => {
                keys.push(MatchKey::Bool(true));
                true
            }
            Type::Literal(Literal::False(_)) => {
                keys.push(MatchKey::Bool(false));
                true
            }
            Type::Literal(Literal::Integer(integer)) => i64::try_from(integer.value)
                .map(|value| keys.push(MatchKey::Int(value)))
                .is_ok(),
            Type::Literal(Literal::Float(float)) => {
                keys.push(MatchKey::Float(float.value));
                true
            }
            Type::Literal(Literal::String(string)) => {
                keys.push(MatchKey::String(heap.intern(string.value)));
                true
            }
            Type::NegativeLiteral(NegativeLiteralType::Integer { literal, .. }) => {
                let Ok(value) = i64::try_from(-i128::from(literal.value)) else {
                    return false;
                };
                keys.push(MatchKey::Int(value));
                true
            }
            Type::NegativeLiteral(NegativeLiteralType::Float { literal, .. }) => {
                keys.push(MatchKey::Float(-literal.value));
                true
            }
            _ => false,
        },
    }
}

fn pattern_only_binds(pattern: &Pattern<'_>) -> bool {
    match pattern {
        Pattern::Variable(_) => true,
        Pattern::Parenthesized(pattern) => pattern_only_binds(pattern.pattern),
        Pattern::As(pattern) => {
            pattern_only_binds(pattern.left) && pattern_only_binds(pattern.right)
        }
        Pattern::Union(_)
        | Pattern::Vec(_)
        | Pattern::Dict(_)
        | Pattern::Tuple(_)
        | Pattern::Type(_) => false,
    }
}

fn relative_target(switch_position: u32, target: u32) -> i32 {
    let Ok(relative) = i32::try_from(i64::from(target) - i64::from(switch_position)) else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a chunk stays within the thirty-two-bit index space") }
    };
    relative
}

fn byte_switch_table(entries: Vec<(Atom, i32)>, default: i32) -> SwitchTable {
    let base = entries
        .iter()
        .map(|(value, _)| value.as_bytes()[0])
        .min()
        .unwrap_or(0);
    let end = entries
        .iter()
        .map(|(value, _)| value.as_bytes()[0])
        .max()
        .unwrap_or(base);
    let mut targets = vec![default; usize::from(end - base) + 1];
    for (value, target) in entries {
        targets[usize::from(value.as_bytes()[0] - base)] = target;
    }

    SwitchTable::StringByte {
        base,
        targets,
        default,
    }
}

fn dict_pattern_integer(minus: bool, integer: &LiteralInteger<'_>) -> Option<i64> {
    let value = i128::from(integer.value);
    let value = if minus { -value } else { value };
    i64::try_from(value).ok()
}

fn check_bind_tuple(tuple: &BindTuple<'_>) -> Result<(), CompileError> {
    check_tuple_sequence(
        CompileErrorKind::TooManyTupleElements,
        "a binding pattern may have",
        "targets",
        &tuple.targets,
    )?;
    let mut seen_rest = false;
    for element in &tuple.targets {
        if seen_rest {
            return Err(CompileError::new(
                CompileErrorKind::TargetAfterRest,
                "no bind target can follow a `...` rest",
                element.span(),
            ));
        }
        match element {
            BindElement::Target(target) => check_bind_target(target)?,
            BindElement::Rest(rest) => {
                seen_rest = true;
                if let Some(target) = &rest.target {
                    check_bind_target(target)?;
                }
            }
        }
    }
    Ok(())
}

pub(in crate::compiler::emit) fn check_bind_target(
    target: &BindTarget<'_>,
) -> Result<(), CompileError> {
    match target {
        BindTarget::Variable(_) => Ok(()),
        BindTarget::Tuple(tuple) => check_bind_tuple(tuple),
        BindTarget::Dict(dict) => {
            for entry in &dict.entries {
                check_bind_target(&entry.target)?;
            }
            Ok(())
        }
    }
}

impl BodyCompiler<'_, '_> {
    /// Compiles a match through its cheapest valid dispatch.
    pub(in crate::compiler::emit) fn matching(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
    ) -> Result<Register, CompileError> {
        let mut keys = Vec::new();
        let mut foldable = true;
        for (index, arm) in matching.arms.iter().enumerate() {
            check_pattern(arm.pattern)?;
            if self.pattern_is_irrefutable(scope, arm.pattern)? {
                if index + 1 != matching.arms.len() {
                    return Err(CompileError::new(
                        CompileErrorKind::UnreachableMatchArm,
                        "a match arm cannot follow a pattern that accepts every value",
                        matching.arms.as_slice()[index + 1].pattern.span(),
                    ));
                }
                continue;
            }
            if !collect_match_keys(self.heap, arm.pattern, &mut keys) {
                foldable = false;
            }
            if pattern_needs_split_bindings(arm.pattern) {
                foldable = false;
            }
        }

        if let Some(result) = self.try_tuple_match(scope, matching)? {
            return Ok(result);
        }
        if let Some(result) = self.try_key_match(scope, matching, &keys, foldable)? {
            return Ok(result);
        }
        if let Some(result) = self.try_trivial_match(scope, matching)? {
            return Ok(result);
        }

        self.match_chain(scope, matching)
    }

    fn try_tuple_match(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
    ) -> Result<Option<Register>, CompileError> {
        let Expression::Tuple(tuple) = matching.expression else {
            return Ok(None);
        };
        if !tuple
            .elements
            .iter()
            .all(|element| matches!(element, TupleElement::Value(_)))
            || matching
                .arms
                .iter()
                .any(|arm| pattern_has_bindings(arm.pattern))
        {
            return Ok(None);
        }

        let element_count = tuple.elements.len();
        if element_count != 0 && element_count <= 4 {
            let mut bool_tuple = true;
            for arm in &matching.arms {
                if self.pattern_is_irrefutable(scope, arm.pattern)? {
                    continue;
                }
                let descriptor = self.lower_match_pattern(scope, arm.pattern)?;
                bool_tuple &=
                    collect_bool_tuple_indices(&descriptor, element_count, &mut Vec::new());
            }
            if bool_tuple {
                return self
                    .match_bool_tuple_literal(scope, matching, tuple)
                    .map(Some);
            }
        }

        for arm in &matching.arms {
            if self.pattern_is_irrefutable(scope, arm.pattern)? {
                continue;
            }
            let descriptor = self.lower_match_pattern(scope, arm.pattern)?;
            if !tuple_window_descriptor(&descriptor, element_count) {
                return Ok(None);
            }
        }
        self.match_tuple_literal(scope, matching, tuple).map(Some)
    }

    fn try_key_match(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
        keys: &[MatchKey],
        foldable: bool,
    ) -> Result<Option<Register>, CompileError> {
        if !foldable || keys.is_empty() {
            return Ok(None);
        }
        if keys.iter().all(|key| matches!(key, MatchKey::String(_))) {
            return self
                .match_with_switch(scope, matching, MatchDispatch::String)
                .map(Some);
        }
        if keys.iter().all(|key| matches!(key, MatchKey::Bool(_))) {
            return self.match_with_bool_switch(scope, matching).map(Some);
        }
        if keys.iter().all(|key| matches!(key, MatchKey::Float(_))) {
            return self
                .match_with_switch(scope, matching, MatchDispatch::Float)
                .map(Some);
        }
        if !keys.iter().all(|key| matches!(key, MatchKey::Int(_))) {
            return Ok(None);
        }

        let mut values = keys
            .iter()
            .filter_map(|key| match key {
                MatchKey::Int(value) => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>();
        let Some(minimum) = values.iter().copied().min() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("integer match keys are non-empty") }
        };
        let Some(maximum) = values.iter().copied().max() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("integer match keys are non-empty") }
        };
        let span_width = i128::from(maximum) - i128::from(minimum) + 1;
        values.sort_unstable();
        values.dedup();
        if values.len() < 8 || span_width > 3 * values.len() as i128 {
            return Ok(None);
        }
        self.match_with_switch(scope, matching, MatchDispatch::Int(minimum))
            .map(Some)
    }

    fn try_trivial_match(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
    ) -> Result<Option<Register>, CompileError> {
        let mut integer_ranges = true;
        let mut count = 0;
        for arm in &matching.arms {
            if self.pattern_is_irrefutable(scope, arm.pattern)? {
                continue;
            }
            count += 1;
            let descriptor = self.lower_match_pattern(scope, arm.pattern)?;
            if pattern_needs_split_bindings(arm.pattern) || !descriptor_is_trivial(&descriptor) {
                return Ok(None);
            }
            integer_ranges &= collect_integer_ranges(&descriptor, &mut Vec::new());
        }
        if count != 0 && integer_ranges {
            return self.match_integer_patterns(scope, matching).map(Some);
        }
        self.match_with_switch(scope, matching, MatchDispatch::Pattern)
            .map(Some)
    }

    /// Compiles every arm in source order. A type pattern emits an `is` test;
    /// a variable is the fallback arm. Bindings are local to the selected arm.
    fn match_chain(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
    ) -> Result<Register, CompileError> {
        let result = self.allocate(matching.span())?;
        let subject = self.allocate(matching.span())?;
        let evaluated = self.expression(scope, matching.expression)?;
        self.move_into(subject, evaluated, matching.expression.span());
        let comparison = self.allocate(matching.span())?;
        let mut jumps = Vec::new();
        let mut default_arm = None;

        for (index, arm) in matching.arms.iter().enumerate() {
            check_pattern(arm.pattern)?;
            if self.pattern_is_irrefutable(scope, arm.pattern)? {
                if index + 1 != matching.arms.len() {
                    return Err(CompileError::new(
                        CompileErrorKind::UnreachableMatchArm,
                        "a match arm cannot follow a pattern that accepts every value",
                        matching.arms.as_slice()[index + 1].pattern.span(),
                    ));
                }
                default_arm = Some(arm);
                continue;
            }
            let mut alternatives = Vec::new();
            if pattern_needs_split_bindings(arm.pattern) {
                pattern_alternatives(arm.pattern, &mut alternatives);
            } else {
                alternatives.push(arm.pattern);
            }
            for pattern in alternatives {
                let descriptor = self.lower_match_pattern(scope, pattern)?;
                let mut keys = Vec::new();
                if collect_match_keys(self.heap, pattern, &mut keys) {
                    for key in keys {
                        let mark = self.registers.mark();
                        let value = self.load_match_key(&key, pattern.span())?;
                        self.chunk.emit(
                            Instruction::Equal {
                                destination: comparison,
                                left: subject,
                                right: value,
                            },
                            pattern.span(),
                        );
                        jumps.push((
                            self.chunk.emit(
                                Instruction::JumpIfTrue {
                                    condition: comparison,
                                    offset: JumpOffset::new(0),
                                },
                                pattern.span(),
                            ),
                            arm,
                            pattern,
                        ));
                        self.registers.release_to(mark);
                    }
                } else {
                    let descriptor = self.add_type_descriptor(descriptor, pattern.span())?;
                    self.chunk.emit(
                        Instruction::Is {
                            destination: comparison,
                            source: subject,
                            descriptor,
                        },
                        pattern.span(),
                    );
                    jumps.push((
                        self.chunk.emit(
                            Instruction::JumpIfTrue {
                                condition: comparison,
                                offset: JumpOffset::new(0),
                            },
                            pattern.span(),
                        ),
                        arm,
                        pattern,
                    ));
                }
            }
        }

        self.emit_match_chain_arms(scope, matching.span(), subject, result, &jumps, default_arm)?;

        Ok(result)
    }

    fn emit_match_chain_arms(
        &mut self,
        scope: &Scope<'_>,
        span: Span,
        subject: Register,
        result: Register,
        jumps: &[(u32, &MatchArm<'_>, &Pattern<'_>)],
        default_arm: Option<&MatchArm<'_>>,
    ) -> Result<(), CompileError> {
        let mut exits = Vec::new();
        if let Some(arm) = default_arm {
            self.emit_match_arm(scope, subject, result, arm, &mut exits)?;
        } else {
            self.chunk
                .emit(Instruction::ThrowUnhandledMatch { subject }, span);
        }
        let mut seen = HashSet::new();
        for (_, arm, pattern) in jumps {
            let arm_pointer = ptr::from_ref(*arm);
            let pattern_pointer = ptr::from_ref(*pattern);
            if !seen.insert((arm_pointer, pattern_pointer)) {
                continue;
            }
            let target = self.code_position();
            for (jump, candidate, candidate_pattern) in jumps {
                if ptr::eq(ptr::from_ref(*candidate), arm_pointer)
                    && ptr::eq(ptr::from_ref(*candidate_pattern), pattern_pointer)
                {
                    self.chunk.patch_jump(*jump, target);
                }
            }
            self.emit_match_arm_pattern(scope, subject, result, arm, pattern, &mut exits)?;
        }

        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }

        Ok(())
    }

    fn load_match_key(&mut self, key: &MatchKey, span: Span) -> Result<Register, CompileError> {
        let destination = self.allocate(span)?;
        match key {
            MatchKey::Null => {
                self.chunk.emit(Instruction::LoadNull { destination }, span);
            }
            MatchKey::Bool(true) => {
                self.chunk.emit(Instruction::LoadTrue { destination }, span);
            }
            MatchKey::Bool(false) => {
                self.chunk
                    .emit(Instruction::LoadFalse { destination }, span);
            }
            MatchKey::Int(value) => self.load_integer(destination, *value, span)?,
            MatchKey::Float(value) => {
                let constant = self.add_constant(BytecodeLiteral::Float(*value), span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination,
                        constant,
                    },
                    span,
                );
            }
            MatchKey::String(value) => {
                let constant = self.add_constant(BytecodeLiteral::String(value.clone()), span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination,
                        constant,
                    },
                    span,
                );
            }
        }
        Ok(destination)
    }

    fn match_tuple_literal(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
        tuple: &TupleExpression<'_>,
    ) -> Result<Register, CompileError> {
        let irrefutable = matching
            .arms
            .iter()
            .map(|arm| self.pattern_is_irrefutable(scope, arm.pattern))
            .collect::<Result<Vec<_>, _>>()?;
        let default_arm = matching
            .arms
            .iter()
            .zip(irrefutable.iter())
            .find(|(_, irrefutable)| **irrefutable)
            .map(|(arm, _)| arm);
        let result = self.allocate(matching.span())?;
        let throw_subject = if default_arm.is_none() {
            Some(self.allocate(matching.expression.span())?)
        } else {
            None
        };
        let mark = self.registers.mark();
        let values = tuple.elements.iter().map(|element| match element {
            TupleElement::Value(value) => *value,
            // SAFETY: the surrounding invariant makes this path unreachable.
            TupleElement::Rest(_) => unsafe {
                unreachable_invariant("a tuple match window contains no spread")
            },
        });
        let first = self.window(scope, values, tuple.elements.len(), tuple.span())?;
        let table = self
            .chunk
            .add_switch_table(SwitchTable::Pattern {
                descriptors: Vec::new(),
                targets: Vec::new(),
                default: 0,
            })
            .map_err(|full| side_table_limit(full, matching.span()))?;
        let switch_position = self.chunk.emit(
            Instruction::SwitchTuplePattern {
                first_element: first,
                element_count: Count::new(tuple_window_gate(tuple.elements.len(), tuple.span())?),
                table,
            },
            matching.expression.span(),
        );
        let default_position = self.code_position();
        let mut exits = Vec::new();
        if let Some(arm) = default_arm {
            self.emit_match_arm(scope, Register::NONE, result, arm, &mut exits)?;
        } else {
            let Some(subject) = throw_subject else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a non-exhaustive tuple match keeps its subject") }
            };
            self.chunk.emit(
                Instruction::NewTuple {
                    element_count: Count::new(tuple_window_gate(
                        tuple.elements.len(),
                        tuple.span(),
                    )?),
                    destination: subject,
                    first_element: first,
                },
                tuple.span(),
            );
            self.chunk.emit(
                Instruction::ThrowUnhandledMatch { subject },
                matching.span(),
            );
        }

        let non_irrefutable = irrefutable
            .iter()
            .enumerate()
            .filter_map(|(index, irrefutable)| (!*irrefutable).then_some(index))
            .collect::<Vec<_>>();
        let mut arm_positions = Vec::with_capacity(non_irrefutable.len());
        for index in &non_irrefutable {
            let arm = &matching.arms.as_slice()[*index];
            arm_positions.push(self.code_position());
            self.emit_match_arm(scope, Register::NONE, result, arm, &mut exits)?;
        }

        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }
        self.chunk.switch_tables[usize::from(table.index())] = self.build_pattern_switch_table(
            scope,
            matching,
            &non_irrefutable,
            &arm_positions,
            switch_position,
            default_position,
        )?;
        self.registers.release_to(mark);
        Ok(result)
    }

    fn match_bool_tuple_literal(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
        tuple: &TupleExpression<'_>,
    ) -> Result<Register, CompileError> {
        let result = self.allocate(matching.span())?;
        let mut outcomes = vec![None; 1 << tuple.elements.len()];
        let mut default = None;
        for (index, arm) in matching.arms.iter().enumerate() {
            if self.pattern_is_irrefutable(scope, arm.pattern)? {
                default = Some(index);
                continue;
            }
            let descriptor = self.lower_match_pattern(scope, arm.pattern)?;
            let mut indices = Vec::new();
            if !collect_bool_tuple_indices(&descriptor, tuple.elements.len(), &mut indices) {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a bool tuple match contains bool tuple patterns") }
            }
            for selected in indices {
                outcomes[selected].get_or_insert(index);
            }
        }
        if let Some(default) = default {
            for outcome in &mut outcomes {
                outcome.get_or_insert(default);
            }
        }

        let throw_subject = if default.is_none() {
            Some(self.allocate(matching.expression.span())?)
        } else {
            None
        };
        let mark = self.registers.mark();
        let mut elements = Vec::with_capacity(tuple.elements.len());
        for element in &tuple.elements {
            let TupleElement::Value(value) = element else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a bool tuple match contains no spread") }
            };
            elements.push(self.expression(scope, value)?);
        }
        let mut exits = Vec::new();
        let mut branches = Vec::new();
        let mut unmatched = Vec::new();
        self.emit_bool_tuple_tree(
            scope,
            matching,
            &outcomes,
            &elements,
            0,
            0,
            result,
            &mut exits,
            &mut branches,
            &mut unmatched,
        )?;

        let default_position = self.code_position();
        if let Some(default) = default {
            self.emit_match_arm(
                scope,
                Register::NONE,
                result,
                &matching.arms.as_slice()[default],
                &mut exits,
            )?;
        } else {
            let Some(subject) = throw_subject else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe {
                    unreachable_invariant("a bool tuple match without a default keeps a tuple")
                }
            };
            self.emit_unhandled_bool_tuple(matching, tuple, &elements, subject)?;
        }
        for jump in unmatched {
            self.chunk.patch_jump(jump, default_position);
        }
        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }
        for (position, subject, false_position) in branches {
            let false_offset = relative_target(position, false_position);
            let default_offset = relative_target(position, default_position);
            self.install_bool_branch(
                position,
                subject,
                false_offset,
                default_offset,
                matching.span(),
            )?;
        }
        self.registers.release_to(mark);
        Ok(result)
    }

    fn emit_unhandled_bool_tuple(
        &mut self,
        matching: &Match<'_>,
        tuple: &TupleExpression<'_>,
        elements: &[Register],
        subject: Register,
    ) -> Result<(), CompileError> {
        let mut slots = Vec::with_capacity(elements.len());
        for _ in elements {
            slots.push(self.allocate(tuple.span())?);
        }
        let first = slots
            .first()
            .copied()
            .unwrap_or_else(|| Register::new(self.registers.mark()));
        for (slot, element) in slots.into_iter().zip(elements) {
            self.move_into(slot, *element, tuple.span());
        }
        self.chunk.emit(
            Instruction::NewTuple {
                element_count: Count::new(tuple_window_gate(tuple.elements.len(), tuple.span())?),
                destination: subject,
                first_element: first,
            },
            tuple.span(),
        );
        self.chunk.emit(
            Instruction::ThrowUnhandledMatch { subject },
            matching.span(),
        );
        Ok(())
    }

    fn install_bool_branch(
        &mut self,
        position: u32,
        subject: Register,
        false_offset: i32,
        default_offset: i32,
        span: Span,
    ) -> Result<(), CompileError> {
        if let (Ok(false_offset), Ok(default_offset)) =
            (i16::try_from(false_offset), i16::try_from(default_offset))
        {
            self.chunk.code[position as usize] = Instruction::BoolPatternBranch {
                subject,
                false_offset: ShortJumpOffset::new(false_offset),
                default_offset: ShortJumpOffset::new(default_offset),
            };
            return Ok(());
        }

        let table = self
            .chunk
            .add_switch_table(SwitchTable::Bool {
                targets: vec![false_offset, 1],
                default: default_offset,
            })
            .map_err(|full| side_table_limit(full, span))?;
        self.chunk.code[position as usize] = Instruction::SwitchBool { subject, table };
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the recursive emitter carries one match tree's state"
    )]
    fn emit_bool_tuple_tree(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
        outcomes: &[Option<usize>],
        elements: &[Register],
        depth: usize,
        index: usize,
        result: Register,
        exits: &mut Vec<u32>,
        branches: &mut Vec<(u32, Register, u32)>,
        unmatched: &mut Vec<u32>,
    ) -> Result<(), CompileError> {
        if 1usize << depth == outcomes.len() {
            if let Some(arm) = outcomes[index] {
                self.emit_match_arm(
                    scope,
                    Register::NONE,
                    result,
                    &matching.arms.as_slice()[arm],
                    exits,
                )?;
            } else {
                unmatched.push(self.chunk.emit(
                    Instruction::Jump {
                        offset: JumpOffset::new(0),
                    },
                    matching.span(),
                ));
            }
            return Ok(());
        }

        let subject = elements[depth];
        let branch = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            matching.expression.span(),
        );
        self.emit_bool_tuple_tree(
            scope,
            matching,
            outcomes,
            elements,
            depth + 1,
            index | (1 << depth),
            result,
            exits,
            branches,
            unmatched,
        )?;
        let false_position = self.code_position();
        self.emit_bool_tuple_tree(
            scope,
            matching,
            outcomes,
            elements,
            depth + 1,
            index,
            result,
            exits,
            branches,
            unmatched,
        )?;
        branches.push((branch, subject, false_position));
        Ok(())
    }

    fn match_with_bool_switch(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
    ) -> Result<Register, CompileError> {
        let result = self.allocate(matching.span())?;
        let subject = self.expression(scope, matching.expression)?;
        let mut selected = [None; 2];
        let mut default = None;
        for (index, arm) in matching.arms.iter().enumerate() {
            if self.pattern_is_irrefutable(scope, arm.pattern)? {
                default = Some(index);
                continue;
            }
            for key in self.switch_arm_keys(matching, index) {
                let MatchKey::Bool(value) = key else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("a bool match contains only bool keys") }
                };
                selected[usize::from(value)].get_or_insert(index);
            }
        }

        let switch_position = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            matching.expression.span(),
        );
        let outcomes = [selected[1].or(default), selected[0].or(default), default];
        let mut positions = Vec::with_capacity(3);
        let mut exits = Vec::new();
        for outcome in outcomes {
            if positions.iter().any(|(emitted, _)| *emitted == outcome) {
                continue;
            }
            let position = self.code_position();
            if let Some(index) = outcome {
                self.emit_match_arm(
                    scope,
                    subject,
                    result,
                    &matching.arms.as_slice()[index],
                    &mut exits,
                )?;
            } else {
                self.chunk.emit(
                    Instruction::ThrowUnhandledMatch { subject },
                    matching.span(),
                );
            }
            positions.push((outcome, position));
        }
        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }

        let position = |outcome| {
            positions
                .iter()
                .find_map(|(emitted, position)| (*emitted == outcome).then_some(*position))
                // SAFETY: the surrounding invariant makes this path unreachable.
                .unwrap_or_else(|| unsafe {
                    unreachable_invariant("every bool outcome has emitted code")
                })
        };
        let false_offset = relative_target(switch_position, position(outcomes[1]));
        let default_offset = relative_target(switch_position, position(outcomes[2]));
        self.install_bool_branch(
            switch_position,
            subject,
            false_offset,
            default_offset,
            matching.span(),
        )?;
        Ok(result)
    }

    fn match_integer_patterns(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
    ) -> Result<Register, CompileError> {
        let result = self.allocate(matching.span())?;
        let subject = self.expression(scope, matching.expression)?;
        let comparison = self.allocate(matching.span())?;
        let mut exits = Vec::new();
        let mut default = None;

        for arm in &matching.arms {
            if self.pattern_is_irrefutable(scope, arm.pattern)? {
                default = Some(arm);
                break;
            }
            let descriptor = self.lower_match_pattern(scope, arm.pattern)?;
            let mut ranges = Vec::new();
            if !collect_integer_ranges(&descriptor, &mut ranges) {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("an integer match contains only integer patterns") }
            }
            let mut accepted = Vec::with_capacity(ranges.len().saturating_sub(1));
            let mut rejected = None;
            for (index, range) in ranges.iter().enumerate() {
                self.emit_integer_pattern_check(subject, comparison, *range, arm.pattern.span())?;
                let last = index + 1 == ranges.len();
                let jump = self.chunk.emit(
                    if last {
                        Instruction::JumpIfFalse {
                            condition: comparison,
                            offset: JumpOffset::new(0),
                        }
                    } else {
                        Instruction::JumpIfTrue {
                            condition: comparison,
                            offset: JumpOffset::new(0),
                        }
                    },
                    arm.pattern.span(),
                );
                if last {
                    rejected = Some(jump);
                } else {
                    accepted.push(jump);
                }
            }
            let body = self.code_position();
            for jump in accepted {
                self.chunk.patch_jump(jump, body);
            }
            self.emit_match_arm(scope, subject, result, arm, &mut exits)?;
            let next = self.code_position();
            let Some(rejected) = rejected else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("an integer pattern has at least one range") }
            };
            self.chunk.patch_jump(rejected, next);
        }

        if let Some(arm) = default {
            self.emit_match_arm(scope, subject, result, arm, &mut exits)?;
        } else {
            self.chunk.emit(
                Instruction::ThrowUnhandledMatch { subject },
                matching.span(),
            );
        }
        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }
        Ok(result)
    }

    fn emit_integer_pattern_check(
        &mut self,
        subject: Register,
        destination: Register,
        (minimum, maximum): (Option<i64>, Option<i64>),
        span: Span,
    ) -> Result<(), CompileError> {
        if minimum == maximum
            && let Some(value) = minimum
        {
            let mark = self.registers.mark();
            let expected = self.allocate(span)?;
            self.load_integer(expected, value, span)?;
            self.chunk.emit(
                Instruction::Equal {
                    destination,
                    left: subject,
                    right: expected,
                },
                span,
            );
            self.registers.release_to(mark);
            return Ok(());
        }

        let descriptor = self.add_type_descriptor(
            TypeDescriptor::IntRange {
                min: minimum,
                max: maximum,
            },
            span,
        )?;
        self.chunk.emit(
            Instruction::Is {
                destination,
                source: subject,
                descriptor,
            },
            span,
        );
        Ok(())
    }

    fn match_with_switch(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
        dispatch: MatchDispatch,
    ) -> Result<Register, CompileError> {
        let result = self.allocate(matching.span())?;
        let subject = self.expression(scope, matching.expression)?;
        let table = self
            .chunk
            .add_switch_table(SwitchTable::Int {
                base: 0,
                targets: Vec::new(),
                default: 0,
            })
            .map_err(|full| side_table_limit(full, matching.span()))?;
        let instruction = match dispatch {
            MatchDispatch::Int(_) => Instruction::SwitchInt { subject, table },
            MatchDispatch::String => Instruction::SwitchString { subject, table },
            MatchDispatch::Float => Instruction::SwitchFloat { subject, table },
            MatchDispatch::Pattern => Instruction::SwitchPattern { subject, table },
        };
        let switch_position = self.chunk.emit(instruction, matching.expression.span());
        let default_position = self.code_position();
        let mut exits = Vec::new();
        let irrefutable = matching
            .arms
            .iter()
            .map(|arm| self.pattern_is_irrefutable(scope, arm.pattern))
            .collect::<Result<Vec<_>, _>>()?;
        let non_irrefutable = irrefutable
            .iter()
            .enumerate()
            .filter_map(|(index, irrefutable)| (!*irrefutable).then_some(index))
            .collect::<Vec<_>>();

        if let Some((arm, _)) = matching
            .arms
            .iter()
            .zip(irrefutable.iter())
            .find(|(_, irrefutable)| **irrefutable)
        {
            self.emit_match_arm(scope, subject, result, arm, &mut exits)?;
        } else {
            self.chunk.emit(
                Instruction::ThrowUnhandledMatch { subject },
                matching.span(),
            );
        }

        let mut arm_positions = Vec::new();
        for index in &non_irrefutable {
            let arm = &matching.arms.as_slice()[*index];
            arm_positions.push(self.code_position());
            self.emit_match_arm(scope, subject, result, arm, &mut exits)?;
        }

        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }

        let built = self.build_match_switch_table(
            scope,
            matching,
            dispatch,
            SwitchLayout {
                arms: &non_irrefutable,
                positions: &arm_positions,
                switch: switch_position,
                default: default_position,
            },
        )?;
        self.chunk.switch_tables[usize::from(table.index())] = built;
        Ok(result)
    }

    fn build_match_switch_table(
        &self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
        dispatch: MatchDispatch,
        layout: SwitchLayout<'_>,
    ) -> Result<SwitchTable, CompileError> {
        Ok(match dispatch {
            MatchDispatch::Int(base) => self.build_int_switch_table(
                matching,
                base,
                layout.arms,
                layout.positions,
                layout.switch,
                layout.default,
            ),
            MatchDispatch::String => self.build_string_switch_table(
                matching,
                layout.arms,
                layout.positions,
                layout.switch,
                layout.default,
            ),
            MatchDispatch::Float => self.build_float_switch_table(
                matching,
                layout.arms,
                layout.positions,
                layout.switch,
                layout.default,
            ),
            MatchDispatch::Pattern => self.build_pattern_switch_table(
                scope,
                matching,
                layout.arms,
                layout.positions,
                layout.switch,
                layout.default,
            )?,
        })
    }

    fn build_int_switch_table(
        &self,
        matching: &Match<'_>,
        base: i64,
        arms: &[usize],
        arm_positions: &[u32],
        switch_position: u32,
        default_position: u32,
    ) -> SwitchTable {
        let mut extent = 0_i128;
        for index in arms {
            for key in self.switch_arm_keys(matching, *index) {
                let MatchKey::Int(value) = key else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("every switch key is an integer") }
                };
                extent = extent.max(i128::from(value) - i128::from(base));
            }
        }
        let Ok(width) = usize::try_from(extent + 1) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the density heuristic bounds the table width") }
        };
        let mut targets = vec![relative_target(switch_position, default_position); width];
        let mut filled = vec![false; width];
        for (position, index) in arms.iter().enumerate() {
            for key in self.switch_arm_keys(matching, *index) {
                let MatchKey::Int(value) = key else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("every switch key is an integer") }
                };
                let Ok(offset) = usize::try_from(i128::from(value) - i128::from(base)) else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("the switch base is its minimum key") }
                };
                if !filled[offset] {
                    filled[offset] = true;
                    targets[offset] = relative_target(switch_position, arm_positions[position]);
                }
            }
        }

        SwitchTable::Int {
            base,
            targets,
            default: relative_target(switch_position, default_position),
        }
    }

    fn build_string_switch_table(
        &self,
        matching: &Match<'_>,
        arms: &[usize],
        arm_positions: &[u32],
        switch_position: u32,
        default_position: u32,
    ) -> SwitchTable {
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        for (position, index) in arms.iter().enumerate() {
            for key in self.switch_arm_keys(matching, *index) {
                let MatchKey::String(value) = key else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("every switch key is a string") }
                };
                if seen.insert(value.clone()) {
                    entries.push((
                        value,
                        relative_target(switch_position, arm_positions[position]),
                    ));
                }
            }
        }
        let default = relative_target(switch_position, default_position);
        if entries.iter().all(|(value, _)| value.as_bytes().len() == 1) {
            return byte_switch_table(entries, default);
        }
        let buckets = string_switch_buckets(&entries);

        SwitchTable::String {
            arms: entries,
            buckets,
            default,
        }
    }

    fn build_float_switch_table(
        &self,
        matching: &Match<'_>,
        arms: &[usize],
        arm_positions: &[u32],
        switch_position: u32,
        default_position: u32,
    ) -> SwitchTable {
        let mut values = Vec::new();
        let mut targets = Vec::new();
        for (position, index) in arms.iter().enumerate() {
            for key in self.switch_arm_keys(matching, *index) {
                let MatchKey::Float(value) = key else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("every float switch key is a float") }
                };
                values.push(value);
                targets.push(relative_target(switch_position, arm_positions[position]));
            }
        }

        SwitchTable::Float {
            values,
            targets,
            default: relative_target(switch_position, default_position),
        }
    }

    fn build_pattern_switch_table(
        &self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
        arms: &[usize],
        arm_positions: &[u32],
        switch_position: u32,
        default_position: u32,
    ) -> Result<SwitchTable, CompileError> {
        let mut descriptors = Vec::with_capacity(arms.len());
        let mut targets = Vec::with_capacity(arms.len());
        for (position, index) in arms.iter().enumerate() {
            let pattern = matching.arms.as_slice()[*index].pattern;
            descriptors.push(self.lower_match_pattern(scope, pattern)?);
            targets.push(relative_target(switch_position, arm_positions[position]));
        }

        let default = relative_target(switch_position, default_position);
        if let Some(table) = dictionary_shape_table(&descriptors, targets.clone(), default) {
            return Ok(table);
        }

        Ok(SwitchTable::Pattern {
            descriptors,
            targets,
            default,
        })
    }

    fn switch_arm_keys(&self, matching: &Match<'_>, index: usize) -> Vec<MatchKey> {
        let mut keys = Vec::new();
        if !collect_match_keys(
            self.heap,
            matching.arms.as_slice()[index].pattern,
            &mut keys,
        ) {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("switch arms contain only literal patterns") }
        }
        keys
    }

    fn emit_match_arm(
        &mut self,
        scope: &Scope<'_>,
        subject: Register,
        result: Register,
        arm: &MatchArm<'_>,
        exits: &mut Vec<u32>,
    ) -> Result<(), CompileError> {
        self.emit_match_arm_pattern(scope, subject, result, arm, arm.pattern, exits)
    }

    fn emit_match_arm_pattern(
        &mut self,
        scope: &Scope<'_>,
        subject: Register,
        result: Register,
        arm: &MatchArm<'_>,
        pattern: &Pattern<'_>,
        exits: &mut Vec<u32>,
    ) -> Result<(), CompileError> {
        let saved = self.save_defined();
        let mark = self.registers.mark();
        let local_count = self.push_pattern_bindings(pattern)?;
        self.bind_pattern(scope, pattern, subject)?;
        self.registers.release_to(mark);
        let value = self.expression(scope, arm.expression)?;
        self.move_into(result, value, arm.expression.span());
        self.truncate_locals(local_count);
        self.restore_defined(saved);
        exits.push(self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            arm.expression.span(),
        ));
        Ok(())
    }

    fn pattern_is_irrefutable(
        &self,
        scope: &Scope<'_>,
        pattern: &Pattern<'_>,
    ) -> Result<bool, CompileError> {
        match pattern {
            Pattern::Variable(_) => Ok(true),
            Pattern::Parenthesized(pattern) => self.pattern_is_irrefutable(scope, pattern.pattern),
            Pattern::As(pattern) => {
                let left = self.lower_match_pattern(scope, pattern.left)?;
                let right = self.lower_match_pattern(scope, pattern.right)?;
                Ok(descriptor_is_top(
                    &TypeDescriptor::Intersection(vec![left, right]),
                    0,
                ))
            }
            Pattern::Union(pattern) => {
                let left = self.lower_match_pattern(scope, pattern.left)?;
                let right = self.lower_match_pattern(scope, pattern.right)?;
                Ok(descriptor_is_top(
                    &TypeDescriptor::Union(vec![left, right]),
                    0,
                ))
            }
            Pattern::Type(r#type) => {
                let descriptor = lower_pattern_type(&self.types(scope), r#type)?;
                Ok(descriptor_is_top(&descriptor, 0))
            }
            Pattern::Vec(_) | Pattern::Dict(_) | Pattern::Tuple(_) => Ok(false),
        }
    }

    fn lower_match_pattern(
        &self,
        scope: &Scope<'_>,
        pattern: &Pattern<'_>,
    ) -> Result<TypeDescriptor, CompileError> {
        match pattern {
            Pattern::Variable(_) => Ok(TypeDescriptor::Wildcard),
            Pattern::Parenthesized(pattern) => self.lower_match_pattern(scope, pattern.pattern),
            Pattern::As(pattern) => Ok(TypeDescriptor::Intersection(vec![
                self.lower_match_pattern(scope, pattern.left)?,
                self.lower_match_pattern(scope, pattern.right)?,
            ])),
            Pattern::Union(pattern) => Ok(TypeDescriptor::Union(vec![
                self.lower_match_pattern(scope, pattern.left)?,
                self.lower_match_pattern(scope, pattern.right)?,
            ])),
            Pattern::Vec(pattern) => {
                let elements = pattern
                    .elements
                    .iter()
                    .map(|pattern| self.lower_match_pattern(scope, pattern))
                    .collect::<Result<Vec<_>, _>>()?;
                let rest = pattern
                    .trailing
                    .as_ref()
                    .map(|trailing| {
                        trailing.pattern.map_or_else(
                            || Ok(TypeDescriptor::Wildcard),
                            |pattern| self.lower_match_pattern(scope, pattern),
                        )
                    })
                    .transpose()?
                    .map(Box::new);
                Ok(TypeDescriptor::VectorShape { elements, rest })
            }
            Pattern::Dict(pattern) => {
                let entries = pattern
                    .entries
                    .iter()
                    .map(|entry| {
                        let key = match &entry.key {
                            DictPatternKey::String(string) => {
                                ShapeKey::String(self.heap.intern(string.value))
                            }
                            DictPatternKey::Integer { minus, literal } => ShapeKey::Int(
                                dict_pattern_integer(minus.is_some(), literal).ok_or_else(
                                    || {
                                        CompileError::new(
                                            CompileErrorKind::IntegerLiteralOutOfRange,
                                            "dictionary pattern key does not fit in an integer",
                                            entry.key.span(),
                                        )
                                    },
                                )?,
                            ),
                        };
                        Ok((key, self.lower_match_pattern(scope, entry.pattern)?))
                    })
                    .collect::<Result<Vec<_>, CompileError>>()?;
                let rest = pattern
                    .trailing
                    .as_ref()
                    .map(|trailing| {
                        let value = match trailing.pattern {
                            Some(pattern) => self.lower_match_pattern(scope, pattern)?,
                            None => TypeDescriptor::Wildcard,
                        };
                        Ok((Box::new(TypeDescriptor::Wildcard), Box::new(value)))
                    })
                    .transpose()?;
                Ok(TypeDescriptor::DictionaryShape { entries, rest })
            }
            Pattern::Tuple(pattern) => {
                let elements = pattern
                    .elements
                    .iter()
                    .map(|pattern| self.lower_match_pattern(scope, pattern))
                    .collect::<Result<Vec<_>, _>>()?;
                let rest = pattern
                    .trailing
                    .as_ref()
                    .map(|trailing| {
                        trailing.pattern.map_or_else(
                            || Ok(TypeDescriptor::Wildcard),
                            |pattern| self.lower_match_pattern(scope, pattern),
                        )
                    })
                    .transpose()?;
                let tuple = rest.clone().map_or_else(
                    || TypeDescriptor::Tuple(elements.clone()),
                    |rest| TypeDescriptor::TupleRest {
                        elements: elements.clone(),
                        rest: Box::new(rest),
                    },
                );
                let vector = TypeDescriptor::VectorShape {
                    elements,
                    rest: rest.map(Box::new),
                };
                Ok(TypeDescriptor::Union(vec![tuple, vector]))
            }
            Pattern::Type(r#type) => lower_pattern_type(&self.types(scope), r#type),
        }
    }

    fn push_pattern_bindings(&mut self, pattern: &Pattern<'_>) -> Result<usize, CompileError> {
        let saved = self.locals.len();
        self.push_nested_pattern_bindings(pattern)?;
        Ok(saved)
    }

    fn push_nested_pattern_bindings(&mut self, pattern: &Pattern<'_>) -> Result<(), CompileError> {
        match pattern {
            Pattern::Variable(variable) => {
                self.push_match_variable_binding(variable);
                Ok(())
            }
            Pattern::As(pattern) => {
                self.push_nested_pattern_bindings(pattern.left)?;
                self.push_nested_pattern_bindings(pattern.right)
            }
            Pattern::Parenthesized(pattern) => self.push_nested_pattern_bindings(pattern.pattern),
            Pattern::Union(pattern) => self.push_nested_pattern_bindings(pattern.left),
            Pattern::Vec(pattern) => {
                for element in &pattern.elements {
                    self.push_nested_pattern_bindings(element)?;
                }
                if let Some(trailing) = pattern.trailing.and_then(|trailing| trailing.pattern) {
                    self.push_nested_pattern_bindings(trailing)?;
                }
                Ok(())
            }
            Pattern::Dict(pattern) => {
                for entry in &pattern.entries {
                    self.push_nested_pattern_bindings(entry.pattern)?;
                }
                if let Some(trailing) = pattern.trailing.and_then(|trailing| trailing.pattern) {
                    self.push_nested_pattern_bindings(trailing)?;
                }
                Ok(())
            }
            Pattern::Tuple(pattern) => {
                for element in &pattern.elements {
                    self.push_nested_pattern_bindings(element)?;
                }
                if let Some(trailing) = pattern.trailing.and_then(|trailing| trailing.pattern) {
                    self.push_nested_pattern_bindings(trailing)?;
                }
                Ok(())
            }
            Pattern::Type(_) => Ok(()),
        }
    }

    fn bind_pattern(
        &mut self,
        scope: &Scope<'_>,
        pattern: &Pattern<'_>,
        value: Register,
    ) -> Result<(), CompileError> {
        match pattern {
            Pattern::Variable(variable) => self.bind_pattern_variable(variable, value),
            Pattern::As(pattern) => {
                self.bind_pattern(scope, pattern.left, value)?;
                self.bind_pattern(scope, pattern.right, value)
            }
            Pattern::Parenthesized(pattern) => self.bind_pattern(scope, pattern.pattern, value),
            Pattern::Union(pattern) => self.bind_union_pattern(scope, pattern, value),
            Pattern::Vec(pattern) => self.bind_sequence_pattern(
                scope,
                &pattern.elements,
                pattern.trailing.as_ref(),
                value,
            ),
            Pattern::Dict(pattern) => self.bind_dict_pattern(scope, pattern, value),
            Pattern::Tuple(pattern) => self.bind_sequence_pattern(
                scope,
                &pattern.elements,
                pattern.trailing.as_ref(),
                value,
            ),
            Pattern::Type(_) => Ok(()),
        }
    }

    fn bind_union_pattern(
        &mut self,
        scope: &Scope<'_>,
        pattern: &UnionPattern<'_>,
        value: Register,
    ) -> Result<(), CompileError> {
        if !pattern_has_bindings(pattern.left) {
            return Ok(());
        }

        let comparison = self.allocate(pattern.span())?;
        let descriptor = self.lower_match_pattern(scope, pattern.left)?;
        let descriptor = self.add_type_descriptor(descriptor, pattern.left.span())?;
        self.chunk.emit(
            Instruction::Is {
                destination: comparison,
                source: value,
                descriptor,
            },
            pattern.left.span(),
        );

        let use_right = self.chunk.emit(
            Instruction::JumpIfFalse {
                condition: comparison,
                offset: JumpOffset::new(0),
            },
            pattern.pipe,
        );

        self.bind_pattern(scope, pattern.left, value)?;
        let exit = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            pattern.pipe,
        );

        let right = self.code_position();
        self.chunk.patch_jump(use_right, right);
        self.bind_pattern(scope, pattern.right, value)?;
        let after = self.code_position();
        self.chunk.patch_jump(exit, after);

        Ok(())
    }

    fn bind_pattern_variable(
        &mut self,
        variable: &Variable<'_>,
        value: Register,
    ) -> Result<(), CompileError> {
        self.ensure_local_writable(variable.name, variable.span())?;
        let register = self.local_register(variable.name, variable.span())?;
        self.move_into(register, value, variable.span());
        self.mark_defined(variable.name);
        Ok(())
    }

    fn bind_sequence_pattern(
        &mut self,
        scope: &Scope<'_>,
        elements: &TokenSeparatedSequence<'_, Pattern<'_>>,
        trailing: Option<&TrailingPattern<'_>>,
        value: Register,
    ) -> Result<(), CompileError> {
        for (index, element) in elements.iter().enumerate() {
            if !pattern_has_bindings(element) {
                continue;
            }
            let extracted = self.allocate(element.span())?;
            self.chunk.emit(
                Instruction::ElementGet {
                    destination: extracted,
                    subject: value,
                    index: ImmediateInt::new(tuple_index(index)),
                },
                element.span(),
            );
            self.bind_pattern(scope, element, extracted)?;
        }
        if let Some(trailing) = trailing.and_then(|trailing| trailing.pattern)
            && pattern_has_bindings(trailing)
        {
            let remainder = self.allocate(trailing.span())?;
            self.chunk.emit(
                Instruction::Rest {
                    destination: remainder,
                    subject: value,
                    from: ImmediateInt::new(tuple_index(elements.len())),
                },
                trailing.span(),
            );
            self.bind_pattern(scope, trailing, remainder)?;
        }

        Ok(())
    }

    fn bind_dict_pattern(
        &mut self,
        scope: &Scope<'_>,
        pattern: &DictPattern<'_>,
        value: Register,
    ) -> Result<(), CompileError> {
        for entry in &pattern.entries {
            if !pattern_has_bindings(entry.pattern) {
                continue;
            }
            let index = self.pattern_dict_key(&entry.key)?;
            let extracted = self.allocate(entry.pattern.span())?;
            self.chunk.emit(
                Instruction::IndexGet {
                    destination: extracted,
                    container: value,
                    index,
                },
                entry.pattern.span(),
            );
            self.bind_pattern(scope, entry.pattern, extracted)?;
        }
        let Some(trailing) = pattern.trailing.and_then(|trailing| trailing.pattern) else {
            return Ok(());
        };
        if !pattern_has_bindings(trailing) {
            return Ok(());
        }

        let remainder = self.allocate(trailing.span())?;
        let mark = self.registers.mark();
        self.chunk.emit(
            Instruction::NewDict {
                pair_count: Count::new(0),
                destination: remainder,
                first_pair: Register::new(mark),
            },
            trailing.span(),
        );
        self.chunk.emit(
            Instruction::Spread {
                container: remainder,
                value,
            },
            trailing.span(),
        );
        for entry in &pattern.entries {
            let index = self.pattern_dict_key(&entry.key)?;
            let removed = self.allocate(entry.span())?;
            self.chunk.emit(
                Instruction::Remove {
                    destination: removed,
                    container: remainder,
                    key: index,
                },
                entry.span(),
            );
        }
        self.registers.release_to(mark);
        self.bind_pattern(scope, trailing, remainder)
    }

    pub(in crate::compiler::emit) fn bind_target(
        &mut self,
        target: &BindTarget<'_>,
        value: Register,
        span: Span,
        keys: &[Register],
        key: &mut usize,
    ) -> Result<(), CompileError> {
        match target {
            BindTarget::Variable(variable) => {
                if variable.name == "$this" {
                    return Err(CompileError::new(
                        CompileErrorKind::CannotBindThis,
                        "`$this` cannot be bound here",
                        variable.span(),
                    ));
                }

                self.ensure_local_writable(variable.name, variable.span())?;
                let register = self.local_register(variable.name, variable.span())?;
                self.move_into(register, value, span);
                self.mark_defined(variable.name);
                Ok(())
            }
            BindTarget::Tuple(tuple) => self.bind_tuple_target(tuple, value, keys, key),
            BindTarget::Dict(dict) => self.bind_dict_target(dict, value, span, keys, key),
        }
    }

    fn bind_tuple_target(
        &mut self,
        tuple: &BindTuple<'_>,
        value: Register,
        keys: &[Register],
        key: &mut usize,
    ) -> Result<(), CompileError> {
        let fixed = tuple
            .targets
            .iter()
            .take_while(|element| matches!(element, BindElement::Target(_)))
            .count();
        let arity = i16::try_from(fixed).map_err(|_| {
            CompileError::new(
                CompileErrorKind::TooManyTupleElements,
                "a binding pattern may have at most 32767 fixed targets",
                tuple.span(),
            )
        })?;
        let subject = self.allocate(tuple.span())?;
        self.move_into(subject, value, tuple.span());
        let has_rest = tuple
            .targets
            .iter()
            .any(|element| matches!(element, BindElement::Rest(_)));
        self.chunk.emit(
            Instruction::CheckDestructure {
                subject,
                required: ImmediateInt::new(arity),
                arity: ImmediateInt::new(arity),
                rest: has_rest,
            },
            tuple.span(),
        );
        let mut position = 0;
        for element in &tuple.targets {
            match element {
                BindElement::Target(target) => {
                    let element = self.allocate(target.span())?;
                    self.chunk.emit(
                        Instruction::ElementGet {
                            destination: element,
                            subject,
                            index: ImmediateInt::new(tuple_index(position)),
                        },
                        target.span(),
                    );
                    self.bind_target(target, element, target.span(), keys, key)?;
                    position += 1;
                }
                BindElement::Rest(rest) => {
                    if let Some(target) = &rest.target {
                        let remainder = self.allocate(rest.span())?;
                        self.chunk.emit(
                            Instruction::Rest {
                                destination: remainder,
                                subject,
                                from: ImmediateInt::new(arity),
                            },
                            rest.span(),
                        );
                        self.bind_target(target, remainder, rest.span(), keys, key)?;
                    }
                }
            }
        }

        Ok(())
    }

    fn bind_dict_target(
        &mut self,
        dict: &BindDict<'_>,
        value: Register,
        span: Span,
        keys: &[Register],
        key: &mut usize,
    ) -> Result<(), CompileError> {
        let descriptor = self.add_type_descriptor(TypeDescriptor::Dictionary(None), span)?;
        let subject = self.allocate(span)?;
        self.chunk.emit(
            Instruction::AsCheck {
                destination: subject,
                source: value,
                descriptor,
                mode: AsMode::Boundary,
            },
            span,
        );
        for entry in &dict.entries {
            let Some(index) = keys.get(*key).copied() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("every dictionary bind key was prepared") }
            };
            *key += 1;
            let element = self.allocate(entry.span())?;
            self.chunk.emit(
                Instruction::IndexGet {
                    destination: element,
                    container: subject,
                    index,
                },
                entry.span(),
            );
            self.bind_target(&entry.target, element, entry.span(), keys, key)?;
        }

        Ok(())
    }

    fn pattern_dict_key(&mut self, key: &DictPatternKey<'_>) -> Result<Register, CompileError> {
        match key {
            DictPatternKey::String(string) => {
                let destination = self.allocate(string.span)?;
                let constant = self.string_constant(string.value, string.span)?;
                self.chunk.emit(
                    Instruction::LoadConstant {
                        destination,
                        constant,
                    },
                    string.span,
                );
                Ok(destination)
            }
            DictPatternKey::Integer { minus, literal } => {
                let value = dict_pattern_integer(minus.is_some(), literal).ok_or_else(|| {
                    CompileError::new(
                        CompileErrorKind::IntegerLiteralOutOfRange,
                        "dictionary pattern key does not fit in an integer",
                        key.span(),
                    )
                })?;
                let destination = self.allocate(key.span())?;
                self.load_integer(destination, value, key.span())?;
                Ok(destination)
            }
        }
    }

    pub(in crate::compiler::emit) fn prepare_bind_keys(
        &mut self,
        scope: &Scope<'_>,
        target: &BindTarget<'_>,
        keys: &mut Vec<Register>,
    ) -> Result<(), CompileError> {
        match target {
            BindTarget::Variable(_) => {}
            BindTarget::Tuple(tuple) => {
                for element in &tuple.targets {
                    match element {
                        BindElement::Target(target) => {
                            self.prepare_bind_keys(scope, target, keys)?;
                        }
                        BindElement::Rest(rest) => {
                            if let Some(target) = &rest.target {
                                self.prepare_bind_keys(scope, target, keys)?;
                            }
                        }
                    }
                }
            }
            BindTarget::Dict(dict) => {
                for entry in &dict.entries {
                    keys.push(self.expression(scope, entry.key)?);
                    self.prepare_bind_keys(scope, &entry.target, keys)?;
                }
            }
        }
        Ok(())
    }
}

fn pattern_alternatives<'arena>(
    pattern: &'arena Pattern<'arena>,
    alternatives: &mut Vec<&'arena Pattern<'arena>>,
) {
    match pattern {
        Pattern::Union(pattern) => {
            pattern_alternatives(pattern.left, alternatives);
            pattern_alternatives(pattern.right, alternatives);
        }
        Pattern::Parenthesized(pattern) => pattern_alternatives(pattern.pattern, alternatives),
        _ => alternatives.push(pattern),
    }
}

fn pattern_needs_split_bindings(pattern: &Pattern<'_>) -> bool {
    match pattern {
        Pattern::Union(_) => pattern_has_bindings(pattern),
        Pattern::Parenthesized(pattern) => pattern_needs_split_bindings(pattern.pattern),
        _ => false,
    }
}

fn check_pattern(pattern: &Pattern<'_>) -> Result<(), CompileError> {
    let mut bindings = HashSet::new();
    check_pattern_bindings(pattern, &mut bindings)
}

fn check_pattern_bindings<'arena>(
    pattern: &Pattern<'arena>,
    bindings: &mut HashSet<&'arena str>,
) -> Result<(), CompileError> {
    match pattern {
        Pattern::Variable(variable) => collect_pattern_binding(variable, bindings),
        Pattern::Type(_) => Ok(()),
        Pattern::Parenthesized(pattern) => check_pattern_bindings(pattern.pattern, bindings),
        Pattern::As(pattern) => {
            check_pattern_bindings(pattern.left, bindings)?;
            check_pattern_bindings(pattern.right, bindings)
        }
        Pattern::Union(pattern) => {
            let mut left_bindings = bindings.clone();
            check_pattern_bindings(pattern.left, &mut left_bindings)?;
            let mut right_bindings = bindings.clone();
            check_pattern_bindings(pattern.right, &mut right_bindings)?;
            if !same_binding_layout(pattern.left, pattern.right) {
                return Err(CompileError::new(
                    CompileErrorKind::InconsistentPatternBindings,
                    "every alternative of a union pattern must introduce the same bindings",
                    pattern.span(),
                ));
            }
            *bindings = left_bindings;
            Ok(())
        }
        Pattern::Vec(pattern) => {
            check_sequence(
                CompileErrorKind::TooManyTupleElements,
                "a vector pattern may have",
                "elements",
                &pattern.elements,
            )?;
            for element in &pattern.elements {
                check_pattern_bindings(element, bindings)?;
            }
            if let Some(trailing) = pattern.trailing.and_then(|trailing| trailing.pattern) {
                check_pattern_bindings(trailing, bindings)?;
            }
            Ok(())
        }
        Pattern::Dict(pattern) => {
            for (index, entry) in pattern.entries.iter().enumerate() {
                if pattern
                    .entries
                    .iter()
                    .take(index)
                    .any(|previous| same_dict_pattern_key(&previous.key, &entry.key))
                {
                    return Err(CompileError::new(
                        CompileErrorKind::DuplicateDictionaryKey,
                        "a dictionary pattern cannot repeat a key",
                        entry.key.span(),
                    ));
                }
            }
            for entry in &pattern.entries {
                check_pattern_bindings(entry.pattern, bindings)?;
            }
            if let Some(trailing) = pattern.trailing.and_then(|trailing| trailing.pattern) {
                check_pattern_bindings(trailing, bindings)?;
            }
            Ok(())
        }
        Pattern::Tuple(pattern) => {
            check_tuple_sequence(
                CompileErrorKind::TooManyTupleElements,
                "a tuple pattern may have",
                "elements",
                &pattern.elements,
            )?;
            for element in &pattern.elements {
                check_pattern_bindings(element, bindings)?;
            }
            if let Some(trailing) = pattern.trailing.and_then(|trailing| trailing.pattern) {
                check_pattern_bindings(trailing, bindings)?;
            }
            Ok(())
        }
    }
}

fn collect_pattern_binding<'arena>(
    variable: &Variable<'arena>,
    bindings: &mut HashSet<&'arena str>,
) -> Result<(), CompileError> {
    if variable.name == "$this" {
        return Err(CompileError::new(
            CompileErrorKind::CannotBindThis,
            "`$this` cannot be bound here",
            variable.span(),
        ));
    }

    if !bindings.insert(variable.name) {
        return Err(CompileError::new(
            CompileErrorKind::DuplicatePatternBinding,
            format!(
                "the match pattern introduces the binding `{}` more than once",
                variable.name
            ),
            variable.span(),
        ));
    }

    Ok(())
}

fn pattern_has_bindings(pattern: &Pattern<'_>) -> bool {
    match pattern {
        Pattern::Variable(_) => true,
        Pattern::Parenthesized(pattern) => pattern_has_bindings(pattern.pattern),
        Pattern::As(pattern) => {
            pattern_has_bindings(pattern.left) || pattern_has_bindings(pattern.right)
        }
        Pattern::Union(pattern) => {
            pattern_has_bindings(pattern.left) || pattern_has_bindings(pattern.right)
        }
        Pattern::Vec(pattern) => {
            pattern.elements.iter().any(pattern_has_bindings)
                || pattern
                    .trailing
                    .and_then(|trailing| trailing.pattern)
                    .is_some_and(pattern_has_bindings)
        }
        Pattern::Dict(pattern) => {
            pattern
                .entries
                .iter()
                .any(|entry| pattern_has_bindings(entry.pattern))
                || pattern
                    .trailing
                    .and_then(|trailing| trailing.pattern)
                    .is_some_and(pattern_has_bindings)
        }
        Pattern::Tuple(pattern) => {
            pattern.elements.iter().any(pattern_has_bindings)
                || pattern
                    .trailing
                    .and_then(|trailing| trailing.pattern)
                    .is_some_and(pattern_has_bindings)
        }
        Pattern::Type(_) => false,
    }
}

fn same_binding_layout(left: &Pattern<'_>, right: &Pattern<'_>) -> bool {
    match (left, right) {
        (Pattern::Parenthesized(left), _) => same_binding_layout(left.pattern, right),
        (_, Pattern::Parenthesized(right)) => same_binding_layout(left, right.pattern),
        (Pattern::Variable(left), Pattern::Variable(right)) => left.name == right.name,
        (Pattern::As(left), Pattern::As(right)) => {
            same_binding_layout(left.left, right.left)
                && same_binding_layout(left.right, right.right)
        }
        (Pattern::Union(left), Pattern::Union(right)) => {
            same_binding_layout(left.left, right.left)
                && same_binding_layout(left.right, right.right)
        }
        (Pattern::Vec(left), Pattern::Vec(right)) => {
            same_sequence_binding_layout(&left.elements, &right.elements)
                && same_trailing_binding_layout(left.trailing.as_ref(), right.trailing.as_ref())
        }
        (Pattern::Dict(left), Pattern::Dict(right)) => {
            left.entries.len() == right.entries.len()
                && left
                    .entries
                    .iter()
                    .zip(right.entries.iter())
                    .all(|(left, right)| same_binding_layout(left.pattern, right.pattern))
                && same_trailing_binding_layout(left.trailing.as_ref(), right.trailing.as_ref())
        }
        (Pattern::Tuple(left), Pattern::Tuple(right)) => {
            same_sequence_binding_layout(&left.elements, &right.elements)
                && same_trailing_binding_layout(left.trailing.as_ref(), right.trailing.as_ref())
        }
        _ => !pattern_has_bindings(left) && !pattern_has_bindings(right),
    }
}

fn same_trailing_binding_layout(
    left: Option<&TrailingPattern<'_>>,
    right: Option<&TrailingPattern<'_>>,
) -> bool {
    match (
        left.and_then(|trailing| trailing.pattern),
        right.and_then(|trailing| trailing.pattern),
    ) {
        (Some(left), Some(right)) => same_binding_layout(left, right),
        (None, None) => true,
        _ => {
            !left.is_some_and(|trailing| {
                trailing
                    .pattern
                    .is_some_and(|pattern| pattern_has_bindings(pattern))
            }) && !right.is_some_and(|trailing| {
                trailing
                    .pattern
                    .is_some_and(|pattern| pattern_has_bindings(pattern))
            })
        }
    }
}

fn same_sequence_binding_layout(
    left: &TokenSeparatedSequence<'_, Pattern<'_>>,
    right: &TokenSeparatedSequence<'_, Pattern<'_>>,
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| same_binding_layout(left, right))
}

fn same_dict_pattern_key(left: &DictPatternKey<'_>, right: &DictPatternKey<'_>) -> bool {
    match (left, right) {
        (
            DictPatternKey::Integer {
                minus: left_minus,
                literal: left,
            },
            DictPatternKey::Integer {
                minus: right_minus,
                literal: right,
            },
        ) => {
            match (
                dict_pattern_integer(left_minus.is_some(), left),
                dict_pattern_integer(right_minus.is_some(), right),
            ) {
                (Some(left), Some(right)) => left == right,
                _ => false,
            }
        }
        (DictPatternKey::String(left), DictPatternKey::String(right)) => left.value == right.value,
        _ => false,
    }
}
