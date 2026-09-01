//! Declaring a compiled unit's symbols.

use std::iter;

use hashbrown::HashSet;
use whim_span::Position;
use whim_span::Span;

use crate::bytecode::aliases::expand_unit_declarations;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::is_external;
use crate::engine::Atom;
use crate::engine::ClassId;
use crate::engine::CompiledUnit;
use crate::engine::ConstantInitializer;
use crate::engine::Engine;
use crate::engine::ExecutionOutcome;
use crate::engine::FuncId;
use crate::engine::FunctionLocator;
use crate::engine::FunctionTable;
use crate::engine::GenericValidationJournalEntry;
use crate::engine::HashMap;
use crate::engine::InlineCache;
use crate::engine::InstanceObject;
use crate::engine::ManagedRef;
use crate::engine::NonNull;
use crate::engine::Rc;
use crate::engine::SymbolKind;
use crate::engine::UnitContext;
use crate::engine::UnitGenericValidation;
use crate::engine::Value;
use crate::engine::VecObject;
use crate::engine::VirtualMachine;
use crate::engine::VirtualMachineControl;
use crate::engine::builtins::BuiltInCallable;
use crate::engine::diagnostics::DiagnosticLabel;
use crate::engine::diagnostics::DiagnosticLabels;
use crate::engine::diagnostics::DiagnosticOrigin;
use crate::engine::literal_value;
use crate::engine::runtime_function;
use crate::symbols::CallableOptimization;
use crate::symbols::ExactBuiltInFunctionEntry;
use crate::symbols::ExactFunctionEntry;
use crate::symbols::RuntimeFunction;
use crate::symbols::SourceText;
use crate::symbols::SymbolEntry;
use crate::symbols::UnitOrigin;
use crate::symbols::UnitSourceFile;
use crate::u32_index;
use crate::unreachable_invariant;
use crate::value::function::BuiltInId;
use crate::value::string::ByteStringObject;

pub(crate) enum ConstantSlot {
    Pending {
        context: Rc<UnitContext>,
        position: u32,
    },
    /// Being forced; a read that re-enters it is a self-referential cycle.
    Evaluating,
    Evaluated(Value),
}

/// One parsed and compiled file in the unit cache.
pub(crate) struct CachedUnit {
    pub unit: Rc<CompiledUnit>,
    /// The source text the unit was compiled from.
    pub source: Rc<str>,
    /// The line-start offsets of the file's source.
    pub line_starts: Vec<u32>,
    /// Whether callable bodies are waiting for exactly-once optimization.
    pub lazy_callables: bool,
}

/// The engine state needed to undo a failed unit registration.
pub(in crate::engine) struct RegistrationMark {
    pub(crate) functions: usize,
    pub(crate) classes: usize,
    pub(crate) units: usize,
    pub(crate) generic_validation_journal: usize,
    pub(crate) type_aliases: usize,
    pub(crate) newtypes: usize,
    pub(crate) constants: usize,
}

type PendingClassConstants = Vec<(ClassId, Vec<Atom>)>;
type PendingStaticProperties = Vec<(ClassId, Vec<Value>)>;

fn remove_external_declarations(unit: &mut CompiledUnit) {
    unit.functions
        .retain(|function| !is_external(&function.attributes));
    unit.classes.retain(|class| !is_external(&class.attributes));
    unit.constants
        .retain(|constant| !is_external(&constant.attributes));
    unit.type_aliases
        .retain(|alias| !is_external(&alias.attributes));
    unit.newtypes
        .retain(|newtype| !is_external(&newtype.attributes));
}

fn optimizer_destructors(unit: &CompiledUnit) -> [Option<u32>; 2] {
    let mut destructors = [None, None];
    let mut next = 0;
    for (position, class) in unit.classes.iter().enumerate() {
        if !class
            .methods
            .iter()
            .any(|method| method.name.as_bytes() == b"__destruct")
        {
            continue;
        }

        destructors[next] = Some(u32_index(position));
        next += 1;
        if next == destructors.len() {
            break;
        }
    }

    destructors
}

impl Engine {
    /// Declares a unit and runs its main chunk.
    pub(in crate::engine) fn run_declared(
        &mut self,
        unit: &Rc<CompiledUnit>,
        line_starts: Vec<u32>,
        source: Option<Rc<str>>,
    ) -> ExecutionOutcome {
        let source = source.map(SourceText::Shared);
        let context = match self.declare_internal(
            unit,
            line_starts,
            source,
            Vec::new(),
            false,
            UnitOrigin::User,
        ) {
            Ok(context) => context,
            Err(VirtualMachineControl::Throw(value)) => return self.uncaught(value),
            Err(VirtualMachineControl::Exit(code)) => {
                return ExecutionOutcome::exited(code);
            }
        };

        let mut vm = VirtualMachine::new(self);
        let execution = vm.run_main(&context);
        let execution = match execution {
            Ok(value) => {
                drop(value);
                Ok(())
            }
            Err(control) => Err(control),
        };
        vm.clear_main_task_local_values();
        let shutdown = vm.run_shutdown_finalizers();
        drop(vm);
        match shutdown.and(execution) {
            Ok(()) => ExecutionOutcome::completed(),
            Err(VirtualMachineControl::Throw(value)) => self.uncaught(value),
            Err(VirtualMachineControl::Exit(code)) => ExecutionOutcome::exited(code),
        }
    }

    /// Registers a unit's declarations: redeclarations are checked first so
    /// a failure leaves no partial registration of names, then functions,
    /// classes in dependency order, type aliases, and finally the constant
    /// initializers evaluate in declaration order.
    pub(crate) fn declare_compiled(
        &mut self,
        unit: &Rc<CompiledUnit>,
        line_starts: Vec<u32>,
        source: Option<Rc<str>>,
        lazy_callables: bool,
    ) -> Result<Rc<UnitContext>, VirtualMachineControl> {
        self.declare_internal(
            unit,
            line_starts,
            source.map(SourceText::Shared),
            Vec::new(),
            lazy_callables,
            UnitOrigin::User,
        )
    }

    /// Expands alias references in an owned unit's declaration types against
    /// every loaded alias, so runtime checks against its declarations never
    /// resolve an alias by name. Must run before the unit is shared: `require`
    /// recognizes an already-declared unit by pointer identity.
    pub(crate) fn expand_declared_types(&self, unit: &mut CompiledUnit) {
        if self.tables.type_aliases.is_empty() && unit.type_aliases.is_empty() {
            return;
        }

        let alias_table: Vec<CompiledTypeAlias> = self
            .tables
            .type_aliases
            .iter()
            .chain(unit.type_aliases.iter())
            .cloned()
            .collect();
        expand_unit_declarations(unit, &alias_table);
    }

    pub(crate) fn prepare_artifact_unit(
        &mut self,
        unit: CompiledUnit,
        line_starts: Vec<u32>,
        source: Rc<str>,
        source_files: Vec<UnitSourceFile>,
    ) -> Result<CompiledUnit, VirtualMachineControl> {
        let unit = Rc::new(unit);
        let validation_context = Rc::new(UnitContext {
            path: unit.path.clone(),
            origin: UnitOrigin::Extension,
            source: Some(SourceText::Shared(source)),
            line_starts,
            source_files,
            main_cache: Box::new(InlineCache::new()),
            main_chunk: NonNull::from(&unit.main),
            closures: HashMap::new(),
            lazy_callables: false,
            optimizer_destructors: optimizer_destructors(&unit),
            unit: Rc::clone(&unit),
        });
        self.validate_external_declarations(&unit, &validation_context)?;
        drop(validation_context);

        let Ok(mut unit) = Rc::try_unwrap(unit) else {
            // SAFETY: the surrounding invariant makes this path unreachable.
            unsafe { unreachable_invariant("artifact validation retained its compiled unit") }
        };
        remove_external_declarations(&mut unit);
        Ok(unit)
    }

    pub(crate) fn declare_compiled_with_source_files(
        &mut self,
        unit: &Rc<CompiledUnit>,
        line_starts: Vec<u32>,
        source: SourceText,
        source_files: Vec<UnitSourceFile>,
    ) -> Result<Rc<UnitContext>, VirtualMachineControl> {
        self.declare_internal(
            unit,
            line_starts,
            Some(source),
            source_files,
            false,
            UnitOrigin::Extension,
        )
    }

    fn declare_internal(
        &mut self,
        unit: &Rc<CompiledUnit>,
        line_starts: Vec<u32>,
        source: Option<SourceText>,
        source_files: Vec<UnitSourceFile>,
        lazy_callables: bool,
        origin: UnitOrigin,
    ) -> Result<Rc<UnitContext>, VirtualMachineControl> {
        if let Some(source) = &source {
            self.sources.insert(unit.path.clone(), source.clone());
        }

        let mark = RegistrationMark {
            functions: self.tables.functions.len(),
            classes: self.tables.classes.len(),
            units: self.units.len(),
            generic_validation_journal: self.generic_validation_journal.len(),
            type_aliases: self.tables.type_aliases.len(),
            newtypes: self.tables.newtypes.len(),
            constants: self.tables.constants.len(),
        };

        self.declaration_depth += 1;
        let claim_capacity = unit.functions.len()
            + unit.classes.len()
            + unit.constants.len()
            + unit.type_aliases.len()
            + unit.newtypes.len();
        let mut claimed = HashSet::with_capacity(claim_capacity);
        let outcome = self.declare_staged(
            unit,
            line_starts,
            source,
            source_files,
            lazy_callables,
            origin,
            &mut claimed,
        );
        self.declaration_depth -= 1;
        match outcome {
            Ok(context) => {
                if self.declaration_depth == 0 {
                    self.generic_validation_journal.clear();
                }
                Ok(context)
            }
            Err(control) => {
                self.rollback_registration(&mark, &claimed);
                Err(control)
            }
        }
    }

    /// Undoes a failed registration, restoring every table to its length
    /// before the attempt and releasing the names the unit had claimed.
    pub(in crate::engine) fn rollback_registration(
        &mut self,
        mark: &RegistrationMark,
        claimed: &HashSet<Atom>,
    ) {
        for name in claimed {
            self.tables.symbols.remove(name);
        }

        self.tables.functions.truncate(mark.functions);
        self.tables.classes.truncate(mark.classes);
        self.units.truncate(mark.units);
        self.optimizer_world = None;
        self.optimizer_class_contexts.clear();
        while self.generic_validation_journal.len() > mark.generic_validation_journal {
            let Some(validation) = self.generic_validation_journal.pop() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("the validation journal length was checked") }
            };

            match validation {
                GenericValidationJournalEntry::TypeArgumentBounds(index) => {
                    if index < mark.units {
                        self.unit_generic_validation[index].type_argument_bounds = false;
                    }
                }
                GenericValidationJournalEntry::TypeParameterDefaults(index) => {
                    if index < mark.units {
                        self.unit_generic_validation[index].type_parameter_defaults = false;
                    }
                }
            }
        }
        self.unit_generic_validation.truncate(mark.units);
        self.tables.type_aliases.truncate(mark.type_aliases);
        self.tables.newtypes.truncate(mark.newtypes);
        self.tables.constants.truncate(mark.constants);
        self.tables.has_destructor_classes = self
            .tables
            .classes
            .iter()
            .any(|class| class.destructor.is_some());
    }

    fn claim_unit_names(
        &mut self,
        unit: &CompiledUnit,
        claimed: &mut HashSet<Atom>,
    ) -> Result<(), VirtualMachineControl> {
        for function in &unit.functions {
            if !function.name.as_bytes().starts_with(b"{closure")
                && !is_external(&function.attributes)
            {
                self.claim_name(&unit.path, &function.name, function.span, claimed)?;
            }
        }
        for class in &unit.classes {
            if !is_external(&class.attributes) {
                self.claim_name(&unit.path, &class.name, class.span, claimed)?;
            }
        }
        for constant in &unit.constants {
            if !is_external(&constant.attributes) {
                self.claim_name(&unit.path, &constant.name, constant.span, claimed)?;
            }
        }
        for alias in &unit.type_aliases {
            if !is_external(&alias.attributes) {
                self.claim_name(&unit.path, &alias.name, alias.span, claimed)?;
            }
        }
        for newtype in &unit.newtypes {
            if !is_external(&newtype.attributes) {
                self.claim_name(&unit.path, &newtype.name, newtype.span, claimed)?;
            }
        }

        Ok(())
    }

    fn claim_name(
        &mut self,
        path: &Atom,
        name: &Atom,
        span: Span,
        claimed: &mut HashSet<Atom>,
    ) -> Result<(), VirtualMachineControl> {
        if self.tables.symbols.contains_key(name) || !claimed.insert(name.clone()) {
            let text = name.to_string_lossy();
            let message = format!("the name {text} is already declared");
            return Err(VirtualMachineControl::Throw(self.declaration_error_at(
                self.tables.well_known.linker_error,
                message.clone(),
                path,
                DiagnosticLabel { span, message },
            )));
        }

        Ok(())
    }

    fn closure_map(&self, unit: &CompiledUnit) -> HashMap<Atom, FuncId> {
        let function_base = self.tables.functions.len();
        let mut offset = 0usize;
        let mut closures = HashMap::new();
        for function in &unit.functions {
            if is_external(&function.attributes) {
                continue;
            }
            if function.name.as_bytes().starts_with(b"{closure") {
                closures.insert(
                    function.name.clone(),
                    FuncId(u32_index(function_base + offset)),
                );
            }
            offset += 1;
        }
        closures
    }

    fn register_unit_functions(&mut self, unit: &CompiledUnit, context: &Rc<UnitContext>) {
        for (index, function) in unit.functions.iter().enumerate() {
            if is_external(&function.attributes) {
                continue;
            }
            let id = FuncId(u32_index(self.tables.functions.len()));
            if !function.name.as_bytes().starts_with(b"{closure") {
                self.tables.symbols.insert(
                    function.name.clone(),
                    SymbolEntry {
                        kind: SymbolKind::Function,
                        index: id.0,
                        table: FunctionTable::User,
                    },
                );
            }
            self.tables.functions.push(runtime_function(
                function,
                context,
                FunctionLocator::TopLevel(u32_index(index)),
                None,
            ));
        }
    }

    fn register_unit_types(&mut self, unit: &CompiledUnit) {
        for alias in &unit.type_aliases {
            if is_external(&alias.attributes) {
                continue;
            }
            let index = u32_index(self.tables.type_aliases.len());
            self.tables.type_aliases.push(alias.clone());
            self.tables.symbols.insert(
                alias.name.clone(),
                SymbolEntry {
                    kind: SymbolKind::TypeAlias,
                    index,
                    table: FunctionTable::User,
                },
            );
        }

        for newtype in &unit.newtypes {
            if is_external(&newtype.attributes) {
                continue;
            }
            let index = u32_index(self.tables.newtypes.len());
            self.tables.newtypes.push(newtype.clone());
            self.tables.symbols.insert(
                newtype.name.clone(),
                SymbolEntry {
                    kind: SymbolKind::Newtype,
                    index,
                    table: FunctionTable::User,
                },
            );
        }
    }

    fn register_unit_constants(
        &mut self,
        unit: &CompiledUnit,
        context: &Rc<UnitContext>,
    ) -> Vec<(u32, Atom)> {
        let mut pending = Vec::with_capacity(unit.constants.len());
        for (position, constant) in unit.constants.iter().enumerate() {
            if is_external(&constant.attributes) {
                continue;
            }
            let index = u32_index(self.tables.constants.len());
            self.tables.constants.push(ConstantSlot::Pending {
                context: Rc::clone(context),
                position: u32_index(position),
            });
            self.tables.symbols.insert(
                constant.name.clone(),
                SymbolEntry {
                    kind: SymbolKind::Constant,
                    index,
                    table: FunctionTable::User,
                },
            );
            pending.push((index, constant.name.clone()));
        }
        pending
    }

    fn initialize_static_properties(
        &mut self,
        unit: &CompiledUnit,
        context: &Rc<UnitContext>,
    ) -> Result<(), VirtualMachineControl> {
        for compiled in &unit.classes {
            if is_external(&compiled.attributes) {
                continue;
            }
            let Some(entry) = self.tables.symbols.get(&compiled.name).copied() else {
                continue;
            };
            let class_id = ClassId(entry.index);
            for property in &compiled.properties {
                if !property.is_static {
                    continue;
                }
                let Some(initializer @ ConstantInitializer::Thunk(_)) = &property.default else {
                    continue;
                };
                let slot = self.tables.classes[class_id.0 as usize].static_names[&property.name];
                let value = self.evaluate_initializer(initializer, context)?;
                self.tables.classes[class_id.0 as usize]
                    .statics
                    .borrow_mut()[slot as usize] = value;
            }
        }
        Ok(())
    }

    fn pending_class_values(
        &self,
        unit: &CompiledUnit,
    ) -> (PendingClassConstants, PendingStaticProperties) {
        let mut constants = Vec::new();
        let mut statics = Vec::new();
        for compiled in &unit.classes {
            if is_external(&compiled.attributes) {
                continue;
            }
            let Some(entry) = self.tables.symbols.get(&compiled.name) else {
                continue;
            };
            let class_id = ClassId(entry.index);
            if !compiled.constants.is_empty() {
                constants.push((
                    class_id,
                    compiled
                        .constants
                        .iter()
                        .map(|constant| constant.name.clone())
                        .collect(),
                ));
            }

            let values = self.tables.classes[class_id.0 as usize]
                .statics
                .borrow()
                .clone();
            if !values.is_empty() {
                statics.push((class_id, values));
            }
        }
        (constants, statics)
    }

    #[expect(clippy::too_many_arguments, reason = "unit declaration inputs")]
    fn declare_staged(
        &mut self,
        unit: &Rc<CompiledUnit>,
        line_starts: Vec<u32>,
        source: Option<SourceText>,
        source_files: Vec<UnitSourceFile>,
        lazy_callables: bool,
        origin: UnitOrigin,
        claimed: &mut HashSet<Atom>,
    ) -> Result<Rc<UnitContext>, VirtualMachineControl> {
        self.claim_unit_names(unit, claimed)?;

        let context = Rc::new(UnitContext {
            unit: Rc::clone(unit),
            origin,
            path: unit.path.clone(),
            source,
            line_starts,
            source_files,
            main_cache: Box::new(InlineCache::new()),
            main_chunk: NonNull::from(&unit.main),
            closures: self.closure_map(unit),
            lazy_callables,
            optimizer_destructors: optimizer_destructors(unit),
        });

        self.validate_external_declarations(unit, &context)?;
        self.units.push(Rc::clone(&context));
        if let Some(world) = &mut self.optimizer_world {
            world.push(Rc::clone(&context.unit));
        }
        self.unit_generic_validation
            .push(UnitGenericValidation::default());
        self.register_unit_functions(unit, &context);
        self.register_unit_types(unit);
        let top_level_constants = self.register_unit_constants(unit, &context);

        self.link_unit_classes(unit, &context)?;
        self.validate_loaded_type_parameter_defaults()?;
        self.validate_loaded_type_argument_bounds()?;
        self.prelink_exact_function_caches(&context)?;

        self.initialize_static_properties(unit, &context)?;
        let (class_constants, static_properties) = self.pending_class_values(unit);

        let mut vm = VirtualMachine::new(self);
        for (class_id, values) in static_properties {
            for (slot, value) in values.iter().enumerate() {
                if !value.is_uninitialized() {
                    vm.check_static_property_value(class_id, u32_index(slot), value)?;
                }
            }
        }

        for (class_id, names) in class_constants {
            for name in names {
                vm.force_class_constant(class_id, name)?;
            }
        }

        for (index, name) in top_level_constants {
            vm.force_constant(index, name)?;
        }

        Ok(context)
    }

    /// Fills optimizer-proven named-function cache slots before execution, so
    /// their hot handler can trust the direct target without resolving or
    /// branching on cache state.
    fn prelink_exact_function_caches(
        &mut self,
        context: &Rc<UnitContext>,
    ) -> Result<(), VirtualMachineControl> {
        // SAFETY: the table and prior lookup prove this pointer or index.
        let main = unsafe { context.main_chunk.as_ref() };
        let result = prelink_exact_function_cache(
            &context.main_cache,
            main,
            &self.tables.symbols,
            &self.tables.functions,
            &self.tables.built_in_functions,
        );
        if let Err(name) = result {
            return Err(self.invalid_exact_function_target(&context.path, &name));
        }

        for function in &self.tables.functions {
            if !Rc::ptr_eq(&function.unit, context) {
                continue;
            }
            if function.optimization != CallableOptimization::Complete {
                continue;
            }

            // SAFETY: the table and prior lookup prove this pointer or index.
            let chunk = unsafe { function.chunk.as_ref() };
            if let Err(name) = prelink_exact_function_cache(
                &function.cache,
                chunk,
                &self.tables.symbols,
                &self.tables.functions,
                &self.tables.built_in_functions,
            ) {
                return Err(self.invalid_exact_function_target(&context.path, &name));
            }
        }

        Ok(())
    }

    pub(in crate::engine) fn invalid_exact_function_target(
        &mut self,
        path: &Atom,
        name: &Atom,
    ) -> VirtualMachineControl {
        VirtualMachineControl::Throw(self.declaration_error(
            self.tables.well_known.linker_error,
            format!(
                "the optimized call target {} is not a declared user function",
                name.to_string_lossy()
            ),
            path,
        ))
    }

    pub(crate) fn evaluate_initializer(
        &mut self,
        initializer: &ConstantInitializer,
        context: &Rc<UnitContext>,
    ) -> Result<Value, VirtualMachineControl> {
        match initializer {
            ConstantInitializer::Literal(literal) => Ok(literal_value(literal)),
            ConstantInitializer::Thunk(chunk) => {
                let chunk_pointer = NonNull::from(&**chunk);
                let mut vm = VirtualMachine::new(self);
                vm.run_initializer(chunk_pointer, context)
            }
        }
    }

    /// Builds an error instance at declaration time, before any frame
    /// exists: the file is the declaring unit's path, the line is zero, and
    /// the trace is empty.
    pub(crate) fn declaration_error(
        &mut self,
        class: ClassId,
        message: String,
        path: &Atom,
    ) -> Value {
        let message = message.into_bytes();
        let slot_count = self.tables.classes[class.0 as usize].slots.len();
        let instance = InstanceObject::new(&self.heap, class, slot_count);
        self.write_error_slot(
            &instance,
            class,
            b"message",
            Value::string(ByteStringObject::from_bytes(&self.heap, &message)),
        );

        self.write_error_slot(&instance, class, b"code", Value::int(0));
        self.write_error_slot(&instance, class, b"file", Value::string(path.to_handle()));
        self.write_error_slot(&instance, class, b"line", Value::int(0));
        self.write_error_slot(
            &instance,
            class,
            b"trace",
            Value::vec(VecObject::new(&self.heap)),
        );

        self.write_error_slot(&instance, class, b"previous", Value::null());
        let error = Value::object(instance);
        if let Some(source) = self.sources.get(path).map(SourceText::to_rc) {
            self.record_exception_origin(
                &error,
                DiagnosticOrigin {
                    path: path.clone(),
                    source,
                    labels: DiagnosticLabels::Single(DiagnosticLabel {
                        span: Span::new(Position::new(0), Position::new(0)),
                        message: "the declaration could not be linked".to_string(),
                    }),
                },
            );
        }
        error
    }

    pub(crate) fn declaration_error_with_origin(
        &mut self,
        class: ClassId,
        message: String,
        path: &Atom,
        origin: DiagnosticOrigin,
    ) -> Value {
        let error = self.declaration_error(class, message, path);
        self.record_exception_origin(&error, origin);
        error
    }

    pub(crate) fn declaration_error_at(
        &mut self,
        class: ClassId,
        message: String,
        path: &Atom,
        label: DiagnosticLabel,
    ) -> Value {
        let Some(origin) = self.diagnostic_origin(path, label) else {
            return self.declaration_error(class, message, path);
        };
        self.declaration_error_with_origin(class, message, path, origin)
    }

    pub(crate) fn linker_error_at(
        &mut self,
        path: &Atom,
        span: Span,
        message: String,
    ) -> VirtualMachineControl {
        VirtualMachineControl::Throw(self.declaration_error_at(
            self.tables.well_known.linker_error,
            message.clone(),
            path,
            DiagnosticLabel { span, message },
        ))
    }

    /// Writes one error slot by property name through the class's resolved
    /// layout, so the engine never hardcodes the `Whim\Unwind\Error` slot
    /// order. Never re-enters the interpreter.
    pub(crate) fn write_error_slot(
        &self,
        instance: &ManagedRef<InstanceObject>,
        class: ClassId,
        name: &[u8],
        value: Value,
    ) {
        let atom = self.heap.intern(name);
        if let Some(slot) = self.tables.classes[class.0 as usize]
            .slot_names
            .get(&atom)
            .copied()
        {
            drop(instance.write_slot(slot as usize, value));
        }
    }
}

pub(crate) struct PrelinkedFunctionSites {
    functions: Vec<Option<ExactFunctionEntry>>,
    built_in_functions: Vec<Option<ExactBuiltInFunctionEntry>>,
}

impl PrelinkedFunctionSites {
    pub(crate) fn install(self, cache: &InlineCache) {
        // SAFETY: the table and prior lookup prove this pointer or index.
        unsafe {
            *cache.exact_functions() = self.functions;
            *cache.exact_built_in_functions() = self.built_in_functions;
        }
    }
}

pub(crate) fn prelink_exact_function_sites(
    chunk: &Chunk,
    symbols: &HashMap<Atom, SymbolEntry>,
    functions: &[RuntimeFunction],
    built_in_functions: &[BuiltInCallable],
) -> Result<PrelinkedFunctionSites, Atom> {
    let mut entries = vec![None; chunk.ic_descriptors.len()];
    let mut built_in_entries = vec![None; chunk.ic_descriptors.len()];
    let mut sites = chunk.code.iter().filter_map(|instruction| {
        let (cache, destination, argument_count) = match instruction {
            Instruction::CallNamedUnchecked {
                cache,
                destination,
                argument_count,
                ..
            } => (cache, destination, argument_count.value()),
            Instruction::CallNamedConstantUnchecked {
                cache, destination, ..
            } => (cache, destination, 1),
            _ => return None,
        };

        Some((cache.index() as usize, destination.index(), argument_count))
    });

    let Some(first) = sites.next() else {
        return Ok(PrelinkedFunctionSites {
            functions: entries,
            built_in_functions: built_in_entries,
        });
    };

    for (site, destination, argument_count) in iter::once(first).chain(sites) {
        let (name, has_type_arguments) = match &chunk.ic_descriptors[site] {
            IcDescriptor::Member {
                name,
                type_arguments,
            } => (name, type_arguments.is_some()),
            IcDescriptor::ClassMember { class, .. } => return Err(class.clone()),
        };

        let Some(symbol) = symbols.get(name) else {
            return Err(name.clone());
        };

        if symbol.kind != SymbolKind::Function {
            return Err(name.clone());
        }

        match symbol.table {
            FunctionTable::User => {
                let function = FuncId(symbol.index);
                // SAFETY: the surrounding invariant keeps this index in bounds.
                let runtime = unsafe { functions.get_unchecked(function.0 as usize) };
                entries[site] = Some(ExactFunctionEntry::from_call_site(
                    function,
                    runtime,
                    chunk,
                    destination,
                ));
            }
            FunctionTable::BuiltIn => {
                let function = BuiltInId(symbol.index);
                let BuiltInCallable::Function(spec) =
                    // SAFETY: the surrounding invariant keeps this index in bounds.
                    (unsafe { built_in_functions.get_unchecked(function.0 as usize) })
                else {
                    return Err(name.clone());
                };
                let direct_handler = (!has_type_arguments
                    && spec.type_parameters.is_empty()
                    && usize::from(argument_count) == spec.parameters.len())
                .then_some(spec.direct_handler)
                .flatten();
                built_in_entries[site] = Some(ExactBuiltInFunctionEntry {
                    function,
                    direct_handler,
                });
            }
        }
    }

    Ok(PrelinkedFunctionSites {
        functions: entries,
        built_in_functions: built_in_entries,
    })
}

pub(crate) fn prelink_exact_function_cache(
    cache: &InlineCache,
    chunk: &Chunk,
    symbols: &HashMap<Atom, SymbolEntry>,
    functions: &[RuntimeFunction],
    built_in_functions: &[BuiltInCallable],
) -> Result<(), Atom> {
    let sites = prelink_exact_function_sites(chunk, symbols, functions, built_in_functions)?;
    sites.install(cache);
    Ok(())
}
