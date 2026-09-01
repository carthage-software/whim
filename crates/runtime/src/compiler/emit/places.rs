//! Lvalue lowering: resolving an assignment target and writing back.

use whim_syn::cst::access::StaticPropertyAccess;
use whim_syn::cst::operation::Assignment;
use whim_syn::cst::operation::TupleDestructure;

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::compiler::emit::Access;
use crate::compiler::emit::AsMode;
use crate::compiler::emit::AssignmentOperator;
use crate::compiler::emit::AssignmentTarget;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::CompileErrorKind;
use crate::compiler::emit::DestructureTarget;
use crate::compiler::emit::Expression;
use crate::compiler::emit::HasSpan;
use crate::compiler::emit::IcDescriptor;
use crate::compiler::emit::ImmediateInt;
use crate::compiler::emit::Instruction;
use crate::compiler::emit::JumpOffset;
use crate::compiler::emit::Register;
use crate::compiler::emit::Scope;
use crate::compiler::emit::ShortCircuit;
use crate::compiler::emit::Span;
use crate::compiler::emit::check_tuple_sequence;
use crate::compiler::emit::compound_instruction;
use crate::compiler::emit::short_circuit_jump;
use crate::compiler::emit::tuple_index;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;

fn check_destructure_targets(destructure: &TupleDestructure<'_>) -> Result<(), CompileError> {
    check_tuple_sequence(
        CompileErrorKind::TooManyTupleElements,
        "a destructuring pattern may have",
        "targets",
        &destructure.targets,
    )?;

    let mut seen_rest = false;
    let mut seen_default = false;
    for element in &destructure.targets {
        if seen_rest {
            return Err(CompileError::new(
                CompileErrorKind::TargetAfterRest,
                "no target can follow a `...` rest; the rest takes every element the fixed \
                 targets did not, so there is nothing left for a later one to name",
                element.span(),
            ));
        }

        match element {
            DestructureTarget::Target(_) if seen_default => {
                return Err(CompileError::new(
                    CompileErrorKind::RequiredTargetAfterDefault,
                    "a required destructuring target cannot follow one with a default",
                    element.span(),
                ));
            }
            DestructureTarget::Target(_) => {}
            DestructureTarget::Default(_) => seen_default = true,
            DestructureTarget::Rest(_) => seen_rest = true,
        }
    }

    Ok(())
}

fn destructure_count(count: usize, subject: &str, span: Span) -> Result<i16, CompileError> {
    i16::try_from(count).map_err(|_| {
        CompileError::new(
            CompileErrorKind::TooManyArguments,
            format!("a destructuring pattern is limited to 32767 {subject}"),
            span,
        )
    })
}

pub(in crate::compiler::emit) enum WriteTarget {
    Output,
    OutputLine,
    Error,
    ErrorLine,
    Diagnostic,
}

/// The prepared place of an assignment target: its subexpressions already
/// evaluated, in source order.
pub(in crate::compiler::emit) enum Place<'arena> {
    Local {
        name: String,
    },
    Property {
        object: Register,
        cache: IcSlot,
    },
    StaticProperty {
        cache: IcSlot,
    },
    /// A value already in a register, with nowhere to store it back: the
    /// result of a call, for instance. Writing through it evaluates and is
    /// then discarded, which is what a value-semantics container means by
    /// writing into a temporary.
    Temporary {
        register: Register,
    },
    Chain {
        root: Box<Self>,
        levels: Option<Vec<Register>>,
        steps: Vec<ChainStep>,
    },
    Tuple {
        places: Vec<DestructurePlace<'arena>>,
        rest: Option<RestPlace<'arena>>,
    },
    Dict {
        entries: Vec<DictDestructurePlace<'arena>>,
    },
}

pub(in crate::compiler::emit) struct DictDestructurePlace<'arena> {
    key: Register,
    place: Place<'arena>,
}

/// One fixed position in a prepared destructuring pattern.
pub(in crate::compiler::emit) enum DestructurePlace<'arena> {
    Required(Place<'arena>),
    Defaulted {
        place: Place<'arena>,
        value: &'arena Expression<'arena>,
    },
}

/// What a destructuring pattern's trailing `...` does with the elements the
/// fixed targets did not take.
pub(in crate::compiler::emit) enum RestPlace<'arena> {
    Bound(Box<Place<'arena>>),
    Discarded,
}

#[derive(Clone, Copy)]
pub(in crate::compiler::emit) enum ChainStep {
    Index(Register),
    /// `container[]`, which only ever appears last.
    Append,
}

#[derive(Clone, Copy)]
struct TupleWrite {
    subject: Register,
    length: Option<Register>,
    position: usize,
    span: Span,
    refresh: bool,
}

impl BodyCompiler<'_, '_> {
    pub(in crate::compiler) fn assignment(
        &mut self,
        scope: &Scope<'_>,
        assignment: &Assignment<'_>,
    ) -> Result<Register, CompileError> {
        match assignment.operator {
            AssignmentOperator::Assign(_) => {
                let place = self.prepare_place(scope, &assignment.target)?;
                let value = self.expression(scope, assignment.value)?;
                self.write_place(scope, &place, value, assignment.span())?;
                Ok(value)
            }
            AssignmentOperator::Coalesce(_)
            | AssignmentOperator::LogicalAnd(_)
            | AssignmentOperator::LogicalOr(_) => {
                let kind = match assignment.operator {
                    AssignmentOperator::Coalesce(_) => ShortCircuit::Coalesce,
                    AssignmentOperator::LogicalAnd(_) => ShortCircuit::And,
                    _ => ShortCircuit::Or,
                };
                let mut place = self.prepare_place(scope, &assignment.target)?;
                self.materialize_place(&mut place, assignment.span())?;
                let current = self.read_place(&place, assignment.span())?;
                let result = self.allocate(assignment.span())?;
                self.move_into(result, current, assignment.span());
                let skip = self
                    .chunk
                    .emit(short_circuit_jump(kind, result), assignment.operator.span());
                let saved = self.save_defined();
                let value = self.expression(scope, assignment.value)?;
                self.move_into(result, value, assignment.value.span());
                self.write_place(scope, &place, result, assignment.span())?;
                self.restore_defined(saved);
                let after = self.code_position();
                self.chunk.patch_jump(skip, after);
                Ok(result)
            }
            _ => {
                let mut place = self.prepare_place(scope, &assignment.target)?;
                let value = self.expression(scope, assignment.value)?;
                self.materialize_place(&mut place, assignment.span())?;
                let current = self.read_place(&place, assignment.span())?;
                if let Place::Local { name } = &place {
                    let name = name.clone();
                    let destination = self.local_register(&name, assignment.span())?;
                    let instruction =
                        compound_instruction(assignment.operator, destination, current, value);
                    self.chunk.emit(instruction, assignment.span());
                    self.mark_defined(&name);
                    return Ok(destination);
                }

                let destination = self.allocate(assignment.span())?;
                let instruction =
                    compound_instruction(assignment.operator, destination, current, value);
                self.chunk.emit(instruction, assignment.span());
                self.write_place(scope, &place, destination, assignment.span())?;
                Ok(destination)
            }
        }
    }

    pub(in crate::compiler) fn assignment_discarded(
        &mut self,
        scope: &Scope<'_>,
        assignment: &Assignment<'_>,
    ) -> Result<(), CompileError> {
        if matches!(assignment.operator, AssignmentOperator::Addition(_)) {
            match &assignment.target {
                AssignmentTarget::Property(access) => {
                    let object = self.expression(scope, access.object)?;
                    let cache = self.add_ic_descriptor(
                        IcDescriptor::Member {
                            name: self.heap.intern(access.property.value.as_bytes()),
                            type_arguments: None,
                        },
                        access.span(),
                    )?;
                    let source = self.expression(scope, assignment.value)?;
                    self.chunk.emit(
                        Instruction::PropertyAdd {
                            object,
                            source,
                            cache,
                        },
                        assignment.span(),
                    );
                    return Ok(());
                }
                AssignmentTarget::ArrayIndex(access) => {
                    let Expression::Access(Access::Property(property)) =
                        access.array.unparenthesized()
                    else {
                        self.assignment(scope, assignment)?;
                        return Ok(());
                    };
                    let object = self.expression(scope, property.object)?;
                    let cache = self.add_ic_descriptor(
                        IcDescriptor::Member {
                            name: self.heap.intern(property.property.value.as_bytes()),
                            type_arguments: None,
                        },
                        property.span(),
                    )?;
                    let index = self.expression(scope, access.index)?;
                    let value = self.expression(scope, assignment.value)?;
                    let container = self.allocate(assignment.span())?;
                    self.chunk.emit(
                        Instruction::PropertyGet {
                            destination: container,
                            object,
                            cache,
                        },
                        assignment.span(),
                    );
                    self.chunk.emit(
                        Instruction::IndexAddAssign {
                            container,
                            index,
                            value,
                            mode: IndexAddMode::Generic,
                        },
                        assignment.span(),
                    );
                    self.chunk.emit(
                        Instruction::PropertySet {
                            object,
                            value: container,
                            cache,
                        },
                        assignment.span(),
                    );
                    return Ok(());
                }
                _ => {}
            }
        }

        self.assignment(scope, assignment)?;
        Ok(())
    }

    /// Evaluates a target's subexpressions, in source order, into a place.
    pub(in crate::compiler::emit) fn prepare_place<'arena>(
        &mut self,
        scope: &Scope<'_>,
        target: &AssignmentTarget<'arena>,
    ) -> Result<Place<'arena>, CompileError> {
        match target {
            AssignmentTarget::Variable(variable) => {
                if variable.name == "$this" {
                    return Err(CompileError::new(
                        CompileErrorKind::ThisOutsideMethod,
                        "`$this` cannot be assigned",
                        variable.span(),
                    ));
                }

                self.ensure_local_writable(variable.name, variable.span())?;

                Ok(Place::Local {
                    name: variable.name.to_string(),
                })
            }
            AssignmentTarget::Property(access) => {
                let object = self.expression(scope, access.object)?;
                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: self.heap.intern(access.property.value.as_bytes()),
                        type_arguments: None,
                    },
                    access.span(),
                )?;

                Ok(Place::Property { object, cache })
            }
            AssignmentTarget::StaticProperty(access) => {
                let cache = self.static_property_cache(scope, access)?;
                Ok(Place::StaticProperty { cache })
            }
            AssignmentTarget::ArrayIndex(access) => {
                let (root, mut indexes) = self.prepare_chain(scope, access.array)?;
                indexes.push(self.expression(scope, access.index)?);
                let steps: Vec<ChainStep> = indexes.into_iter().map(ChainStep::Index).collect();
                Ok(Place::Chain {
                    root: Box::new(root),
                    levels: None,
                    steps,
                })
            }
            AssignmentTarget::ArrayAppend(append) => {
                let (root, indexes) = self.prepare_chain(scope, append.array)?;
                let mut steps: Vec<ChainStep> = indexes.into_iter().map(ChainStep::Index).collect();
                steps.push(ChainStep::Append);
                Ok(Place::Chain {
                    root: Box::new(root),
                    levels: None,
                    steps,
                })
            }
            AssignmentTarget::Tuple(destructure) => {
                check_destructure_targets(destructure)?;
                let mut places = Vec::new();
                let mut rest = None;
                for element in &destructure.targets {
                    match element {
                        DestructureTarget::Target(inner) => {
                            places.push(DestructurePlace::Required(
                                self.prepare_place(scope, inner)?,
                            ));
                        }
                        DestructureTarget::Default(default) => {
                            places.push(DestructurePlace::Defaulted {
                                place: self.prepare_place(scope, &default.target)?,
                                value: default.value,
                            });
                        }
                        DestructureTarget::Rest(inner) => {
                            rest = Some(match &inner.target {
                                Some(target) => {
                                    RestPlace::Bound(Box::new(self.prepare_place(scope, target)?))
                                }
                                None => RestPlace::Discarded,
                            });
                        }
                    }
                }

                Ok(Place::Tuple { places, rest })
            }
            AssignmentTarget::Dict(destructure) => {
                let mut entries = Vec::with_capacity(destructure.entries.len());
                for entry in &destructure.entries {
                    let key = self.expression(scope, entry.key)?;
                    let place = self.prepare_place(scope, &entry.target)?;
                    entries.push(DictDestructurePlace { key, place });
                }

                Ok(Place::Dict { entries })
            }
        }
    }

    /// Decomposes the container side of a write into the root that can hold
    /// the result and the index steps that lead from it, evaluating every
    /// subexpression in source order.
    pub(in crate::compiler::emit) fn prepare_chain<'arena>(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'arena>,
    ) -> Result<(Place<'arena>, Vec<Register>), CompileError> {
        let mut links = Vec::new();
        let mut current = expression;
        let base = loop {
            match current {
                Expression::Parenthesized(parenthesized) => current = parenthesized.expression,
                Expression::ArrayAccess(access) => {
                    links.push(access);
                    current = access.array;
                }
                other => break other,
            }
        };

        let root = self.prepare_chain_base(scope, base)?;
        let mut indexes = Vec::with_capacity(links.len());
        for access in links.into_iter().rev() {
            indexes.push(self.expression(scope, access.index)?);
        }

        Ok((root, indexes))
    }

    fn prepare_chain_base<'arena>(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'arena>,
    ) -> Result<Place<'arena>, CompileError> {
        match expression {
            Expression::Variable(variable) if variable.name != "$this" => {
                self.ensure_local_writable(variable.name, variable.span())?;
                Ok(Place::Local {
                    name: variable.name.to_string(),
                })
            }
            Expression::Access(Access::Property(access)) => {
                let object = self.expression(scope, access.object)?;
                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: self.heap.intern(access.property.value.as_bytes()),
                        type_arguments: None,
                    },
                    access.span(),
                )?;

                Ok(Place::Property { object, cache })
            }
            Expression::Access(Access::StaticProperty(access)) => {
                let cache = self.static_property_cache(scope, access)?;
                Ok(Place::StaticProperty { cache })
            }
            other => {
                let register = self.expression(scope, other)?;
                Ok(Place::Temporary { register })
            }
        }
    }

    pub(in crate::compiler::emit) fn materialize_place(
        &mut self,
        place: &mut Place<'_>,
        span: Span,
    ) -> Result<(), CompileError> {
        let Place::Chain {
            root,
            levels,
            steps,
        } = place
        else {
            return Ok(());
        };
        if levels.is_some() {
            return Ok(());
        }

        let root_register = self.read_place(root, span)?;
        *levels = Some(self.materialize_levels(root_register, steps, span)?);
        Ok(())
    }

    /// Loads the container each step reaches into, from the root down. The
    /// leaf step's own container is the last level, so a chain of `n` steps
    /// has `n` levels.
    pub(in crate::compiler::emit) fn materialize_levels(
        &mut self,
        root_register: Register,
        steps: &[ChainStep],
        span: Span,
    ) -> Result<Vec<Register>, CompileError> {
        let mut levels = vec![root_register];
        for step in &steps[..steps.len().saturating_sub(1)] {
            let ChainStep::Index(index) = step else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("only the leaf step appends") }
            };

            let container = levels[levels.len() - 1];
            let destination = self.allocate(span)?;
            self.chunk.emit(
                Instruction::IndexGet {
                    destination,
                    container,
                    index: *index,
                },
                span,
            );
            levels.push(destination);
        }

        Ok(levels)
    }

    pub(in crate::compiler::emit) fn read_place(
        &mut self,
        place: &Place<'_>,
        span: Span,
    ) -> Result<Register, CompileError> {
        match place {
            Place::Local { name } => {
                let name = name.clone();
                self.read_local(&name, span)
            }
            Place::Property { object, cache } => {
                let destination = self.allocate(span)?;
                self.chunk.emit(
                    Instruction::PropertyGet {
                        destination,
                        object: *object,
                        cache: *cache,
                    },
                    span,
                );
                Ok(destination)
            }
            Place::StaticProperty { cache } => {
                let destination = self.allocate(span)?;
                self.chunk.emit(
                    Instruction::StaticPropertyGet {
                        destination,
                        cache: *cache,
                    },
                    span,
                );
                Ok(destination)
            }
            Place::Temporary { register } => Ok(*register),
            Place::Chain { levels, steps, .. } => {
                let Some(levels) = levels else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe { unreachable_invariant("a chain is materialized before it is read") }
                };
                let container = levels[levels.len() - 1];
                let Some(ChainStep::Index(index)) = steps.last() else {
                    return Err(CompileError::new(
                        CompileErrorKind::InvalidCompoundAssignmentTarget,
                        "a compound assignment cannot target an append or destructuring pattern",
                        span,
                    ));
                };
                let destination = self.allocate(span)?;
                self.chunk.emit(
                    Instruction::IndexGet {
                        destination,
                        container,
                        index: *index,
                    },
                    span,
                );
                Ok(destination)
            }
            Place::Tuple { .. } | Place::Dict { .. } => Err(CompileError::new(
                CompileErrorKind::InvalidCompoundAssignmentTarget,
                "a compound assignment cannot target an append or destructuring pattern",
                span,
            )),
        }
    }

    pub(in crate::compiler::emit) fn write_place(
        &mut self,
        scope: &Scope<'_>,
        place: &Place<'_>,
        value: Register,
        span: Span,
    ) -> Result<(), CompileError> {
        self.write_place_after_destructure_write(scope, place, value, span, false)
    }

    fn write_place_after_destructure_write(
        &mut self,
        scope: &Scope<'_>,
        place: &Place<'_>,
        value: Register,
        span: Span,
        refresh_chain: bool,
    ) -> Result<(), CompileError> {
        match place {
            Place::Local { name } => {
                let name = name.clone();
                self.ensure_local_writable(&name, span)?;
                let register = self.local_register(&name, span)?;
                self.move_into(register, value, span);
                self.mark_defined(&name);
                Ok(())
            }
            Place::Property { object, cache } => {
                self.chunk.emit(
                    Instruction::PropertySet {
                        object: *object,
                        value,
                        cache: *cache,
                    },
                    span,
                );
                Ok(())
            }
            Place::StaticProperty { cache } => {
                self.chunk.emit(
                    Instruction::StaticPropertySet {
                        cache: *cache,
                        value,
                    },
                    span,
                );
                Ok(())
            }
            Place::Temporary { .. } => Ok(()),
            Place::Chain { .. } => self.write_chain(scope, place, value, span, refresh_chain),
            Place::Tuple { places, rest } => {
                self.write_tuple(scope, places, rest.as_ref(), value, span, refresh_chain)
            }
            Place::Dict { entries } => self.write_dict(scope, entries, value, span, refresh_chain),
        }
    }

    fn write_chain(
        &mut self,
        scope: &Scope<'_>,
        place: &Place<'_>,
        value: Register,
        span: Span,
        refresh: bool,
    ) -> Result<(), CompileError> {
        let Place::Chain {
            root,
            levels,
            steps,
        } = place
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("write_chain receives a chain place") }
        };

        if levels.is_none() && self.write_direct_property_index_set(root, steps, value, span)? {
            return Ok(());
        }

        if levels.is_none() && self.write_direct_property_append(root, steps, value, span) {
            return Ok(());
        }

        let refreshed_levels = if refresh || levels.is_none() {
            let root_register = self.read_place(root, span)?;
            Some(self.materialize_levels(root_register, steps, span)?)
        } else {
            None
        };
        // SAFETY: the surrounding invariant proves this option contains a value.
        let levels = unsafe {
            unwrap_option_invariant(
                refreshed_levels.as_deref().or(levels.as_deref()),
                "a chain is materialized before it is written",
            )
        };
        let leaf = levels[levels.len() - 1];
        match steps.last() {
            Some(ChainStep::Index(index)) => self.chunk.emit(
                Instruction::IndexSet {
                    container: leaf,
                    index: *index,
                    value,
                },
                span,
            ),
            Some(ChainStep::Append) => self.chunk.emit(
                Instruction::Append {
                    container: leaf,
                    value,
                },
                span,
            ),
            // SAFETY: the surrounding invariant makes this path unreachable.
            None => unsafe { unreachable_invariant("a chain has at least one step") },
        };
        for level in (1..levels.len()).rev() {
            let ChainStep::Index(index) = steps[level - 1] else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("only the leaf step appends") }
            };
            self.chunk.emit(
                Instruction::IndexSet {
                    container: levels[level - 1],
                    index,
                    value: levels[level],
                },
                span,
            );
        }
        self.write_place(scope, root, levels[0], span)
    }

    fn write_direct_property_index_set(
        &mut self,
        root: &Place<'_>,
        steps: &[ChainStep],
        value: Register,
        span: Span,
    ) -> Result<bool, CompileError> {
        let Place::Property { object, cache } = root else {
            return Ok(false);
        };

        let [ChainStep::Index(index)] = steps else {
            return Ok(false);
        };

        if value.index() == index.index().saturating_add(1) {
            self.chunk.emit(
                Instruction::PropertyIndexSet {
                    object: *object,
                    first_operand: *index,
                    cache: *cache,
                },
                span,
            );

            return Ok(true);
        }

        let first_operand = self.allocate(span)?;
        let second_operand = self.allocate(span)?;
        self.move_into(first_operand, *index, span);
        self.move_into(second_operand, value, span);
        self.chunk.emit(
            Instruction::PropertyIndexSet {
                object: *object,
                first_operand,
                cache: *cache,
            },
            span,
        );

        Ok(true)
    }

    fn write_direct_property_append(
        &mut self,
        root: &Place<'_>,
        steps: &[ChainStep],
        value: Register,
        span: Span,
    ) -> bool {
        let Place::Property { object, cache } = root else {
            return false;
        };

        if !matches!(steps, [ChainStep::Append]) {
            return false;
        }

        self.chunk.emit(
            Instruction::PropertyIndexUpdate {
                object: *object,
                operand: value,
                cache: *cache,
                mode: PropertyIndexUpdateMode::Append,
            },
            span,
        );

        true
    }

    fn write_tuple(
        &mut self,
        scope: &Scope<'_>,
        places: &[DestructurePlace<'_>],
        rest: Option<&RestPlace<'_>>,
        value: Register,
        span: Span,
        refresh: bool,
    ) -> Result<(), CompileError> {
        let arity = destructure_count(places.len(), "targets", span)?;
        let required_count = places
            .iter()
            .take_while(|place| matches!(place, DestructurePlace::Required(_)))
            .count();
        let required = destructure_count(required_count, "required targets", span)?;
        let subject = self.allocate(span)?;
        self.move_into(subject, value, span);
        self.chunk.emit(
            Instruction::CheckDestructure {
                subject,
                required: ImmediateInt::new(required),
                arity: ImmediateInt::new(arity),
                rest: rest.is_some(),
            },
            span,
        );
        let length = if required == arity {
            None
        } else {
            let length = self.allocate(span)?;
            self.chunk.emit(
                Instruction::Length {
                    destination: length,
                    source: subject,
                },
                span,
            );
            Some(length)
        };
        let mut refresh_later = refresh;
        for (position, place) in places.iter().enumerate() {
            let write = TupleWrite {
                subject,
                length,
                position,
                span,
                refresh: refresh_later,
            };
            self.write_tuple_place(scope, place, write)?;
            refresh_later = true;
        }
        if let Some(RestPlace::Bound(place)) = rest {
            let remainder = self.allocate(span)?;
            self.chunk.emit(
                Instruction::Rest {
                    destination: remainder,
                    subject,
                    from: ImmediateInt::new(arity),
                },
                span,
            );
            self.write_place_after_destructure_write(scope, place, remainder, span, refresh_later)?;
        }
        Ok(())
    }

    fn write_tuple_place(
        &mut self,
        scope: &Scope<'_>,
        place: &DestructurePlace<'_>,
        write: TupleWrite,
    ) -> Result<(), CompileError> {
        match place {
            DestructurePlace::Required(place) => {
                let element = self.tuple_element(write.subject, write.position, write.span)?;
                self.write_place_after_destructure_write(
                    scope,
                    place,
                    element,
                    write.span,
                    write.refresh,
                )
            }
            DestructurePlace::Defaulted { place, value } => {
                let Some(length) = write.length else {
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    unsafe {
                        unreachable_invariant(
                            "a defaulted target makes the required prefix shorter",
                        )
                    }
                };
                self.write_defaulted_tuple_place(scope, place, value, length, write)
            }
        }
    }

    fn write_defaulted_tuple_place(
        &mut self,
        scope: &Scope<'_>,
        place: &Place<'_>,
        default: &Expression<'_>,
        length: Register,
        write: TupleWrite,
    ) -> Result<(), CompileError> {
        let index = self.allocate(write.span)?;
        self.chunk.emit(
            Instruction::LoadInt {
                destination: index,
                immediate: ImmediateInt::new(tuple_index(write.position)),
            },
            write.span,
        );
        let present = self.allocate(write.span)?;
        self.chunk.emit(
            Instruction::LessThan {
                destination: present,
                left: index,
                right: length,
            },
            write.span,
        );
        let use_present = self.chunk.emit(
            Instruction::JumpIfTrue {
                condition: present,
                offset: JumpOffset::new(0),
            },
            write.span,
        );
        let saved = self.save_defined();
        let default_value = self.expression(scope, default)?;
        self.restore_defined(saved);
        self.write_place_after_destructure_write(
            scope,
            place,
            default_value,
            default.span(),
            write.refresh,
        )?;
        let exit = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            default.span(),
        );
        let present_position = self.code_position();
        self.chunk.patch_jump(use_present, present_position);
        let element = self.tuple_element(write.subject, write.position, write.span)?;
        self.write_place_after_destructure_write(scope, place, element, write.span, write.refresh)?;
        let after = self.code_position();
        self.chunk.patch_jump(exit, after);
        Ok(())
    }

    fn tuple_element(
        &mut self,
        subject: Register,
        position: usize,
        span: Span,
    ) -> Result<Register, CompileError> {
        let element = self.allocate(span)?;
        self.chunk.emit(
            Instruction::ElementGet {
                destination: element,
                subject,
                index: ImmediateInt::new(tuple_index(position)),
            },
            span,
        );
        Ok(element)
    }

    fn write_dict(
        &mut self,
        scope: &Scope<'_>,
        entries: &[DictDestructurePlace<'_>],
        value: Register,
        span: Span,
        refresh: bool,
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
        let mut refresh_later = refresh;
        for entry in entries {
            let element = self.allocate(span)?;
            self.chunk.emit(
                Instruction::IndexGet {
                    destination: element,
                    container: subject,
                    index: entry.key,
                },
                span,
            );
            self.write_place_after_destructure_write(
                scope,
                &entry.place,
                element,
                span,
                refresh_later,
            )?;
            refresh_later = true;
        }
        Ok(())
    }

    pub(in crate::compiler) fn static_property_cache(
        &mut self,
        scope: &Scope<'_>,
        access: &StaticPropertyAccess<'_>,
    ) -> Result<IcSlot, CompileError> {
        let class = self.class_reference_atom(scope, &access.class)?;
        let member = self.heap.intern(
            access
                .property
                .name
                .strip_prefix('$')
                .unwrap_or(access.property.name)
                .as_bytes(),
        );

        self.add_ic_descriptor(
            IcDescriptor::ClassMember {
                class,
                member,
                type_arguments: None,
            },
            access.span(),
        )
    }
}
