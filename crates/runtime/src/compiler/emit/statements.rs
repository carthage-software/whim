//! Statements, and the control flow between them.

use std::mem;
use std::ops::Range;
use std::slice;

use whim_syn::cst::atom::Literal;
use whim_syn::cst::atom::Variable;
use whim_syn::cst::binding::BindingTarget as BindTarget;
use whim_syn::cst::binding::ElementBindingTarget as BindElement;
use whim_syn::cst::control_flow::DoWhile;
use whim_syn::cst::control_flow::ElseBody;
use whim_syn::cst::control_flow::For;
use whim_syn::cst::control_flow::Foreach;
use whim_syn::cst::control_flow::If;
use whim_syn::cst::control_flow::Try;
use whim_syn::cst::control_flow::TryCatchClause;
use whim_syn::cst::control_flow::While;
use whim_syn::cst::expression::Return;
use whim_syn::cst::operation::BinaryOperator;
use whim_syn::cst::statement::Block;
use whim_syn::cst::statement::FinalLocal;
use whim_syn::cst::statement::Using;

use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::BytecodeLiteral;
use crate::compiler::emit::CatchEntry;
use crate::compiler::emit::Cleanup;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::CompileErrorKind;
use crate::compiler::emit::ControlFrame;
use crate::compiler::emit::Expression;
use crate::compiler::emit::HasSpan;
use crate::compiler::emit::Instruction;
use crate::compiler::emit::JumpOffset;
use crate::compiler::emit::LoopFrame;
use crate::compiler::emit::LoopJump;
use crate::compiler::emit::Register;
use crate::compiler::emit::Scope;
use crate::compiler::emit::Span;
use crate::compiler::emit::Statement;
use crate::compiler::emit::TypeDescriptor;
use crate::compiler::emit::UsingCleanup;
use crate::compiler::emit::UsingResource;
use crate::compiler::emit::UsingTarget;
use crate::compiler::emit::check_sequence;
use crate::compiler::emit::lower_checked_type;
use crate::compiler::emit::matching::check_bind_target;
use crate::compiler::emit::pop_finally_holes;
use crate::compiler::emit::pop_loop_frame;
use crate::compiler::emit::scan_statements;
use crate::compiler::emit::subtract_holes;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;

fn using_target_variables<'target, 'arena>(
    target: &'target BindTarget<'arena>,
    variables: &mut Vec<&'target Variable<'arena>>,
) {
    match target {
        BindTarget::Variable(variable) => variables.push(variable),
        BindTarget::Tuple(tuple) => {
            for element in &tuple.targets {
                match element {
                    BindElement::Target(target) => using_target_variables(target, variables),
                    BindElement::Rest(rest) => {
                        if let Some(target) = &rest.target {
                            using_target_variables(target, variables);
                        }
                    }
                }
            }
        }
        BindTarget::Dict(dict) => {
            for entry in &dict.entries {
                using_target_variables(&entry.target, variables);
            }
        }
    }
}

struct PreparedUsing<'arena> {
    names: Vec<&'arena str>,
    targets: Vec<UsingTarget>,
    resources: Vec<UsingResource>,
    binding_ranges: Vec<Range<usize>>,
}

struct TryState<'source, 'arena> {
    finally_block: Option<&'source Block<'arena>>,
    defined: Vec<bool>,
    temporary_floor: u16,
    start: u32,
    end: u32,
    exceptional_ranges: Vec<(u32, u32)>,
    exits: Vec<u32>,
    entries: Vec<CatchEntry>,
}

struct GuardedCatches {
    descriptors: Vec<DescriptorIndex>,
    caught: Register,
    rejected: Vec<u32>,
}

impl<'arena> BodyCompiler<'_, 'arena> {
    pub(in crate::compiler::emit) fn statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        statement: &'source Statement<'arena>,
    ) -> Result<(), CompileError> {
        scan_statements(slice::from_ref(statement))?;
        match statement {
            Statement::Expression(expression_statement) => {
                self.expression_discarded(scope, expression_statement.expression)?;
                self.registers.release_temporaries();
                Ok(())
            }
            Statement::FinalLocal(final_local) => {
                self.final_local_statement(scope, final_local)?;
                self.registers.release_temporaries();
                Ok(())
            }
            Statement::Block(block) => self.statements_inner(scope, block.statements),
            Statement::Noop(_) => Ok(()),
            Statement::If(if_statement) => self.if_statement(scope, if_statement),
            Statement::While(loop_statement) => self.while_statement(scope, loop_statement),
            Statement::DoWhile(loop_statement) => self.do_while_statement(scope, loop_statement),
            Statement::For(loop_statement) => self.for_statement(scope, loop_statement),
            Statement::Foreach(loop_statement) => self.foreach_statement(scope, loop_statement),
            Statement::Try(try_statement) => self.try_statement(scope, try_statement),
            Statement::Using(using) => self.using_statement(scope, using),
            Statement::Namespace(_)
            | Statement::Use(_)
            | Statement::Class(_)
            | Statement::Interface(_)
            | Statement::Enum(_)
            | Statement::Function(_)
            | Statement::Constant(_)
            | Statement::TypeAlias(_)
            | Statement::Newtype(_) => Err(CompileError::new(
                CompileErrorKind::NestedDeclaration,
                "a declaration is only valid at the top level of a file or namespace",
                statement.span(),
            )),
        }
    }

    fn final_local_statement(
        &mut self,
        scope: &Scope<'_>,
        final_local: &FinalLocal<'_>,
    ) -> Result<(), CompileError> {
        let variable = &final_local.variable;
        if variable.name == "$this" {
            return Err(CompileError::new(
                CompileErrorKind::CannotBindThis,
                "`$this` cannot be declared as a final local",
                variable.span(),
            ));
        }

        let register = self.local_register(variable.name, variable.span())?;
        let Some(position) = self.local_position(variable.name) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the final local was reserved before emission") }
        };
        if self.locals[position].written {
            return Err(CompileError::new(
                CompileErrorKind::CannotAssignFinalLocal,
                format!(
                    "the final binding `{}` must be its first assignment",
                    variable.name
                ),
                variable.span(),
            ));
        }

        let value = self.expression(scope, final_local.value)?;
        self.move_into(register, value, final_local.span());
        self.mark_defined(variable.name);
        self.mark_local_final(variable.name, variable.span());
        Ok(())
    }

    pub(in crate::compiler::emit) fn emit_return(
        &mut self,
        scope: &Scope<'_>,
        return_expression: &Return<'_>,
    ) -> Result<(), CompileError> {
        if !self.shape.allows_return() {
            return Err(CompileError::new(
                CompileErrorKind::ReturnOutsideCallable,
                "`return` may only be used inside a function, method, or closure",
                return_expression.span(),
            ));
        }
        if self.shape.returns_never() {
            return Err(CompileError::new(
                CompileErrorKind::ReturnInNeverFunction,
                "a `never` function cannot return normally",
                return_expression.span(),
            ));
        }

        let finally_frames: Vec<(usize, Cleanup<'arena>)> = self
            .flow
            .frames
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, frame)| match frame {
                ControlFrame::Finally { cleanup, .. } => Some((index, cleanup.clone())),
                ControlFrame::Loop(_) => None,
            })
            .collect();
        match return_expression.value {
            Some(value) => {
                if self.shape.returns_void() {
                    return Err(CompileError::new(
                        CompileErrorKind::ValueReturnInVoidFunction,
                        "a `void` function cannot return a value",
                        return_expression.span(),
                    ));
                }
                let register = self.expression(scope, value)?;
                if finally_frames.is_empty() {
                    let instruction = self.return_instruction(register);
                    self.chunk.emit(instruction, return_expression.span());
                } else {
                    let result = self.allocate(return_expression.span())?;
                    self.move_into(result, register, return_expression.span());
                    self.clear_temporaries_except(Some(result), return_expression.span());
                    let saved_floor = self.registers.pin_temporaries();
                    let holes = self.emit_finally_copies(scope, &finally_frames)?;
                    self.registers.unpin_temporaries(saved_floor);
                    let instruction = self.return_instruction(result);
                    self.chunk.emit(instruction, return_expression.span());
                    self.finish_finally_holes(holes, self.code_position());
                }
            }
            None => {
                if finally_frames.is_empty() {
                    self.chunk
                        .emit(Instruction::ReturnNull, return_expression.span());
                } else {
                    self.clear_temporaries_except(None, return_expression.span());
                    let holes = self.emit_finally_copies(scope, &finally_frames)?;
                    self.chunk
                        .emit(Instruction::ReturnNull, return_expression.span());
                    self.finish_finally_holes(holes, self.code_position());
                }
            }
        }
        Ok(())
    }

    fn clear_temporaries_except(&mut self, preserved: Option<Register>, span: Span) {
        for index in self.registers.temporary_floor()..self.registers.mark() {
            let target = Register::new(index);
            if Some(target) != preserved {
                self.chunk.emit(Instruction::Clear { target }, span);
            }
        }
    }

    fn emit_finally_copies(
        &mut self,
        scope: &Scope<'_>,
        finally_frames: &[(usize, Cleanup<'arena>)],
    ) -> Result<Vec<(usize, u32)>, CompileError> {
        let mut holes = Vec::with_capacity(finally_frames.len());
        for (frame_index, cleanup) in finally_frames {
            holes.push((*frame_index, self.code_position()));
            self.emit_cleanup(scope, cleanup.clone())?;
        }
        Ok(holes)
    }

    fn finish_finally_holes(&mut self, holes: Vec<(usize, u32)>, end: u32) {
        for (frame_index, start) in holes {
            match &mut self.flow.frames[frame_index] {
                ControlFrame::Finally {
                    holes: frame_holes, ..
                } => frame_holes.push((start, end)),
                // SAFETY: the surrounding invariant makes this path unreachable.
                ControlFrame::Loop(_) => unsafe {
                    unreachable_invariant("a crossed return frame is a finally frame")
                },
            }
        }
    }

    fn emit_cleanup(
        &mut self,
        scope: &Scope<'_>,
        cleanup: Cleanup<'arena>,
    ) -> Result<(), CompileError> {
        let flow = mem::take(&mut self.flow);
        let result = match cleanup {
            Cleanup::Finally(block) => self.statements_inner(scope, block.statements),
            Cleanup::Using(index) => self.emit_using_cleanup(index, false),
        };
        self.flow = flow;
        result
    }

    fn emit_using_cleanup(
        &mut self,
        index: usize,
        chain_previous: bool,
    ) -> Result<(), CompileError> {
        let cleanup = &self.using_cleanups[index];
        let targets = cleanup.targets.clone();
        let resources = cleanup.resources.clone();
        let span = cleanup.span;
        let temporary_floor = self.registers.mark();

        self.clear_released_temporaries(span);

        for target in &targets {
            self.chunk.emit(
                Instruction::Clear {
                    target: target.register,
                },
                span,
            );
        }
        for target in targets {
            self.chunk.emit(
                Instruction::Move {
                    destination: target.register,
                    source: target.backup,
                },
                span,
            );
            self.chunk.emit(
                Instruction::Clear {
                    target: target.backup,
                },
                span,
            );
        }

        if resources.is_empty() {
            self.chunk.emit(Instruction::DrainFinalizers, span);
            return Ok(());
        }

        let checks_start = self.code_position();
        for resource in &resources {
            self.chunk.emit(
                Instruction::CheckSoleReference {
                    source: resource.register,
                    message: resource.message,
                    chain_previous,
                },
                span,
            );
        }
        let checks_end = self.code_position();

        for resource in &resources {
            self.chunk.emit(
                Instruction::Clear {
                    target: resource.register,
                },
                span,
            );
        }
        self.chunk.emit(Instruction::DrainFinalizers, span);

        let exit = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            span,
        );
        let handler = self.code_position();
        for resource in resources {
            self.chunk.emit(
                Instruction::Clear {
                    target: resource.register,
                },
                span,
            );
        }
        self.chunk.emit(Instruction::DrainFinalizers, span);
        self.chunk.emit(Instruction::Rethrow, span);

        let catch_all = self.add_type_descriptor(TypeDescriptor::Mixed, span)?;
        self.chunk.catch_table.push(CatchEntry {
            start: checks_start,
            end: checks_end,
            handler,
            type_descriptor: catch_all,
            temporary_floor,
            binding: None,
        });

        let end = self.code_position();
        self.chunk.patch_jump(exit, end);
        Ok(())
    }

    pub(in crate::compiler::emit) fn loop_jump(
        &mut self,
        scope: &Scope<'_>,
        level: u64,
        kind: LoopJump,
        span: Span,
    ) -> Result<(), CompileError> {
        let name = match kind {
            LoopJump::Break => "break",
            LoopJump::Continue => "continue",
        };
        if level == 0 {
            return Err(CompileError::new(
                CompileErrorKind::LoopJumpOutsideLoop,
                format!("a `{name}` level must be at least one"),
                span,
            ));
        }
        let mut remaining = level;
        let mut target = None;
        let mut finally_frames = Vec::new();
        for (index, frame) in self.flow.frames.iter().enumerate().rev() {
            match frame {
                ControlFrame::Loop(_) => {
                    remaining -= 1;
                    if remaining == 0 {
                        target = Some(index);
                        break;
                    }
                }
                ControlFrame::Finally { cleanup, .. } => {
                    finally_frames.push((index, cleanup.clone()));
                }
            }
        }
        let Some(target) = target else {
            return Err(CompileError::new(
                CompileErrorKind::LoopJumpOutsideLoop,
                format!("`{name} {level}` names more loops than enclose it"),
                span,
            ));
        };
        self.clear_temporaries_except(None, span);
        let saved_floor = self.registers.pin_temporaries();
        let holes = self.emit_finally_copies(scope, &finally_frames)?;
        self.registers.unpin_temporaries(saved_floor);
        let jump = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            span,
        );
        let escaping = self.save_defined();
        match &mut self.flow.frames[target] {
            ControlFrame::Loop(frame) => {
                frame.escape_states.push(escaping);
                match kind {
                    LoopJump::Break => frame.break_jumps.push(jump),
                    LoopJump::Continue => frame.continue_jumps.push(jump),
                }
            }
            ControlFrame::Finally { .. } => {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("the jump target is a loop frame") };
            }
        }
        self.finish_finally_holes(holes, self.code_position());
        Ok(())
    }
    fn condition_jumps(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
        when_true: bool,
    ) -> Result<Vec<u32>, CompileError> {
        if let Expression::Binary(binary) = expression.unparenthesized() {
            let direct = matches!(binary.operator, BinaryOperator::And(_)) && !when_true
                || matches!(binary.operator, BinaryOperator::Or(_)) && when_true;
            if direct {
                let mut jumps = self.condition_jumps(scope, binary.lhs, when_true)?;
                let saved = self.save_defined();
                jumps.extend(self.condition_jumps(scope, binary.rhs, when_true)?);
                self.restore_defined(saved);
                return Ok(jumps);
            }

            let bypass = matches!(binary.operator, BinaryOperator::And(_)) && when_true
                || matches!(binary.operator, BinaryOperator::Or(_)) && !when_true;
            if bypass {
                let skips = self.condition_jumps(scope, binary.lhs, !when_true)?;
                let saved = self.save_defined();
                let jumps = self.condition_jumps(scope, binary.rhs, when_true)?;
                self.restore_defined(saved);
                let after = self.code_position();
                for skip in skips {
                    self.chunk.patch_jump(skip, after);
                }
                return Ok(jumps);
            }

            let null_jump = match (
                binary.operator,
                binary.lhs.unparenthesized(),
                binary.rhs.unparenthesized(),
            ) {
                (BinaryOperator::Equal(_), Expression::Literal(Literal::Null(_)), subject)
                | (BinaryOperator::Equal(_), subject, Expression::Literal(Literal::Null(_))) => {
                    Some((subject, !when_true))
                }
                (BinaryOperator::NotEqual(_), Expression::Literal(Literal::Null(_)), subject)
                | (BinaryOperator::NotEqual(_), subject, Expression::Literal(Literal::Null(_))) => {
                    Some((subject, when_true))
                }
                _ => None,
            };
            if let Some((subject, jump_if_not_null)) = null_jump {
                let subject = self.expression(scope, subject)?;
                let offset = JumpOffset::new(0);
                let instruction = if jump_if_not_null {
                    Instruction::JumpIfNotNull { subject, offset }
                } else {
                    Instruction::JumpIfNull { subject, offset }
                };

                return Ok(vec![self.chunk.emit(instruction, expression.span())]);
            }
        }

        let first_instruction = self.chunk.code.len();
        let condition = self.expression(scope, expression)?;
        let instruction = if when_true {
            Instruction::JumpIfTrue {
                condition,
                offset: JumpOffset::new(0),
            }
        } else {
            Instruction::JumpIfFalse {
                condition,
                offset: JumpOffset::new(0),
            }
        };
        let jump = self.chunk.emit(instruction, expression.span());
        self.record_branch_fusion(first_instruction, condition, jump);
        Ok(vec![jump])
    }

    pub(in crate::compiler::emit) fn if_statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        if_statement: &'source If<'arena>,
    ) -> Result<(), CompileError> {
        let mut exits = Vec::new();
        let mut current = if_statement;
        let before = self.save_defined();
        let mut arm_ends: Vec<Vec<bool>> = Vec::new();
        let mut has_final_else = false;
        loop {
            let mark = self.registers.mark();
            let skips = self.condition_jumps(scope, current.condition, false)?;
            self.registers.release_to(mark);
            let saved = self.save_defined();
            self.statements_inner(scope, current.body.statements)?;
            arm_ends.push(self.save_defined());
            self.restore_defined(saved);
            match &current.r#else {
                None => {
                    let after = self.code_position();
                    for skip in skips {
                        self.chunk.patch_jump(skip, after);
                    }
                    break;
                }
                Some(else_clause) => {
                    exits.push(self.chunk.emit(
                        Instruction::Jump {
                            offset: JumpOffset::new(0),
                        },
                        else_clause.r#else.span(),
                    ));
                    let after = self.code_position();
                    for skip in skips {
                        self.chunk.patch_jump(skip, after);
                    }
                    match &else_clause.body {
                        ElseBody::If(nested) => {
                            current = nested;
                        }
                        ElseBody::Block(block) => {
                            let saved = self.save_defined();
                            self.statements_inner(scope, block.statements)?;
                            arm_ends.push(self.save_defined());
                            self.restore_defined(saved);
                            has_final_else = true;
                            break;
                        }
                    }
                }
            }
        }
        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }
        let mut merged = if has_final_else {
            arm_ends[0].clone()
        } else {
            before
        };
        for state in &arm_ends {
            for (slot, defined) in merged.iter_mut().zip(state) {
                *slot = *slot && *defined;
            }
        }
        self.restore_defined(merged);
        Ok(())
    }
    fn while_statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        loop_statement: &'source While<'arena>,
    ) -> Result<(), CompileError> {
        let head = self.code_position();
        self.flow.frames.push(ControlFrame::Loop(LoopFrame::new()));
        let mark = self.registers.mark();
        let exits = self.condition_jumps(scope, loop_statement.condition, false)?;
        self.registers.release_to(mark);
        let saved = self.save_defined();
        self.statements_inner(scope, loop_statement.body.statements)?;
        let frame = pop_loop_frame(&mut self.flow);
        self.emit_loop_backedge(head, loop_statement.span());
        let after = self.code_position();
        for exit in exits {
            self.chunk.patch_jump(exit, after);
        }
        for jump in frame.break_jumps {
            self.chunk.patch_jump(jump, after);
        }
        for jump in frame.continue_jumps {
            self.chunk.patch_jump(jump, head);
        }
        self.restore_defined(saved);
        Ok(())
    }

    fn do_while_statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        loop_statement: &'source DoWhile<'arena>,
    ) -> Result<(), CompileError> {
        let head = self.code_position();
        self.flow.frames.push(ControlFrame::Loop(LoopFrame::new()));
        self.statements_inner(scope, loop_statement.body.statements)?;
        let condition_position = self.code_position();
        let mark = self.registers.mark();
        let backs = self.condition_jumps(scope, loop_statement.condition, true)?;
        self.registers.release_to(mark);
        let frame = pop_loop_frame(&mut self.flow);
        for escaping in &frame.escape_states {
            self.intersect_defined(escaping);
        }
        for back in backs {
            self.chunk.patch_jump(back, head);
        }
        let after = self.code_position();
        for jump in frame.break_jumps {
            self.chunk.patch_jump(jump, after);
        }
        for jump in frame.continue_jumps {
            self.chunk.patch_jump(jump, condition_position);
        }
        Ok(())
    }

    fn for_statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        loop_statement: &'source For<'arena>,
    ) -> Result<(), CompileError> {
        for initialization in &loop_statement.initializations {
            let mark = self.registers.mark();
            self.expression_discarded(scope, initialization)?;
            self.registers.release_to(mark);
        }
        let head = self.code_position();
        self.flow.frames.push(ControlFrame::Loop(LoopFrame::new()));
        let mut exits = None;
        let condition_count = loop_statement.conditions.len();
        for (position, condition) in loop_statement.conditions.iter().enumerate() {
            let mark = self.registers.mark();
            if position + 1 == condition_count {
                exits = Some(self.condition_jumps(scope, condition, false)?);
            } else {
                self.expression_discarded(scope, condition)?;
            }
            self.registers.release_to(mark);
        }
        let saved = self.save_defined();
        self.statements_inner(scope, loop_statement.body.statements)?;
        let frame = pop_loop_frame(&mut self.flow);
        let continue_target = self.code_position();
        self.flow.frames.push(ControlFrame::Loop(LoopFrame::new()));
        for increment in &loop_statement.increments {
            let mark = self.registers.mark();
            self.expression_discarded(scope, increment)?;
            self.registers.release_to(mark);
        }
        let increment_frame = pop_loop_frame(&mut self.flow);
        self.emit_loop_backedge(head, loop_statement.span());
        let after = self.code_position();
        if let Some(exits) = exits {
            for exit in exits {
                self.chunk.patch_jump(exit, after);
            }
        }
        for jump in frame.break_jumps {
            self.chunk.patch_jump(jump, after);
        }
        for jump in frame.continue_jumps {
            self.chunk.patch_jump(jump, continue_target);
        }

        for jump in increment_frame.break_jumps {
            self.chunk.patch_jump(jump, after);
        }

        for jump in increment_frame.continue_jumps {
            self.chunk.patch_jump(jump, head);
        }

        self.restore_defined(saved);
        Ok(())
    }

    fn foreach_statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        loop_statement: &'source Foreach<'arena>,
    ) -> Result<(), CompileError> {
        let outer_mark = self.registers.mark();
        let subject = self.expression(scope, loop_statement.expression)?;
        let iterator = self.allocate(loop_statement.span())?;
        self.chunk.emit(
            Instruction::ForeachInit {
                iterator,
                subject,
                reserve: Register::NONE,
            },
            loop_statement.expression.span(),
        );
        let key_slot = if loop_statement.target.key().is_some() {
            self.allocate(loop_statement.span())?
        } else {
            Register::NONE
        };
        let value_slot = self.allocate(loop_statement.span())?;
        let saved_floor = self.registers.pin_temporaries();
        let saved = self.save_defined();
        let head = self.code_position();
        self.chunk.emit(
            Instruction::ForeachNext {
                iterator,
                key_destination: key_slot,
                value_destination: value_slot,
            },
            loop_statement.r#as.span(),
        );
        let exit = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            loop_statement.r#as.span(),
        );
        let mark = self.registers.mark();
        if let Some(key_target) = loop_statement.target.key() {
            let place = self.prepare_place(scope, key_target)?;
            self.write_place(scope, &place, key_slot, key_target.span())?;
        }
        let value_target = loop_statement.target.value();
        let place = self.prepare_place(scope, value_target)?;
        self.write_place(scope, &place, value_slot, value_target.span())?;
        self.registers.release_to(mark);
        self.flow.frames.push(ControlFrame::Loop(LoopFrame::new()));
        self.statements_inner(scope, loop_statement.body.statements)?;
        let frame = pop_loop_frame(&mut self.flow);
        self.emit_loop_backedge(head, loop_statement.span());
        let after = self.code_position();
        self.chunk.patch_jump(exit, after);
        for jump in frame.break_jumps {
            self.chunk.patch_jump(jump, after);
        }
        for jump in frame.continue_jumps {
            self.chunk.patch_jump(jump, head);
        }
        self.restore_defined(saved);
        self.registers.unpin_temporaries(saved_floor);
        self.registers.release_to(outer_mark);
        Ok(())
    }

    pub(in crate::compiler::emit) fn using_statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        using: &'source Using<'arena>,
    ) -> Result<(), CompileError> {
        let before = self.save_defined();
        let outer_mark = self.registers.mark();
        let prepared = self.prepare_using(using)?;
        let saved_floor = self.registers.pin_temporaries();
        let temporary_floor = self.registers.mark();
        let cleanup_index = self.using_cleanups.len();
        self.using_cleanups.push(UsingCleanup {
            resources: prepared.resources.clone(),
            targets: prepared.targets.clone(),
            span: using.span(),
        });

        let cleanup = Cleanup::Using(cleanup_index);
        let protected_start = self.code_position();
        self.flow.frames.push(ControlFrame::Finally {
            cleanup: cleanup.clone(),
            holes: Vec::new(),
        });

        self.bind_using_resources(scope, using, &prepared)?;
        self.statements_inner(scope, using.body.statements)?;
        let holes = pop_finally_holes(&mut self.flow, true);
        let protected_end = self.code_position();
        let mut after = self.save_defined();
        for name in prepared.names {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let position = unsafe {
                unwrap_option_invariant(
                    self.local_position(name),
                    "a using target has a reserved local",
                )
            };
            after[position] = before[position];
        }
        self.emit_cleanup(scope, cleanup)?;
        let mut exceptional_ranges = Vec::new();
        subtract_holes(
            protected_start,
            protected_end,
            &holes,
            &mut exceptional_ranges,
        );

        self.emit_using_exception_handler(
            cleanup_index,
            using.span(),
            exceptional_ranges,
            before,
            temporary_floor,
        )?;

        self.restore_defined(after);
        self.registers.unpin_temporaries(saved_floor);
        self.registers.release_to(outer_mark);
        Ok(())
    }

    fn prepare_using(
        &mut self,
        using: &Using<'arena>,
    ) -> Result<PreparedUsing<'arena>, CompileError> {
        let mut names = Vec::new();
        let mut targets = Vec::new();
        let mut binding_ranges = Vec::with_capacity(using.bindings.len());
        for binding in &using.bindings {
            check_bind_target(&binding.target)?;
            let start = targets.len();
            let mut variables = Vec::new();
            using_target_variables(&binding.target, &mut variables);
            for variable in variables {
                if variable.name == "$this" {
                    return Err(CompileError::new(
                        CompileErrorKind::CannotBindThis,
                        "`$this` cannot be bound by `using`",
                        variable.span(),
                    ));
                }
                if names.contains(&variable.name) {
                    return Err(CompileError::new(
                        CompileErrorKind::DuplicateUsingBinding,
                        format!(
                            "the variable `{}` is bound more than once by this `using` statement",
                            variable.name
                        ),
                        variable.span(),
                    ));
                }
                names.push(variable.name);
                self.ensure_local_writable(variable.name, variable.span())?;
                let register = self.local_register(variable.name, variable.span())?;
                let backup = self.allocate(variable.span())?;
                self.chunk.emit(
                    Instruction::Move {
                        destination: backup,
                        source: register,
                    },
                    variable.span(),
                );
                targets.push(UsingTarget {
                    register,
                    backup,
                    span: variable.span(),
                });
            }
            binding_ranges.push(start..targets.len());
        }
        let resources = self.prepare_using_resources(&names, &targets)?;
        Ok(PreparedUsing {
            names,
            targets,
            resources,
            binding_ranges,
        })
    }

    fn prepare_using_resources(
        &mut self,
        names: &[&str],
        targets: &[UsingTarget],
    ) -> Result<Vec<UsingResource>, CompileError> {
        let mut resources = Vec::with_capacity(targets.len());
        for (name, target) in names.iter().zip(targets) {
            let resource = self.allocate(target.span)?;
            let message = format!(
                "the resource bound to {name} is still held by another strong reference at the end of the `using` block"
            );
            let message = self.add_constant(
                BytecodeLiteral::String(self.heap.intern(message.as_bytes())),
                target.span,
            )?;
            self.chunk.emit(
                Instruction::LoadNull {
                    destination: resource,
                },
                target.span,
            );
            resources.push(UsingResource {
                register: resource,
                message,
            });
        }
        Ok(resources)
    }

    fn bind_using_resources(
        &mut self,
        scope: &Scope<'_>,
        using: &Using<'_>,
        prepared: &PreparedUsing<'_>,
    ) -> Result<(), CompileError> {
        for (binding, range) in using.bindings.iter().zip(&prepared.binding_ranges) {
            let mark = self.registers.mark();
            let mut keys = Vec::new();
            self.prepare_bind_keys(scope, &binding.target, &mut keys)?;
            let value = self.expression(scope, binding.value)?;
            let mut key = 0;
            self.bind_target(
                &binding.target,
                value,
                binding.target.span(),
                &keys,
                &mut key,
            )?;
            for position in range.clone() {
                self.chunk.emit(
                    Instruction::Move {
                        destination: prepared.resources[position].register,
                        source: prepared.targets[position].register,
                    },
                    prepared.targets[position].span,
                );
            }
            for index in mark..self.registers.mark() {
                self.chunk.emit(
                    Instruction::Clear {
                        target: Register::new(index),
                    },
                    binding.span(),
                );
            }
            self.registers.release_to(mark);
            self.registers.release_temporaries();
        }
        Ok(())
    }

    fn emit_using_exception_handler(
        &mut self,
        cleanup_index: usize,
        span: Span,
        ranges: Vec<(u32, u32)>,
        defined: Vec<bool>,
        temporary_floor: u16,
    ) -> Result<(), CompileError> {
        if ranges.is_empty() {
            return Ok(());
        }
        let exit = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            span,
        );
        let handler = self.code_position();
        self.restore_defined(defined);
        self.emit_using_cleanup(cleanup_index, true)?;
        self.chunk.emit(Instruction::Rethrow, span);
        let catch_all = self.add_type_descriptor(TypeDescriptor::Mixed, span)?;
        for (start, end) in ranges {
            self.chunk.catch_table.push(CatchEntry {
                start,
                end,
                handler,
                type_descriptor: catch_all,
                temporary_floor,
                binding: None,
            });
        }
        let end = self.code_position();
        self.chunk.patch_jump(exit, end);
        Ok(())
    }

    pub(in crate::compiler::emit) fn try_statement<'source>(
        &mut self,
        scope: &Scope<'_>,
        try_statement: &'source Try<'arena>,
    ) -> Result<(), CompileError> {
        if try_statement.catch_clauses.is_empty()
            && try_statement.else_clause.is_none()
            && try_statement.finally_clause.is_none()
        {
            return Err(CompileError::new(
                CompileErrorKind::TryWithoutClause,
                "a `try` needs a `catch`, `else`, or `finally`; on its own it does nothing",
                try_statement.r#try.span(),
            ));
        }
        check_sequence(
            CompileErrorKind::TooManyCatchClauses,
            "one `try` statement may have",
            "catch clauses",
            try_statement.catch_clauses,
        )?;

        let mut state = self.prepare_try(scope, try_statement)?;
        if try_statement
            .catch_clauses
            .iter()
            .any(|clause| clause.guard.is_some())
        {
            self.emit_guarded_catches(scope, try_statement, &mut state)?;
        } else {
            self.emit_direct_catches(scope, try_statement, &mut state)?;
        }

        self.emit_finally_handler(scope, &mut state)?;
        let after = self.code_position();
        for exit in state.exits {
            self.chunk.patch_jump(exit, after);
        }
        self.chunk.catch_table.extend(state.entries);
        self.restore_defined(state.defined);
        Ok(())
    }

    fn prepare_try<'source>(
        &mut self,
        scope: &Scope<'_>,
        statement: &'source Try<'arena>,
    ) -> Result<TryState<'source, 'arena>, CompileError> {
        let finally_block = statement
            .finally_clause
            .as_ref()
            .map(|clause| &clause.block);
        let defined = self.save_defined();
        let temporary_floor = self.registers.mark();
        let start = self.code_position();
        if let Some(block) = finally_block {
            self.flow.frames.push(ControlFrame::Finally {
                cleanup: Cleanup::Finally(block.clone()),
                holes: Vec::new(),
            });
        }

        self.statements_inner(scope, statement.block.statements)?;
        let body_holes = pop_finally_holes(&mut self.flow, finally_block.is_some());
        let end = self.code_position();
        let normal_end = self.save_defined();
        let mut exceptional_ranges = Vec::new();
        subtract_holes(start, end, &body_holes, &mut exceptional_ranges);
        if let Some(else_clause) = &statement.else_clause {
            self.restore_defined(normal_end);
            let else_start = self.code_position();
            if let Some(block) = finally_block {
                self.flow.frames.push(ControlFrame::Finally {
                    cleanup: Cleanup::Finally(block.clone()),
                    holes: Vec::new(),
                });
            }

            self.statements_inner(scope, else_clause.block.statements)?;
            let else_holes = pop_finally_holes(&mut self.flow, finally_block.is_some());
            subtract_holes(
                else_start,
                self.code_position(),
                &else_holes,
                &mut exceptional_ranges,
            );
        }
        self.restore_defined(defined.clone());
        if let Some(block) = finally_block {
            self.emit_cleanup(scope, Cleanup::Finally(block.clone()))?;
        }

        let exit = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            statement.r#try.span(),
        );
        Ok(TryState {
            finally_block,
            defined,
            temporary_floor,
            start,
            end,
            exceptional_ranges,
            exits: vec![exit],
            entries: Vec::new(),
        })
    }

    fn emit_guarded_catches<'source>(
        &mut self,
        scope: &Scope<'_>,
        statement: &'source Try<'arena>,
        state: &mut TryState<'source, 'arena>,
    ) -> Result<(), CompileError> {
        let descriptors = statement
            .catch_clauses
            .iter()
            .map(|clause| {
                let descriptor = lower_checked_type(&self.types(scope), clause.r#type)?;
                self.add_type_descriptor(descriptor, clause.r#catch.span())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let register_mark = self.registers.mark();
        let caught = self.allocate(statement.r#try.span())?;
        let saved_floor = self.registers.pin_temporaries();
        let mut guarded = GuardedCatches {
            descriptors,
            caught,
            rejected: Vec::new(),
        };

        for (index, clause) in statement.catch_clauses.iter().enumerate() {
            self.emit_guarded_catch(scope, clause, index, state, &mut guarded)?;
        }

        let rejected_handler = self.code_position();
        for jump in guarded.rejected {
            self.chunk.patch_jump(jump, rejected_handler);
        }
        self.restore_defined(state.defined.clone());
        if let Some(block) = state.finally_block {
            self.emit_cleanup(scope, Cleanup::Finally(block.clone()))?;
        }

        self.chunk
            .emit(Instruction::Rethrow, statement.r#try.span());
        self.registers.unpin_temporaries(saved_floor);
        self.registers.release_to(register_mark);
        Ok(())
    }

    fn emit_guarded_catch<'source>(
        &mut self,
        scope: &Scope<'_>,
        clause: &'source TryCatchClause<'arena>,
        index: usize,
        state: &mut TryState<'source, 'arena>,
        guarded: &mut GuardedCatches,
    ) -> Result<(), CompileError> {
        let dispatch = self.code_position();
        for jump in &guarded.rejected {
            self.chunk.patch_jump(*jump, dispatch);
        }
        guarded.rejected.clear();
        let clause_start = dispatch;
        if index != 0 {
            let matches = self.allocate(clause.r#catch.span())?;
            self.chunk.emit(
                Instruction::Is {
                    destination: matches,
                    source: guarded.caught,
                    descriptor: guarded.descriptors[index],
                },
                clause.r#catch.span(),
            );
            guarded.rejected.push(self.chunk.emit(
                Instruction::JumpIfFalse {
                    condition: matches,
                    offset: JumpOffset::new(0),
                },
                clause.r#catch.span(),
            ));
            self.registers.release_temporaries();
        }
        let handler = self.code_position();
        self.restore_defined(state.defined.clone());
        if let Some(variable) = &clause.variable {
            self.ensure_local_writable(variable.name, variable.span())?;
            let binding = self.local_register(variable.name, clause.r#catch.span())?;
            self.move_into(binding, guarded.caught, clause.r#catch.span());
            self.mark_defined(variable.name);
        }

        if let Some(block) = state.finally_block {
            self.flow.frames.push(ControlFrame::Finally {
                cleanup: Cleanup::Finally(block.clone()),
                holes: Vec::new(),
            });
        }
        if let Some(guard) = &clause.guard {
            let condition = self.expression(scope, guard.condition)?;
            guarded.rejected.push(self.chunk.emit(
                Instruction::JumpIfFalse {
                    condition,
                    offset: JumpOffset::new(0),
                },
                guard.span(),
            ));
            self.registers.release_temporaries();
        }

        self.statements_inner(scope, clause.block.statements)?;
        let catch_holes = pop_finally_holes(&mut self.flow, state.finally_block.is_some());
        subtract_holes(
            clause_start,
            self.code_position(),
            &catch_holes,
            &mut state.exceptional_ranges,
        );
        self.restore_defined(state.defined.clone());
        if let Some(block) = state.finally_block {
            self.emit_cleanup(scope, Cleanup::Finally(block.clone()))?;
        }
        state.exits.push(self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            clause.r#catch.span(),
        ));
        state.entries.push(CatchEntry {
            start: state.start,
            end: state.end,
            handler,
            type_descriptor: guarded.descriptors[index],
            temporary_floor: state.temporary_floor,
            binding: Some(guarded.caught),
        });
        Ok(())
    }

    fn emit_direct_catches<'source>(
        &mut self,
        scope: &Scope<'_>,
        statement: &'source Try<'arena>,
        state: &mut TryState<'source, 'arena>,
    ) -> Result<(), CompileError> {
        for clause in statement.catch_clauses {
            let handler = self.code_position();
            let descriptor = lower_checked_type(&self.types(scope), clause.r#type)?;
            let descriptor = self.add_type_descriptor(descriptor, clause.r#catch.span())?;
            self.restore_defined(state.defined.clone());
            let binding = match &clause.variable {
                Some(variable) => {
                    self.ensure_local_writable(variable.name, variable.span())?;
                    let register = self.local_register(variable.name, clause.r#catch.span())?;
                    self.mark_defined(variable.name);
                    Some(register)
                }
                None => None,
            };
            if let Some(block) = state.finally_block {
                self.flow.frames.push(ControlFrame::Finally {
                    cleanup: Cleanup::Finally(block.clone()),
                    holes: Vec::new(),
                });
            }

            self.statements_inner(scope, clause.block.statements)?;
            let catch_holes = pop_finally_holes(&mut self.flow, state.finally_block.is_some());
            subtract_holes(
                handler,
                self.code_position(),
                &catch_holes,
                &mut state.exceptional_ranges,
            );
            self.restore_defined(state.defined.clone());
            if let Some(block) = state.finally_block {
                self.emit_cleanup(scope, Cleanup::Finally(block.clone()))?;
            }
            state.exits.push(self.chunk.emit(
                Instruction::Jump {
                    offset: JumpOffset::new(0),
                },
                clause.r#catch.span(),
            ));
            state.entries.push(CatchEntry {
                start: state.start,
                end: state.end,
                handler,
                type_descriptor: descriptor,
                temporary_floor: state.temporary_floor,
                binding,
            });
        }
        Ok(())
    }

    fn emit_finally_handler(
        &mut self,
        scope: &Scope<'_>,
        state: &mut TryState<'_, 'arena>,
    ) -> Result<(), CompileError> {
        let Some(block) = state.finally_block else {
            return Ok(());
        };
        let handler = self.code_position();
        self.restore_defined(state.defined.clone());
        self.emit_cleanup(scope, Cleanup::Finally(block.clone()))?;
        self.chunk
            .emit(Instruction::Rethrow, block.right_brace.span());
        let catch_all =
            self.add_type_descriptor(TypeDescriptor::Mixed, block.right_brace.span())?;
        for &(start, end) in &state.exceptional_ranges {
            state.entries.push(CatchEntry {
                start,
                end,
                handler,
                type_descriptor: catch_all,
                temporary_floor: state.temporary_floor,
                binding: None,
            });
        }
        state.exceptional_ranges.clear();
        Ok(())
    }
}
