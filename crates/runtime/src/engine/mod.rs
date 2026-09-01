//! The engine: the heap, the registries, the output streams, and the run.

#![deny(clippy::nursery, clippy::pedantic)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "engine state is shared across sibling runtime modules"
)]

use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::io;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;

use hashbrown::HashMap;
use hashbrown::HashSet;

use whim_loop::Scheduler;
use whim_loop::Stack;
use whim_loop::TaskId;
use whim_span::HasSpan;
use whim_syn::arena::LocalArena;
use whim_syn::cst::Program;
use whim_syn::parser;

use crate::blocking::BlockingPool;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::literal_value;
use crate::bytecode::verify::verify_unit;
use crate::classes::MethodBodyKind;
use crate::classes::is_instance_of;
use crate::compiler::CompileConfiguration;
use crate::compiler::CompileError;
use crate::compiler::compile_with_path_bytes_configuration_and_built_in_functions;
use crate::core::async_::task_local::TaskLocalValues;
use crate::core::async_::task_local::new_task_local_values;
use crate::core::classes::ERROR_SLOT_CODE;
use crate::core::classes::ERROR_SLOT_MESSAGE;
use crate::core::classes::ERROR_SLOT_PREVIOUS;
use crate::core::classes::ERROR_SLOT_TRACE;
use crate::core::classes::TRACE_FRAME_SLOT_ARGUMENTS;
use crate::core::classes::TRACE_FRAME_SLOT_FILE;
use crate::core::classes::TRACE_FRAME_SLOT_FUNCTION;
use crate::core::classes::TRACE_FRAME_SLOT_LINE;
use crate::core::coroutine::CoroutineObject;
use crate::core::private::syscall::StandardStream;
use crate::engine::declare::CachedUnit;
use crate::engine::diagnostics::DiagnosticLabel;
use crate::engine::diagnostics::DiagnosticLabels;
use crate::engine::diagnostics::DiagnosticOrigin;
use crate::engine::diagnostics::ExceptionDiagnostic;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::WorldCache;
use crate::optimizer::optimize_unit;
use crate::optimizer::optimize_unit_entry;
use crate::symbols::FunctionLocator;
use crate::symbols::FunctionTable;
use crate::symbols::InlineCache;
use crate::symbols::SourceText;
use crate::symbols::SymbolKind;
use crate::symbols::UnitContext;
use crate::symbols::line_starts_of;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::atom::Atom;
use crate::value::dict::keys::KeyRef;
use crate::value::function::FuncId;
use crate::value::heap::Heap;
use crate::value::heap::handle::ManagedRef;
use crate::value::object::ClassId;
use crate::value::object::InstanceObject;
use crate::value::vec::VecObject;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::errors::debug_render;

/// The tunables of one engine.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfiguration {
    /// The greatest number of frames an execution may stack.
    pub call_depth_limit: usize,
    /// The cycle collector's root-buffer threshold; `None` keeps the heap's
    /// default.
    pub cycle_threshold: Option<usize>,
    /// Whether compiled bytecode is optimized before it is verified and run.
    pub optimize: bool,
    /// Whether trace-boundary frames remain visible in captured traces.
    pub full_trace: bool,
    /// Whether the engine's error stream is colored.
    pub diagnostic_color: bool,
}

impl Default for EngineConfiguration {
    fn default() -> Self {
        Self {
            call_depth_limit: 10_000,
            cycle_threshold: None,
            optimize: true,
            full_trace: false,
            diagnostic_color: false,
        }
    }
}

/// How an execution ended.
pub struct ExecutionOutcome {
    exit_code: u8,
    error: Option<EngineError>,
}

impl fmt::Debug for ExecutionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(error) = &self.error {
            return formatter
                .debug_tuple("Uncaught")
                .field(&error.rendered())
                .finish();
        }

        formatter
            .debug_struct("ExecutionOutcome")
            .field("exit_code", &self.exit_code)
            .finish()
    }
}

/// An error value carried out of the engine.
pub struct EngineError {
    _value: Option<Value>,
    rendered: String,
    _heap: Rc<Heap>,
}

impl EngineError {
    #[must_use]
    pub(crate) fn rendered(&self) -> &str {
        &self.rendered
    }
}

impl fmt::Debug for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("EngineError")
            .field(&self.rendered)
            .finish()
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.rendered)
    }
}

impl Error for EngineError {}

impl ExecutionOutcome {
    pub(crate) const fn completed() -> Self {
        Self {
            exit_code: 0,
            error: None,
        }
    }

    pub(crate) const fn exited(code: u8) -> Self {
        Self {
            exit_code: code,
            error: None,
        }
    }

    pub(crate) const fn uncaught(error: EngineError) -> Self {
        Self {
            exit_code: 255,
            error: Some(error),
        }
    }

    /// The process exit code the outcome maps to.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

/// The engine. See the [module documentation](self).
pub struct Engine {
    pub(crate) blocking: BlockingPool,
    pub(crate) configuration: EngineConfiguration,
    pub(crate) tables: tables::RuntimeTables,
    pub(crate) units: Vec<Rc<UnitContext>>,
    pub(crate) optimizer_world: Option<WorldCache>,
    pub(crate) optimizer_class_contexts: HashMap<Atom, CompiledClassLike>,
    pub(crate) unit_generic_validation: Vec<UnitGenericValidation>,
    pub(crate) generic_validation_journal: Vec<GenericValidationJournalEntry>,
    pub(crate) sources: HashMap<Atom, SourceText>,
    pub(crate) exception_diagnostics: HashMap<usize, ExceptionDiagnostic>,
    pub(crate) declaration_depth: usize,
    pub(crate) unit_cache: HashMap<PathBuf, CachedUnit>,
    pub(crate) loaded_paths: HashSet<PathBuf>,
    pub(crate) autoloader: Option<Value>,
    pub(crate) autoload_in_flight: HashSet<(SymbolKind, Atom)>,
    pub(crate) coroutine_stack: Vec<Rc<CoroutineObject>>,
    pub(crate) coroutine_stack_pool: Vec<Stack>,
    pub(crate) scheduler: Option<Scheduler<Rc<CoroutineObject>, Value>>,
    pub(crate) main_task_local_values: TaskLocalValues,
    pub(crate) next_task_local_id: u64,
    pub(crate) finalizer_tasks: HashSet<TaskId>,
    pub(crate) cancelled_tasks: HashSet<TaskId>,
    pub(crate) arguments: Vec<Vec<u8>>,
    pub(crate) script: Option<Vec<u8>>,
    pub(crate) output_failure: Option<io::Error>,
    pub(crate) heap: Rc<Heap>,
}

#[derive(Clone, Copy)]
pub(crate) enum GenericValidationJournalEntry {
    TypeArgumentBounds(usize),
    TypeParameterDefaults(usize),
}

#[derive(Default)]
pub(crate) struct UnitGenericValidation {
    pub(crate) type_argument_bounds: bool,
    pub(crate) type_parameter_defaults: bool,
}

mod artifact;
pub(crate) mod attributes;
pub(crate) mod builtins;
pub(crate) mod declare;
pub(crate) mod diagnostics;
mod optimize;
mod tables;

use crate::engine::builtins::runtime_function;
use crate::engine::builtins::text_of;
use crate::linker::descriptors::descriptor_from_built_in_spec;

impl Drop for Engine {
    fn drop(&mut self) {
        self.main_task_local_values.clear();
        if !self.tables.classes.is_empty() {
            let mut vm = VirtualMachine::new(self);
            let shutdown = vm.run_shutdown_finalizers();
            drop(vm);
            match shutdown {
                Err(VirtualMachineControl::Throw(value)) => drop(self.uncaught(value)),
                Err(VirtualMachineControl::Exit(_)) | Ok(()) => {}
            }
        }
        self.tables.constants.clear();
        self.tables.classes.clear();
        self.tables.functions.clear();
        self.tables.newtypes.clear();
        self.tables.newtype_values.clear();
        self.tables.newtype_value_cache.clear();
        self.optimizer_world = None;
        self.optimizer_class_contexts.clear();
        self.units.clear();
        self.sources.clear();
        self.exception_diagnostics.clear();
        self.autoloader = None;
        self.coroutine_stack.clear();
        self.coroutine_stack_pool.clear();
        self.finalizer_tasks.clear();
        self.cancelled_tasks.clear();
        self.scheduler = None;
    }
}

impl Engine {
    /// An engine writing to the process's standard streams, buffered.
    #[must_use]
    pub fn new(configuration: EngineConfiguration) -> Self {
        let heap = Heap::new();
        heap.configure_cycle_threshold(configuration.cycle_threshold);

        let tables = tables::RuntimeTables::new(&heap);

        Self {
            blocking: BlockingPool::new(),
            configuration,
            tables,
            units: Vec::new(),
            optimizer_world: None,
            optimizer_class_contexts: HashMap::new(),
            unit_generic_validation: Vec::new(),
            generic_validation_journal: Vec::new(),
            sources: HashMap::new(),
            exception_diagnostics: HashMap::new(),
            declaration_depth: 0,
            unit_cache: HashMap::new(),
            loaded_paths: HashSet::new(),
            autoloader: None,
            autoload_in_flight: HashSet::new(),
            coroutine_stack: Vec::new(),
            coroutine_stack_pool: Vec::new(),
            scheduler: None,
            main_task_local_values: new_task_local_values(),
            next_task_local_id: 0,
            finalizer_tasks: HashSet::new(),
            cancelled_tasks: HashSet::new(),
            arguments: Vec::new(),
            script: None,
            output_failure: None,
            heap,
        }
    }

    /// Replaces the settings used by later compilation and execution.
    pub fn configure(&mut self, configuration: EngineConfiguration) {
        self.heap
            .configure_cycle_threshold(configuration.cycle_threshold);

        self.configuration = configuration;
    }

    #[must_use]
    pub(crate) fn is_throwable_instance(&self, class: ClassId) -> bool {
        is_instance_of(
            &self.tables.classes,
            class,
            self.tables.well_known.throwable,
        )
    }

    /// Parses, compiles, declares, and executes a source text at `path`.
    pub fn run_source(&mut self, source: &str, path: &Path) -> ExecutionOutcome {
        let arena = LocalArena::new();
        let diagnostic_path = path.to_string_lossy();
        let runtime_path = path.as_os_str().as_bytes();
        let path_atom = self.heap.intern(runtime_path);
        let retained_source = Rc::<str>::from(source);
        let program = match parser::parse(&arena, source) {
            Ok(program) => program,
            Err(errors) => {
                let origin = DiagnosticOrigin {
                    path: path_atom.clone(),
                    source: retained_source,
                    labels: DiagnosticLabels::Single(DiagnosticLabel {
                        span: errors.span(),
                        message: errors.to_string(),
                    }),
                };

                let value = self.declaration_error_with_origin(
                    self.tables.well_known.parser_error,
                    errors.to_string(),
                    &path_atom,
                    origin,
                );

                return self.uncaught(value);
            }
        };

        let unit = match self.compile_program(program, &diagnostic_path, runtime_path) {
            Ok(unit) => unit,
            Err(error) => {
                let origin = DiagnosticOrigin {
                    path: path_atom.clone(),
                    source: retained_source,
                    labels: DiagnosticLabels::Multiple(
                        error
                            .labels()
                            .into_iter()
                            .map(|(span, message)| DiagnosticLabel {
                                span,
                                message: message.to_string(),
                            })
                            .collect(),
                    ),
                };

                let value = self.declaration_error_with_origin(
                    self.tables.well_known.compiler_error,
                    error.message,
                    &path_atom,
                    origin,
                );

                return self.uncaught(value);
            }
        };

        let unit = Rc::new(unit);
        if let Err(error) = verify_unit(&unit) {
            let value = self.declaration_error(
                self.tables.well_known.compiler_error,
                format!("the compiled unit did not verify: {error:?}"),
                &path_atom,
            );

            return self.uncaught(value);
        }

        self.run_declared(&unit, line_starts_of(source), Some(retained_source))
    }

    pub(crate) fn compile_program(
        &self,
        program: &Program<'_>,
        diagnostic_path: &str,
        runtime_path: &[u8],
    ) -> Result<CompiledUnit, CompileError> {
        let mut unit = compile_with_path_bytes_configuration_and_built_in_functions(
            program,
            diagnostic_path,
            runtime_path,
            &self.heap,
            CompileConfiguration {
                optimization: OptimizationConfiguration {
                    enabled: self.compiler_optimizes_units(),
                    ..OptimizationConfiguration::default()
                },
                trusted_return_types: false,
            },
            &self.tables.built_in_function_declarations,
        )?;

        self.expand_declared_types(&mut unit);

        Ok(self.optimize_unit_against_world(unit))
    }

    #[cfg(test)]
    pub(crate) fn run_unit(&mut self, unit: &Rc<CompiledUnit>) -> ExecutionOutcome {
        if let Err(error) = verify_unit(unit) {
            let value = self.declaration_error(
                self.tables.well_known.compiler_error,
                format!("the compiled unit failed verification: {error:?}"),
                &unit.path,
            );

            return self.uncaught(value);
        }

        self.run_declared(unit, Vec::new(), None)
    }

    fn uncaught(&mut self, value: Value) -> ExecutionOutcome {
        let rendered = self.render_error_with_color(&value, self.configuration.diagnostic_color);
        let mut bytes = Vec::with_capacity(rendered.len() + 10);
        bytes.extend_from_slice(b"uncaught ");
        bytes.extend_from_slice(rendered.as_bytes());
        bytes.push(b'\n');
        let result = Self::write_standard_stream(StandardStream::Error, &bytes);
        self.note_output_failure(result);
        ExecutionOutcome::uncaught(self.engine_error(value))
    }

    pub(crate) fn write_panic(&mut self, message: &[u8], trace: &Value) {
        let color = self.configuration.diagnostic_color;
        let prefix = Self::diagnostic_paint("1;31", "panic", color);
        let mut suffix = String::new();
        self.render_trace(trace, color, &mut suffix);
        suffix.push('\n');

        let mut bytes = Vec::with_capacity(prefix.len() + message.len() + suffix.len() + 2);
        bytes.extend_from_slice(prefix.as_bytes());
        bytes.extend_from_slice(b": ");
        bytes.extend_from_slice(message);
        bytes.extend_from_slice(suffix.as_bytes());

        let result = Self::write_standard_stream(StandardStream::Error, &bytes);
        self.note_output_failure(result);
    }

    pub(crate) fn engine_error(&self, value: Value) -> EngineError {
        EngineError {
            rendered: self.render_error(&value),
            _value: Some(value),
            _heap: Rc::clone(&self.heap),
        }
    }

    pub(crate) fn engine_error_message(&self, message: String) -> EngineError {
        EngineError {
            rendered: message,
            _value: None,
            _heap: Rc::clone(&self.heap),
        }
    }

    /// Whether the compiler optimizes a unit on its own. A unit the engine
    /// will optimize against the loaded world is compiled unoptimized: every
    /// unit is optimized exactly once.
    pub(crate) const fn compiler_optimizes_units(&self) -> bool {
        self.configuration.optimize && !self.world_optimizes_units()
    }

    const fn world_optimizes_units(&self) -> bool {
        self.configuration.optimize && !self.units.is_empty()
    }

    /// Optimizes a freshly compiled unit against every declaration already
    /// loaded, once. The unit is not yet declared: nothing points into it, so
    /// every chunk may be rewritten freely.
    pub(crate) fn optimize_unit_against_world(&self, mut unit: CompiledUnit) -> CompiledUnit {
        if !self.world_optimizes_units() {
            return unit;
        }

        let world: Vec<&CompiledUnit> = self
            .units
            .iter()
            .map(|context| context.unit.as_ref())
            .collect();

        optimize_unit(
            &mut unit,
            &world,
            &self.tables.built_in_function_declarations,
            &self.heap,
            OptimizationConfiguration::default(),
        );

        unit
    }

    pub(crate) fn optimize_required_unit_against_world(
        &self,
        mut unit: CompiledUnit,
    ) -> (CompiledUnit, bool) {
        if !self.world_optimizes_units() {
            return (unit, false);
        }

        let world: Vec<&CompiledUnit> = self
            .units
            .iter()
            .map(|context| context.unit.as_ref())
            .collect();

        optimize_unit_entry(
            &mut unit,
            &world,
            &self.tables.built_in_function_declarations,
            &self.heap,
            OptimizationConfiguration::default(),
        );

        (unit, true)
    }

    pub(crate) fn note_output_failure(&mut self, result: io::Result<()>) {
        if let Err(error) = result
            && self.output_failure.is_none()
        {
            self.output_failure = Some(error);
        }
    }

    pub(crate) fn write_standard_stream(stream: StandardStream, bytes: &[u8]) -> io::Result<()> {
        let file = stream.file();
        let mut remaining = bytes;
        loop {
            while !remaining.is_empty() {
                // SAFETY: `file` is a live C stream and `remaining` lives through the call.
                let count = unsafe {
                    libc::fwrite(
                        remaining.as_ptr().cast::<libc::c_void>(),
                        1,
                        remaining.len(),
                        file,
                    )
                };

                // SAFETY: `file` is a live C stream.
                if unsafe { libc::ferror(file) } != 0 {
                    let error = io::Error::last_os_error();
                    // SAFETY: `file` is a live C stream.
                    unsafe { libc::clearerr(file) };
                    match error.kind() {
                        ErrorKind::Interrupted => {}
                        ErrorKind::WouldBlock => Self::wait_stream_writable(stream)?,
                        _ => return Err(error),
                    }
                }

                remaining = &remaining[count..];
            }

            // SAFETY: `file` is a live C stream.
            if unsafe { libc::fflush(file) } == 0 {
                return Ok(());
            }

            let error = io::Error::last_os_error();
            // SAFETY: `file` is a live C stream.
            unsafe { libc::clearerr(file) };
            match error.kind() {
                ErrorKind::Interrupted => {}
                ErrorKind::WouldBlock => Self::wait_stream_writable(stream)?,
                _ => return Err(error),
            }
        }
    }

    fn wait_stream_writable(stream: StandardStream) -> io::Result<()> {
        let mut request = libc::pollfd {
            fd: stream.number(),
            events: libc::POLLOUT,
            revents: 0,
        };

        loop {
            // SAFETY: `request` is a live poll record for this call.
            if unsafe { libc::poll(std::ptr::addr_of_mut!(request), 1, -1) } >= 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    /// Sets the program arguments as platform bytes without requiring UTF-8.
    pub fn set_argument_bytes(&mut self, arguments: Vec<Vec<u8>>) {
        self.arguments = arguments;
    }

    /// Sets the script path as platform path bytes, without requiring UTF-8.
    /// Bytes returned by [`crate::path::path_bytes`] round-trip through
    /// `Whim\Env\current_script` exactly.
    pub fn set_script_bytes(&mut self, script: Option<Vec<u8>>) {
        self.script = script;
    }

    /// Takes the first output failure the engine met, if any.
    #[must_use]
    pub const fn take_output_failure(&mut self) -> Option<io::Error> {
        self.output_failure.take()
    }

    /// Renders an error instance for `toString`: class,
    /// message, code, source origin when retained, trace frames, and previous
    /// chain. This surface is always plain text; only the engine's actual
    /// interactive error stream receives color.
    #[must_use]
    pub(crate) fn render_error(&self, value: &Value) -> String {
        self.render_error_with_color(value, false)
    }

    fn render_error_with_color(&self, value: &Value, color: bool) -> String {
        let Some(instance) = value.as_object() else {
            return format!("{} value", value.kind_name());
        };

        if !self.is_throwable_instance(instance.class()) {
            return format!("{} value", value.kind_name());
        }

        let mut seen = HashSet::new();
        self.render_throwable(instance, &mut seen, color)
    }

    fn render_trace_argument(&self, value: &Value, depth: u32) -> String {
        if depth > 4 {
            return "...".to_string();
        }

        if let Some(id) = value.newtype_id() {
            let backing = value.clone_with_newtype(self.tables.newtype_value(id).parent);
            return self.render_trace_argument(&backing, depth);
        }

        match value.transparent() {
            ValueView::Object(instance) => String::from_utf8_lossy(
                self.tables.classes[instance.class().0 as usize]
                    .name
                    .as_bytes(),
            )
            .into_owned(),
            ValueView::Vec(values) => {
                let values = values
                    .iter()
                    .map(|value| self.render_trace_argument(value, depth + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("vec[{values}]")
            }
            ValueView::Dict(values) => {
                let values = values
                    .iter()
                    .map(|(key, value)| {
                        let key = match key {
                            KeyRef::Int(key) => key.to_string(),
                            KeyRef::Bool(key) => key.to_string(),
                            KeyRef::String(key) => {
                                format!("'{}'", String::from_utf8_lossy(key.flatten()))
                            }
                            KeyRef::ShortString(key) => {
                                format!("'{}'", String::from_utf8_lossy(key.as_bytes()))
                            }
                        };
                        format!("{key} => {}", self.render_trace_argument(value, depth + 1))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                format!("dict[{values}]")
            }
            ValueView::Tuple(values) => {
                let values = values
                    .iter()
                    .map(|value| self.render_trace_argument(value, depth + 1))
                    .collect::<Vec<_>>();
                let trailing = if values.len() == 1 { "," } else { "" };
                format!("({}{trailing})", values.join(", "))
            }
            ValueView::Function(function) => function.signature().to_string(),
            ValueView::Iter(_) => "iterator".to_string(),
            _ => debug_render(&self.heap, value, depth),
        }
    }

    fn render_trace_arguments(&self, arguments: &Value) -> Option<String> {
        const LIMIT: usize = 512;

        let arguments = arguments.as_vec()?;
        if arguments.is_empty() {
            return None;
        }

        let mut rendered = arguments
            .iter()
            .map(|argument| self.render_trace_argument(argument, 0))
            .collect::<Vec<_>>()
            .join(", ");

        if rendered.len() > LIMIT {
            let mut end = LIMIT - 3;
            while !rendered.is_char_boundary(end) {
                end -= 1;
            }
            rendered.truncate(end);
            rendered.push_str("...");
        }

        Some(rendered)
    }

    /// Renders one throwable and its own source origin, trace, and previous
    /// chain.
    fn render_throwable(
        &self,
        instance: &ManagedRef<InstanceObject>,
        seen: &mut HashSet<usize>,
        color: bool,
    ) -> String {
        let address = instance.raw_box().addr().get();
        if !seen.insert(address) {
            return "[cyclic previous exception]".to_string();
        }

        let class_name = String::from_utf8_lossy(
            self.tables.classes[instance.class().0 as usize]
                .name
                .as_bytes(),
        )
        .into_owned();
        let message = text_of(&instance.read_slot(ERROR_SLOT_MESSAGE));
        let code = instance.read_slot(ERROR_SLOT_CODE).as_int().unwrap_or(0);
        let (headline, details) = message
            .split_once('\n')
            .map_or((message.as_str(), None), |(headline, details)| {
                (headline, Some(details))
            });

        let class_name = Self::diagnostic_paint("1;31", &class_name, color);
        let code = Self::diagnostic_paint("33", &code.to_string(), color);
        let mut rendered = format!("{class_name}: {headline} (code {code})");
        if let Some(details) = details {
            rendered.push('\n');
            rendered.push_str(details);
        }

        if let Some(origin) = self.exception_origin(instance) {
            rendered.push('\n');
            rendered.push_str(Self::render_origin(origin, color).trim_end());
            if let Some(note) = self.exception_note(instance) {
                rendered.push('\n');
                rendered.push_str("note: ");
                rendered.push_str(note);
            }
        }

        self.render_previous_chain(instance, seen, color, &mut rendered);
        self.render_stack_backtrace(instance, color, &mut rendered);

        rendered
    }

    /// Appends a compact cause chain without recursively repeating every
    /// cause's stack trace.
    fn render_previous_chain(
        &self,
        instance: &ManagedRef<InstanceObject>,
        seen: &mut HashSet<usize>,
        color: bool,
        rendered: &mut String,
    ) {
        let mut previous = instance.read_slot(ERROR_SLOT_PREVIOUS);
        let mut index = 0usize;
        let mut wrote_title = false;
        while let Some(cause) = previous.as_object() {
            if !self.is_throwable_instance(cause.class()) {
                break;
            }

            let address = cause.raw_box().addr().get();
            if !seen.insert(address) {
                if !wrote_title {
                    rendered.push_str("\n\nCaused by:");
                }

                rendered.push_str("\n  [cyclic previous exception]");
                break;
            }

            if !wrote_title {
                rendered.push_str("\n\n");
                rendered.push_str(&Self::diagnostic_paint("36", "Caused by:", color));
                wrote_title = true;
            }

            let class = String::from_utf8_lossy(
                self.tables.classes[cause.class().0 as usize]
                    .name
                    .as_bytes(),
            );

            let message = text_of(&cause.read_slot(ERROR_SLOT_MESSAGE));
            let headline = message
                .split_once('\n')
                .map_or(message.as_str(), |part| part.0);
            let _ = write!(
                rendered,
                "\n  {index}: {}: {}",
                Self::diagnostic_paint("1", &class, color),
                Self::diagnostic_paint("2", headline, color),
            );

            if let Some(origin) = self.exception_origin(cause) {
                let location = Self::origin_location(origin);
                rendered.push_str("\n     ");
                rendered.push_str(&Self::diagnostic_paint("36", "-->", color));
                rendered.push(' ');
                rendered.push_str(&Self::diagnostic_paint("2", &location, color));
            }

            previous = cause.read_slot(ERROR_SLOT_PREVIOUS);
            index += 1;
        }
    }

    fn origin_location(origin: &DiagnosticOrigin) -> String {
        let span = match &origin.labels {
            DiagnosticLabels::Single(label) => label.span,
            DiagnosticLabels::Multiple(labels) => {
                let Some(label) = labels.first() else {
                    return origin.path.to_string_lossy().into_owned();
                };
                label.span
            }
        };

        let offset = (span.start.offset as usize).min(origin.source.len());
        let source = &origin.source[..offset];
        let line = source.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = source
            .rsplit_once('\n')
            .map_or(source, |(_, tail)| tail)
            .chars()
            .count()
            + 1;

        format!("{}:{line}:{column}", origin.path.to_string_lossy())
    }

    fn render_stack_backtrace(
        &self,
        instance: &ManagedRef<InstanceObject>,
        color: bool,
        rendered: &mut String,
    ) {
        let trace = instance.read_slot(ERROR_SLOT_TRACE);
        self.render_trace(&trace, color, rendered);
    }

    fn render_trace(&self, trace: &Value, color: bool, rendered: &mut String) {
        let Some(entries) = trace.as_vec() else {
            return;
        };

        let frames = entries
            .iter()
            .filter_map(|entry| {
                let frame = entry.as_object()?;
                let function = text_of(&frame.read_slot(TRACE_FRAME_SLOT_FUNCTION));
                let file = text_of(&frame.read_slot(TRACE_FRAME_SLOT_FILE));
                let line = frame.read_slot(TRACE_FRAME_SLOT_LINE).as_int().unwrap_or(0);
                let arguments =
                    self.render_trace_arguments(&frame.read_slot(TRACE_FRAME_SLOT_ARGUMENTS));
                Some((function, file, line, arguments))
            })
            .collect::<Vec<_>>();

        if frames.is_empty() {
            return;
        }

        rendered.push_str("\n\n");
        rendered.push_str(&Self::diagnostic_paint("36", "Stack backtrace:", color));
        let mut index = 0usize;
        while index < frames.len() {
            if !self.configuration.full_trace
                && index > 0
                && Self::is_library_frame(&frames[index].1)
            {
                let start = index;
                while index < frames.len() && Self::is_library_frame(&frames[index].1) {
                    index += 1;
                }
                if index - start > 1 {
                    let label = Self::library_run_label(&frames[start..index]);
                    let hidden = format!("⋮ {} frames hidden - {label}", index - start);
                    rendered.push('\n');
                    rendered.push_str(&Self::diagnostic_paint("2", &hidden, color));
                    if index < frames.len() {
                        rendered.push('\n');
                    }
                    continue;
                }
                index = start;
            }

            let (function, file, line, arguments) = &frames[index];
            let library = Self::is_library_frame(file);
            let mut symbol = function.clone();
            if (!library || self.configuration.full_trace)
                && let Some(arguments) = arguments
            {
                symbol.push_str(" called with (");
                symbol.push_str(arguments);
                symbol.push(')');
            }
            let number = format!("{index:>2}");
            let location = if *line == 0 {
                file.clone()
            } else {
                format!("{file}:{line}")
            };

            rendered.push('\n');
            if library {
                let frame = format!("{number}  {symbol}\n    {location}");
                rendered.push_str(&Self::diagnostic_paint("2", &frame, color));
            } else {
                rendered.push_str(&Self::diagnostic_paint("2", &number, color));
                rendered.push_str("  ");
                rendered.push_str(&Self::diagnostic_paint("1", &symbol, color));
                rendered.push_str("\n    ");
                rendered.push_str(&Self::diagnostic_paint("2", &location, color));
            }

            index += 1;
            if index < frames.len() {
                rendered.push('\n');
            }
        }
    }

    fn is_library_frame(file: &str) -> bool {
        file.starts_with("<std>/") || file == "<internal>"
    }

    fn library_run_label(frames: &[(String, String, i64, Option<String>)]) -> String {
        let mut domains = frames.iter().filter_map(|(function, _, _, _)| {
            let mut components = function.split('\\');
            let root = components.next()?;
            let domain = components.next()?;
            Some(format!("{root}\\{domain}"))
        });

        let Some(first) = domains.next() else {
            return "library code".to_string();
        };

        if domains.all(|domain| domain == first) {
            first
        } else {
            "library code".to_string()
        }
    }

    fn diagnostic_paint(style: &str, text: &str, color: bool) -> String {
        if color {
            format!("\u{1b}[{style}m{text}\u{1b}[0m")
        } else {
            text.to_string()
        }
    }
}
