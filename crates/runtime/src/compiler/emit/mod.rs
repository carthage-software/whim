//! Function-body emission: the compiler state, its locals, and its registers.

use std::mem;

use hashbrown::HashMap;
use hashbrown::HashSet;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::access::Access;
use whim_syn::cst::access::ClassReference;
use whim_syn::cst::access::NullSafePropertyAccess;
use whim_syn::cst::access::PropertyAccess;
use whim_syn::cst::array::ArrayAccess;
use whim_syn::cst::array::DictEntry;
use whim_syn::cst::array::TupleElement;
use whim_syn::cst::array::VecElement;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::atom::Variable;
use whim_syn::cst::call::Argument;
use whim_syn::cst::call::ArgumentList;
use whim_syn::cst::call::Call;
use whim_syn::cst::call::Callee;
use whim_syn::cst::call::MethodCall;
use whim_syn::cst::call::NullSafeMethodCall;
use whim_syn::cst::call::PartialApplication;
use whim_syn::cst::call::PartialArgument;
use whim_syn::cst::construct::Construct;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::operation::AssignmentOperator;
use whim_syn::cst::operation::AssignmentTarget;
use whim_syn::cst::operation::DestructureTarget;
use whim_syn::cst::statement::Block;
use whim_syn::cst::statement::Statement;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::SIDE_TABLE_CAPACITY;
use crate::bytecode::chunk::SideTableFull;
use crate::bytecode::chunk::descriptors::CallDescriptor;
use crate::bytecode::chunk::descriptors::CatchEntry;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::Literal as BytecodeLiteral;
use crate::bytecode::chunk::descriptors::LiteralKey;
use crate::bytecode::chunk::descriptors::PresetDescriptor;
use crate::bytecode::chunk::descriptors::PresetSlot;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::chunk::descriptors::literal_key;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::AsMode;
use crate::bytecode::instruction::operands::CallDescriptorIndex;
use crate::bytecode::instruction::operands::Comparison;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::Count;
use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::ImmediateInt;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::PresetDescriptorIndex;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::rewrite::compact;
use crate::bytecode::rewrite::control_flow_targets;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::compiler::embed::EmbeddedFiles;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::limits::check_count;
use crate::compiler::limits::check_sequence;
use crate::compiler::limits::check_tuple_count;
use crate::compiler::limits::check_tuple_sequence;
use crate::compiler::names::Resolver;
use crate::compiler::registers::REGISTER_CAPACITY;
use crate::compiler::registers::Registers;
use crate::compiler::types::ClassContext;
use crate::compiler::types::GenericTable;
use crate::compiler::types::TypeScope;
use crate::compiler::types::lowering::lower_checked_type;
use crate::compiler::types::lowering::lower_pattern_type;
use crate::compiler::types::lowering::lower_type;
use crate::compiler::types::rendering::check_call_type_argument_arity;
use crate::compiler::types::rendering::check_type_argument_arity;
use crate::unreachable_invariant;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

pub(in crate::compiler) struct Scope<'compilation> {
    /// The engine heap names are interned in.
    pub(in crate::compiler) heap: &'compilation Heap,
    /// The path of the source file being compiled.
    pub(in crate::compiler) runtime_path: &'compilation [u8],
    /// Byte offsets for the start of each source line.
    pub(in crate::compiler) line_starts: &'compilation [u32],
    /// The name resolver of the enclosing namespace region.
    pub(in crate::compiler) resolver: &'compilation Resolver,
    /// The enclosing class, when inside one.
    pub(in crate::compiler) class: Option<&'compilation ClassContext>,
    /// The type-parameter names in scope: the binders of the enclosing
    /// function or method, together with any from an enclosing generic class.
    /// A bare local name matching one of these lowers to a runtime parameter
    /// descriptor resolved through the active specialization environment.
    pub(in crate::compiler) binders: Vec<String>,
    /// Class binders unavailable in this static context. A nested callable may
    /// shadow one with its own active binder.
    pub(in crate::compiler) forbidden_binders: Vec<String>,
    /// The generic declarations the unit makes, for arity checking.
    pub(in crate::compiler) generics: &'compilation GenericTable<'compilation>,
    /// Files read by `embed!` during this compilation.
    pub(in crate::compiler) embedded_files: &'compilation EmbeddedFiles,
    /// Whether the enclosing compilation is trusted code whose written return
    /// types are guaranteed by review, so returns compile unchecked.
    pub(in crate::compiler) trusted_returns: bool,
}

struct Local {
    name: String,
    /// The local's fixed register.
    register: Register,
    defined: bool,
    written: bool,
    final_span: Option<Span>,
}

struct ScopedBinding {
    span: Span,
    /// The register visible only while its arm is compiled.
    register: Register,
}

#[derive(Clone, Copy)]
struct UsingTarget {
    register: Register,
    backup: Register,
    span: Span,
}

#[derive(Clone, Copy)]
struct UsingResource {
    register: Register,
    message: ConstantIndex,
}

struct UsingCleanup {
    resources: Vec<UsingResource>,
    targets: Vec<UsingTarget>,
    span: Span,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ReturnKind {
    Forbidden,
    Value,
    Void,
    Never,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler::emit) enum ValueUse {
    Needed,
    Discarded,
}

impl ValueUse {
    const fn discarded(self) -> bool {
        matches!(self, Self::Discarded)
    }
}

fn wide_code_position(position: usize) -> i64 {
    let Ok(position) = u32::try_from(position) else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a chunk stays within the thirty-two-bit index space") }
    };

    i64::from(position)
}

impl ReturnKind {
    pub(in crate::compiler) const fn callable(returns_void: bool, returns_never: bool) -> Self {
        if returns_void {
            Self::Void
        } else if returns_never {
            Self::Never
        } else {
            Self::Value
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::compiler) struct BodyShape {
    /// Whether the body is an instance method with `$this` at register zero.
    pub is_instance_method: bool,
    /// The body's return contract.
    pub return_kind: ReturnKind,
    /// Whether the body is a constructor whose promoted parameters write
    /// their properties in the prologue.
    pub promote_parameters: bool,
    /// Whether the body belongs to trusted code whose written return types
    /// are guaranteed by review, so returns compile unchecked. Only the
    /// standard library compiles this way.
    pub trusted_returns: bool,
}

impl BodyShape {
    const fn allows_return(self) -> bool {
        !matches!(self.return_kind, ReturnKind::Forbidden)
    }

    const fn returns_void(self) -> bool {
        matches!(self.return_kind, ReturnKind::Void)
    }

    const fn returns_never(self) -> bool {
        matches!(self.return_kind, ReturnKind::Never)
    }
}

pub(in crate::compiler) struct BodyCompiler<'compilation, 'arena> {
    heap: &'compilation Heap,
    path: &'compilation str,
    /// The exact platform path bytes recorded in runtime values.
    runtime_path: &'compilation [u8],
    chunk: Chunk,
    registers: Registers,
    /// The named locals, in register order.
    locals: Vec<Local>,
    /// Positions in `locals` by name, newest last, so lookup never scans.
    local_index: HashMap<String, Vec<usize>>,
    /// Pool positions by literal identity, so interning never scans.
    constant_index: HashMap<LiteralKey, ConstantIndex>,
    scoped_bindings: Vec<ScopedBinding>,
    using_cleanups: Vec<UsingCleanup>,
    /// Compare instructions emitted only to feed their following branch.
    branch_fusions: Vec<usize>,
    /// Loop increments emitted next to their backedge.
    loop_backedge_fusions: Vec<usize>,
    flow: ControlFlow<'arena>,
    synthesized: &'compilation mut Vec<CompiledFunction>,
    source_text: &'compilation str,
    aliases: &'compilation [CompiledTypeAlias],
    shape: BodyShape,
}

pub(in crate::compiler) fn integer_gate(
    value: u64,
    negated: bool,
    span: Span,
) -> Result<i64, CompileError> {
    let value = i128::from(value);
    i64::try_from(if negated { -value } else { value }).map_err(|_| {
        CompileError::new(
            CompileErrorKind::IntegerLiteralOutOfRange,
            "the integer literal does not fit a 64-bit signed integer",
            span,
        )
    })
}

fn argument_gate(count: usize, span: Span) -> Result<u8, CompileError> {
    check_count(
        CompileErrorKind::TooManyArguments,
        "a call may pass",
        "arguments",
        count,
        span,
    )
}

fn written_value_gate(count: usize, span: Span) -> Result<u8, CompileError> {
    check_count(
        CompileErrorKind::TooManyWrittenValues,
        "a write construct may pass",
        "values",
        count,
        span,
    )
}

fn capture_gate(count: usize, span: Span) -> Result<u8, CompileError> {
    check_count(
        CompileErrorKind::TooManyCaptures,
        "a function may capture",
        "variables from its enclosing scope",
        count,
        span,
    )
}

fn tuple_window_gate(count: usize, span: Span) -> Result<u8, CompileError> {
    check_tuple_count(
        CompileErrorKind::TooManyTupleElements,
        "a tuple may have",
        "elements",
        count,
        span,
    )
}

fn register_limit(span: Span) -> CompileError {
    CompileError::new(
        CompileErrorKind::TooManyRegisters,
        format!(
            "a function may use at most {REGISTER_CAPACITY} locals and live \
             temporaries at once"
        ),
        span,
    )
}

fn side_table_limit(full: SideTableFull, span: Span) -> CompileError {
    CompileError::new(
        CompileErrorKind::SideTableFull,
        format!(
            "a function may contain at most {SIDE_TABLE_CAPACITY} {}",
            full.table.counts()
        ),
        span,
    )
}

fn line_and_column(line_starts: &[u32], offset: u32) -> (u32, u32) {
    let line = line_starts.partition_point(|start| *start <= offset);
    let line_start = line
        .checked_sub(1)
        .and_then(|index| line_starts.get(index))
        .copied()
        .unwrap_or_default();

    (
        u32::try_from(line).unwrap_or(u32::MAX),
        offset.saturating_sub(line_start).saturating_add(1),
    )
}

pub(in crate::compiler) mod analysis;
mod calls;
mod closures;
mod constructs;
mod expressions;
mod flow;
mod matching;
mod places;
mod statements;

use crate::compiler::emit::analysis::collect_assigned_in_expression;
use crate::compiler::emit::analysis::collect_scoped_bindings_in_expression;
use crate::compiler::emit::analysis::collect_scoped_bindings_in_statements;
use crate::compiler::emit::analysis::collect_variables_in_expression;
use crate::compiler::emit::analysis::collect_variables_in_statements;
use crate::compiler::emit::analysis::references_this_in_block;
use crate::compiler::emit::constructs::tuple_index;
use crate::compiler::emit::expressions::operators::ShortCircuit;
use crate::compiler::emit::expressions::operators::binary_instruction;
use crate::compiler::emit::expressions::operators::compound_instruction;
use crate::compiler::emit::expressions::operators::short_circuit_jump;
use crate::compiler::emit::flow::Cleanup;
use crate::compiler::emit::flow::ControlFlow;
use crate::compiler::emit::flow::ControlFrame;
use crate::compiler::emit::flow::LoopFrame;
use crate::compiler::emit::flow::LoopJump;
use crate::compiler::emit::flow::pop_finally_holes;
use crate::compiler::emit::flow::pop_loop_frame;
use crate::compiler::emit::flow::scan_statements;
use crate::compiler::emit::flow::subtract_holes;
use crate::compiler::emit::places::ChainStep;
use crate::compiler::emit::places::Place;
use crate::compiler::emit::places::WriteTarget;

impl<'compilation, 'arena> BodyCompiler<'compilation, 'arena> {
    fn types<'scope>(&'scope self, scope: &'scope Scope<'compilation>) -> TypeScope<'scope> {
        TypeScope {
            heap: scope.heap,
            resolver: scope.resolver,
            class: scope.class,
            aliases: self.aliases,
            binders: &scope.binders,
            forbidden_binders: &scope.forbidden_binders,
            generics: scope.generics,
        }
    }

    pub(in crate::compiler) fn new(
        heap: &'compilation Heap,
        path: &'compilation str,
        runtime_path: &'compilation [u8],
        source_text: &'compilation str,
        synthesized: &'compilation mut Vec<CompiledFunction>,
        aliases: &'compilation [CompiledTypeAlias],
        shape: BodyShape,
    ) -> Self {
        Self {
            heap,
            path,
            runtime_path,
            chunk: Chunk::new(),
            registers: Registers::new(),
            locals: Vec::new(),
            local_index: HashMap::new(),
            constant_index: HashMap::new(),
            scoped_bindings: Vec::new(),
            using_cleanups: Vec::new(),
            branch_fusions: Vec::new(),
            loop_backedge_fusions: Vec::new(),
            flow: ControlFlow::default(),
            synthesized,
            source_text,
            aliases,
            shape,
        }
    }

    fn allocate(&mut self, span: Span) -> Result<Register, CompileError> {
        self.registers
            .allocate()
            .ok_or_else(|| register_limit(span))
    }

    /// Clears released compiler temporaries before ownership is observed.
    fn clear_released_temporaries(&mut self, span: Span) {
        for index in self.registers.mark()..self.registers.count() {
            self.chunk.emit(
                Instruction::Clear {
                    target: Register::new(index),
                },
                span,
            );
        }
    }

    /// Interns a constant, reporting a full pool against `span`. The by-key
    /// index makes emission-time interning constant-time; the pool itself
    /// never scans.
    fn add_constant(
        &mut self,
        literal: BytecodeLiteral,
        span: Span,
    ) -> Result<ConstantIndex, CompileError> {
        let key = literal_key(&literal);
        if let Some(index) = self.constant_index.get(&key) {
            return Ok(*index);
        }

        let index = self
            .chunk
            .push_constant(literal)
            .map_err(|full| side_table_limit(full, span))?;
        self.constant_index.insert(key, index);

        Ok(index)
    }

    fn add_type_descriptor(
        &mut self,
        descriptor: TypeDescriptor,
        span: Span,
    ) -> Result<DescriptorIndex, CompileError> {
        self.chunk
            .add_type_descriptor(descriptor)
            .map_err(|full| side_table_limit(full, span))
    }

    fn add_call_descriptor(
        &mut self,
        descriptor: CallDescriptor,
        span: Span,
    ) -> Result<CallDescriptorIndex, CompileError> {
        self.chunk
            .add_call_descriptor(descriptor)
            .map_err(|full| side_table_limit(full, span))
    }

    fn add_preset_descriptor(
        &mut self,
        descriptor: PresetDescriptor,
        span: Span,
    ) -> Result<PresetDescriptorIndex, CompileError> {
        self.chunk
            .add_preset_descriptor(descriptor)
            .map_err(|full| side_table_limit(full, span))
    }

    /// Allocates an inline-cache slot, reporting a full table against `span`.
    fn add_ic_descriptor(
        &mut self,
        descriptor: IcDescriptor,
        span: Span,
    ) -> Result<IcSlot, CompileError> {
        self.chunk
            .add_ic_descriptor(descriptor)
            .map_err(|full| side_table_limit(full, span))
    }

    pub(crate) fn declare_local(
        &mut self,
        name: &str,
        defined: bool,
        span: Span,
    ) -> Result<(), CompileError> {
        if self.local_index.contains_key(name) {
            return Ok(());
        }

        let register = self
            .registers
            .reserve_local()
            .ok_or_else(|| register_limit(span))?;
        self.push_local(Local {
            name: name.to_string(),
            register,
            defined,
            written: defined,
            final_span: None,
        });

        Ok(())
    }

    fn push_local(&mut self, local: Local) {
        self.local_index
            .entry(local.name.clone())
            .or_default()
            .push(self.locals.len());
        self.locals.push(local);
    }

    fn truncate_locals(&mut self, count: usize) {
        while self.locals.len() > count {
            let Some(local) = self.locals.pop() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("the truncated tail is non-empty") }
            };
            let Some(positions) = self.local_index.get_mut(&local.name) else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("every local is indexed by its name") }
            };
            positions.pop();
            if positions.is_empty() {
                self.local_index.remove(&local.name);
            }
        }
    }

    /// Reserves every local referenced or assigned anywhere in the
    /// statements, so no local appears after temporaries exist.
    pub(crate) fn declare_assigned_locals(
        &mut self,
        statements: &[Statement<'_>],
        span: Span,
    ) -> Result<(), CompileError> {
        let names = collect_variables_in_statements(statements);
        for name in names {
            if name != "$this" {
                self.declare_local(&name, false, span)?;
            }
        }

        let mut bindings = Vec::new();
        collect_scoped_bindings_in_statements(statements, &mut bindings);
        self.declare_scoped_bindings(bindings)?;

        Ok(())
    }

    /// Reserves original-value slots only for parameters the body can
    /// overwrite. Untouched parameters remain their own trace source.
    pub(crate) fn prepare_trace_arguments(
        &mut self,
        parameter_list: &ParameterList<'_>,
        assigned: &[String],
        span: Span,
    ) -> Result<(), CompileError> {
        let first = parameter_list
            .parameters
            .first()
            .map(|parameter| self.local_register(parameter.variable.name, span))
            .transpose()?
            .unwrap_or_else(|| Register::new(0));
        self.chunk.parameter_register_start = first.index();
        let Ok(parameter_count) = u16::try_from(parameter_list.parameters.len()) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the parameter-count gate bounds the trace window") }
        };
        self.chunk.parameter_register_count = parameter_count;
        if !parameter_list
            .parameters
            .iter()
            .any(|parameter| assigned.iter().any(|name| name == parameter.variable.name))
        {
            return Ok(());
        }
        self.chunk
            .trace_argument_registers
            .reserve(parameter_list.parameters.len());

        for parameter in &parameter_list.parameters {
            if assigned.iter().any(|name| name == parameter.variable.name) {
                let register = self
                    .registers
                    .reserve_local()
                    .ok_or_else(|| register_limit(parameter.span()))?;
                self.chunk.trace_argument_registers.push(register);
            } else {
                self.chunk.trace_argument_registers.push(Register::NONE);
            }
        }

        Ok(())
    }

    fn trace_argument_for(&self, parameter: Register) -> Option<Register> {
        let position = parameter
            .index()
            .checked_sub(self.chunk.parameter_register_start)?;
        if position >= self.chunk.parameter_register_count {
            return None;
        }

        self.chunk
            .trace_argument_registers
            .get(usize::from(position))
            .copied()
            .filter(|register| *register != Register::NONE)
    }

    pub(crate) fn declare_referenced_locals(
        &mut self,
        expression: &Expression<'_>,
    ) -> Result<(), CompileError> {
        let names = collect_variables_in_expression(expression);
        for name in names {
            if name != "$this" {
                self.declare_local(&name, false, expression.span())?;
            }
        }

        let mut bindings = Vec::new();
        collect_scoped_bindings_in_expression(expression, &mut bindings);
        self.declare_scoped_bindings(bindings)?;

        Ok(())
    }

    /// Reserves the registers used by lexical match-pattern bindings before any
    /// temporary exists.
    pub(crate) fn declare_scoped_bindings(
        &mut self,
        bindings: Vec<(String, Span)>,
    ) -> Result<(), CompileError> {
        for (_, span) in bindings {
            let register = self
                .registers
                .reserve_local()
                .ok_or_else(|| register_limit(span))?;
            self.scoped_bindings.push(ScopedBinding { span, register });
        }

        Ok(())
    }

    pub(in crate::compiler) fn statement_public(
        &mut self,
        scope: &Scope<'_>,
        statement: &Statement<'arena>,
    ) -> Result<(), CompileError> {
        self.statement(scope, statement)
    }

    /// The return instruction for a value: unchecked when the body belongs to
    /// trusted code whose written return type is a reviewed guarantee.
    pub(crate) const fn return_instruction(&self, source: Register) -> Instruction {
        if self.shape.trusted_returns {
            Instruction::ReturnUnchecked { source }
        } else {
            Instruction::Return { source }
        }
    }

    pub(crate) fn finish_initializer(mut self, register: Register, span: Span) -> Chunk {
        self.chunk
            .emit(Instruction::Return { source: register }, span);
        self.fuse_recorded_control_flow();
        self.record_uninitialized_registers();
        self.chunk.local_register_count = self.registers.local_count();
        self.chunk.register_count = self.registers.count();
        self.chunk.refresh_runtime_metadata();
        self.chunk
    }

    /// Emits defaults, type checks, and promoted property writes.
    pub(in crate::compiler) fn parameter_prologue(
        &mut self,
        scope: &Scope<'_>,
        parameter_list: &ParameterList<'_>,
    ) -> Result<(), CompileError> {
        for parameter in &parameter_list.parameters {
            let Some(default) = &parameter.default else {
                continue;
            };

            let target = self.local_register(parameter.variable.name, parameter.span())?;
            let fill = self.chunk.emit(
                Instruction::FillDefault {
                    target,
                    offset: JumpOffset::new(0),
                },
                parameter.span(),
            );

            let mark = self.registers.mark();
            let value = self.expression(scope, default.value)?;
            self.move_into(target, value, default.value.span());
            self.registers.release_to(mark);
            self.chunk.patch_jump(fill, self.code_position());
        }

        for parameter in &parameter_list.parameters {
            if parameter.default.is_none() {
                continue;
            }

            let Some(annotation) = parameter.r#type else {
                continue;
            };

            let descriptor = lower_type(&self.types(scope), annotation)?;
            let descriptor = self.add_type_descriptor(descriptor, annotation.span())?;
            let target = self.local_register(parameter.variable.name, parameter.span())?;
            self.chunk.emit(
                Instruction::AsCheck {
                    destination: target,
                    source: target,
                    descriptor,
                    mode: AsMode::Boundary,
                },
                parameter.span(),
            );
        }

        if self.shape.promote_parameters {
            for parameter in &parameter_list.parameters {
                if !parameter.is_promoted_property() {
                    continue;
                }

                let cache = self.add_ic_descriptor(
                    IcDescriptor::Member {
                        name: self.heap.intern(
                            parameter
                                .variable
                                .name
                                .strip_prefix('$')
                                .unwrap_or(parameter.variable.name)
                                .as_bytes(),
                        ),
                        type_arguments: None,
                    },
                    parameter.span(),
                )?;

                let value = self.local_register(parameter.variable.name, parameter.span())?;
                self.chunk.emit(
                    Instruction::PropertyInitRaw {
                        object: Register::new(0),
                        value,
                        cache,
                    },
                    parameter.span(),
                );
            }
        }

        Ok(())
    }

    pub(in crate::compiler) fn statements(
        &mut self,
        scope: &Scope<'_>,
        statements: &[Statement<'arena>],
    ) -> Result<(), CompileError> {
        self.statements_inner(scope, statements)
    }

    fn statements_inner<'source>(
        &mut self,
        scope: &Scope<'_>,
        statements: &'source [Statement<'arena>],
    ) -> Result<(), CompileError> {
        for statement in statements {
            self.statement(scope, statement)?;
        }

        Ok(())
    }

    fn code_position(&self) -> u32 {
        let Ok(position) = u32::try_from(self.chunk.code.len()) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("a chunk stays within the thirty-two-bit index space") }
        };

        position
    }

    fn emit_loop_backedge(&mut self, target: u32, span: Span) {
        let candidate = self.chunk.code.len().checked_sub(1).and_then(|index| {
            self.chunk
                .code
                .get(index)
                .copied()
                .map(|instruction| (index, instruction))
        });

        let jump = self.chunk.emit(
            Instruction::Jump {
                offset: JumpOffset::new(0),
            },
            span,
        );
        self.chunk.patch_jump(jump, target);
        if let Some((
            index,
            Instruction::AddImmediate {
                destination,
                source,
                ..
            },
        )) = candidate
            && destination == source
            && i16::try_from(i64::from(target) - wide_code_position(index)).is_ok()
        {
            self.loop_backedge_fusions.push(index);
        }
    }

    pub(crate) fn finish(mut self, end: Span) -> Chunk {
        self.chunk.emit(Instruction::ReturnNull, end);
        self.fuse_recorded_control_flow();
        self.record_uninitialized_registers();
        self.chunk.local_register_count = self.registers.local_count();
        self.chunk.register_count = self.registers.count();
        self.chunk.refresh_runtime_metadata();
        self.chunk
    }

    /// Records the registers read before their first write.
    fn record_uninitialized_registers(&mut self) {
        self.chunk
            .uninitialized_registers
            .extend(
                self.chunk
                    .code
                    .iter()
                    .filter_map(|instruction| match instruction {
                        Instruction::CheckDefined { subject, .. } => Some(*subject),
                        _ => None,
                    }),
            );
        self.chunk
            .uninitialized_registers
            .sort_unstable_by_key(|register| register.index());
        self.chunk.uninitialized_registers.dedup();
    }

    fn record_branch_fusion(&mut self, first: usize, condition: Register, jump: u32) {
        let Some(producer) = usize::try_from(jump)
            .ok()
            .and_then(|jump| jump.checked_sub(1))
        else {
            return;
        };
        if producer < first {
            return;
        }
        let (Instruction::Equal { destination, .. }
        | Instruction::NotEqual { destination, .. }
        | Instruction::LessThan { destination, .. }
        | Instruction::LessThanOrEqual { destination, .. }
        | Instruction::GreaterThan { destination, .. }
        | Instruction::GreaterThanOrEqual { destination, .. }
        | Instruction::Is { destination, .. }) = self.chunk.code[producer]
        else {
            return;
        };
        if destination == condition {
            self.branch_fusions.push(producer);
        }
    }

    fn fuse_recorded_control_flow(&mut self) {
        if self.branch_fusions.is_empty() && self.loop_backedge_fusions.is_empty() {
            return;
        }

        let targets = control_flow_targets(&self.chunk);
        let mut remove = vec![false; self.chunk.code.len()];
        self.fuse_recorded_branches(&targets, &mut remove);
        self.fuse_recorded_loop_backedges(&targets, &mut remove);
        if remove.iter().any(|removed| *removed) {
            compact(&mut self.chunk, &remove);
        }
    }

    fn fuse_recorded_branches(&mut self, targets: &HashSet<usize>, remove: &mut [bool]) {
        for producer in mem::take(&mut self.branch_fusions) {
            let branch = producer + 1;
            if targets.contains(&branch) {
                continue;
            }
            let Some(replacement) = self.branch_replacement(producer) else {
                continue;
            };
            self.chunk.code[producer] = replacement;
            remove[branch] = true;
        }
    }

    fn branch_replacement(&self, producer: usize) -> Option<Instruction> {
        let branch = producer + 1;
        let (condition, offset, when_true) = match self.chunk.code[branch] {
            Instruction::JumpIfFalse { condition, offset } => (condition, offset, false),
            Instruction::JumpIfTrue { condition, offset } => (condition, offset, true),
            _ => return None,
        };
        let target = wide_code_position(branch) + i64::from(offset.offset());
        let relative = i16::try_from(target - wide_code_position(producer)).ok()?;
        let offset = ShortJumpOffset::new(relative);
        let jump_unless = |comparison, left, right| {
            Some(Instruction::JumpUnless {
                comparison,
                left,
                right,
                offset,
            })
        };

        match self.chunk.code[producer] {
            Instruction::Equal {
                destination,
                left,
                right,
            } if destination == condition => jump_unless(
                if when_true {
                    Comparison::NotEqual
                } else {
                    Comparison::Equal
                },
                left,
                right,
            ),
            Instruction::NotEqual {
                destination,
                left,
                right,
            } if destination == condition => jump_unless(
                if when_true {
                    Comparison::Equal
                } else {
                    Comparison::NotEqual
                },
                left,
                right,
            ),
            Instruction::LessThan {
                destination,
                left,
                right,
            } if destination == condition && !when_true => {
                jump_unless(Comparison::LessThan, left, right)
            }
            Instruction::LessThanOrEqual {
                destination,
                left,
                right,
            } if destination == condition && !when_true => {
                jump_unless(Comparison::LessThanOrEqual, left, right)
            }
            Instruction::GreaterThan {
                destination,
                left,
                right,
            } if destination == condition && !when_true => {
                jump_unless(Comparison::GreaterThan, left, right)
            }
            Instruction::GreaterThanOrEqual {
                destination,
                left,
                right,
            } if destination == condition && !when_true => {
                jump_unless(Comparison::GreaterThanOrEqual, left, right)
            }
            Instruction::Is {
                destination,
                source,
                descriptor,
            } if destination == condition
                && matches!(
                    self.chunk.type_descriptors[usize::from(descriptor.index())],
                    TypeDescriptor::IntRange { .. }
                ) =>
            {
                Some(if when_true {
                    Instruction::IntRangeJumpIf {
                        subject: source,
                        descriptor,
                        offset,
                    }
                } else {
                    Instruction::IntRangeJumpUnless {
                        subject: source,
                        descriptor,
                        offset,
                    }
                })
            }
            _ => None,
        }
    }

    fn fuse_recorded_loop_backedges(&mut self, targets: &HashSet<usize>, remove: &mut [bool]) {
        for increment in mem::take(&mut self.loop_backedge_fusions) {
            let jump = increment + 1;
            if targets.contains(&jump) || remove[increment] || remove[jump] {
                continue;
            }
            let Instruction::AddImmediate {
                destination,
                source,
                immediate,
            } = self.chunk.code[increment]
            else {
                continue;
            };
            let Instruction::Jump { offset } = self.chunk.code[jump] else {
                continue;
            };
            if destination != source {
                continue;
            }
            let target = wide_code_position(jump) + i64::from(offset.offset());
            let Ok(relative) = i16::try_from(target - wide_code_position(increment)) else {
                continue;
            };

            self.chunk.code[increment] = Instruction::IncrementJump {
                target: destination,
                immediate,
                offset: ShortJumpOffset::new(relative),
            };
            remove[jump] = true;
        }
    }

    fn read_local(&mut self, name: &str, span: Span) -> Result<Register, CompileError> {
        if name == "$this" {
            if !self.shape.is_instance_method {
                return Err(CompileError::new(
                    CompileErrorKind::ThisOutsideMethod,
                    "`$this` is available only inside a class method",
                    span,
                ));
            }

            return Ok(Register::new(0));
        }

        let position = self.local_position(name);
        let Some(position) = position else {
            self.declare_local(name, false, span)?;
            return self.read_local(name, span);
        };

        if !self.locals[position].defined {
            let constant = self.string_constant(name.as_bytes(), span)?;
            self.chunk.emit(
                Instruction::CheckDefined {
                    subject: self.locals[position].register,
                    name: constant,
                },
                span,
            );

            self.locals[position].defined = true;
        }

        Ok(self.locals[position].register)
    }

    fn local_position(&self, name: &str) -> Option<usize> {
        self.local_index
            .get(name)
            .and_then(|positions| positions.last().copied())
    }

    fn push_match_variable_binding(&mut self, variable: &Variable<'_>) {
        let Some(binding) = self
            .scoped_bindings
            .iter()
            .find(|binding| binding.span == variable.span())
        else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe {
                unreachable_invariant("every match binding register is reserved before emission")
            }
        };

        self.push_local(Local {
            name: variable.name.to_string(),
            register: binding.register,
            defined: false,
            written: false,
            final_span: None,
        });
    }

    fn local_register(&mut self, name: &str, span: Span) -> Result<Register, CompileError> {
        if let Some(position) = self.local_position(name) {
            return Ok(self.locals[position].register);
        }

        self.declare_local(name, false, span)?;
        let Some(local) = self.locals.last() else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("the local was just declared") }
        };

        Ok(local.register)
    }

    fn mark_defined(&mut self, name: &str) {
        if let Some(position) = self.local_position(name) {
            self.locals[position].defined = true;
            self.locals[position].written = true;
        }
    }

    fn ensure_local_writable(&self, name: &str, span: Span) -> Result<(), CompileError> {
        let Some(final_span) = self
            .local_position(name)
            .and_then(|position| self.locals[position].final_span)
        else {
            return Ok(());
        };

        Err(CompileError::new(
            CompileErrorKind::CannotAssignFinalLocal,
            format!("cannot assign to the final binding `{name}`"),
            span,
        )
        .with_note(final_span, format!("`{name}` is bound once here")))
    }

    fn mark_local_final(&mut self, name: &str, span: Span) {
        if let Some(position) = self.local_position(name) {
            self.locals[position].final_span = Some(span);
        }
    }

    fn final_local_span(&self, name: &str) -> Option<Span> {
        self.local_position(name)
            .and_then(|position| self.locals[position].final_span)
    }

    /// Marks a local unavailable after its binding has been explicitly dropped.
    fn mark_undefined(&mut self, name: &str) {
        if let Some(position) = self.local_position(name) {
            self.locals[position].defined = false;
        }
    }

    /// Saves the definite-assignment state before a conditional region.
    fn save_defined(&self) -> Vec<bool> {
        self.locals.iter().map(|local| local.defined).collect()
    }

    /// Restores the definite-assignment state after a conditional region.
    fn restore_defined(&mut self, saved: Vec<bool>) {
        for (local, defined) in self.locals.iter_mut().zip(saved) {
            local.defined = defined;
        }
    }

    /// Keeps only the assignments a joining path also made: a variable is
    /// definitely assigned after a join when every path into it assigned it.
    fn intersect_defined(&mut self, other: &[bool]) {
        for (local, defined) in self.locals.iter_mut().zip(other) {
            local.defined = local.defined && *defined;
        }
    }

    /// Emits `Move destination, source` unless they already coincide.
    fn move_into(&mut self, destination: Register, source: Register, span: Span) {
        if destination != source {
            self.chunk.emit(
                Instruction::Move {
                    destination,
                    source,
                },
                span,
            );
        }
    }

    fn move_argument_into(
        &mut self,
        destination: Register,
        source: Register,
        inner: u16,
        span: Span,
    ) {
        if destination != source && source.index() >= inner {
            self.chunk.emit(
                Instruction::MoveOwned {
                    destination,
                    source,
                },
                span,
            );
        } else {
            self.move_into(destination, source, span);
        }
    }

    fn window<'expression>(
        &mut self,
        scope: &Scope<'_>,
        values: impl Iterator<Item = &'expression Expression<'expression>>,
        count: usize,
        span: Span,
    ) -> Result<Register, CompileError> {
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            slots.push(self.allocate(span)?);
        }

        let first = slots
            .first()
            .copied()
            .unwrap_or_else(|| Register::new(self.registers.mark()));
        for (slot, value) in slots.iter().zip(values) {
            let inner = self.registers.mark();
            let register = self.expression(scope, value)?;
            self.move_argument_into(*slot, register, inner, value.span());
            self.registers.release_to(inner);
        }

        let _ = span;
        Ok(first)
    }
}

#[cfg(test)]
mod tests {
    use super::line_and_column;

    #[test]
    fn indexed_source_positions_are_one_based() {
        let line_starts = [0, 6, 7];

        assert_eq!(line_and_column(&line_starts, 0), (1, 1));
        assert_eq!(line_and_column(&line_starts, 4), (1, 5));
        assert_eq!(line_and_column(&line_starts, 6), (2, 1));
        assert_eq!(line_and_column(&line_starts, 7), (3, 1));
        assert_eq!(line_and_column(&line_starts, 10), (3, 4));
    }
}
