//! Loading compiled units, `require`, and the autoload chain.

use std::fs;
use std::path::Path;

use whim_span::HasSpan;
use whim_syn::arena::LocalArena;
use whim_syn::parser;

use crate::bytecode::unit::CompiledUnit;
use crate::compiler;
use crate::compiler::CompileConfiguration;
use crate::core::symbols::strip_leading_backslash;
use crate::engine::declare::CachedUnit;
use crate::engine::diagnostics::DiagnosticLabel;
use crate::engine::diagnostics::DiagnosticLabels;
use crate::engine::diagnostics::DiagnosticOrigin;
use crate::optimizer::OptimizationConfiguration;
use crate::path::path_bytes;
use crate::path::path_from_bytes;
use crate::symbols::line_starts_of;
use crate::vm::Atom;
use crate::vm::Frame;
use crate::vm::FrameFlags;
use crate::vm::NonNull;
use crate::vm::OptionalClassId;
use crate::vm::OptionalFuncId;
use crate::vm::Rc;
use crate::vm::SymbolKind;
use crate::vm::Throw;
use crate::vm::TypeEnvironmentId;
use crate::vm::UnitContext;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;

impl VirtualMachine<'_> {
    /// Handles a `require!` or `require_once!` from a frame: resolves the
    /// path relative to the requiring file, loads and declares the unit, and
    /// pushes its main chunk so a top-level `return` becomes the
    /// expression's value.
    pub(in crate::vm) fn require_from_frame(
        &mut self,
        path_value: Value,
        once: bool,
        destination: u16,
    ) -> Result<(), VirtualMachineControl> {
        let Some(requested) = path_value.as_string_bytes() else {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.type_error,
                format!(
                    "require! expects a path string, {} given",
                    path_value.kind_name()
                ),
            ));
        };

        let requested = requested.to_vec();
        let base = {
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let unit = unsafe { self.current_frame().unit.as_ref() };
            path_from_bytes(unit.path.as_bytes())
                .parent()
                .map(Path::to_path_buf)
        };

        match self.load_unit(&requested, base.as_deref(), once)? {
            None => {
                let target = self.current_frame().base as usize + usize::from(destination);
                self.stack[target] = Value::null();
                Ok(())
            }
            Some(context) => {
                self.push_unit_frame(&context, destination);
                Ok(())
            }
        }
    }

    /// Loads and declares a file. The loaded set prevents circular loads and
    /// makes later `require_once!` calls return `null`.
    pub(crate) fn load_unit(
        &mut self,
        requested: &[u8],
        base: Option<&Path>,
        once: bool,
    ) -> Result<Option<Rc<UnitContext>>, VirtualMachineControl> {
        let raw = path_from_bytes(requested);

        let joined = if raw.is_absolute() {
            raw
        } else {
            match base {
                Some(base) => base.join(&raw),
                None => raw,
            }
        };

        let canonical = match fs::canonicalize(&joined) {
            Ok(canonical) => canonical,
            Err(error) => {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.require_error,
                    format!("cannot read {}: {error}", joined.display()),
                ));
            }
        };

        if once && self.engine.loaded_paths.contains(&canonical) {
            return Ok(None);
        }

        if !self.engine.unit_cache.contains_key(&canonical) {
            let cached = self.compile_file(&canonical)?;
            self.engine.unit_cache.insert(canonical.clone(), cached);
        }

        self.engine.loaded_paths.insert(canonical.clone());
        let (unit, line_starts, source, lazy_callables) = {
            let cached = &self.engine.unit_cache[&canonical];
            (
                Rc::clone(&cached.unit),
                cached.line_starts.clone(),
                Rc::clone(&cached.source),
                cached.lazy_callables,
            )
        };

        self.autoload_unit_bases(&unit)?;
        let context =
            self.engine
                .declare_compiled(&unit, line_starts, Some(source), lazy_callables)?;
        if self.engine.configuration.optimize && !self.frames.is_empty() {
            self.world_refinement_pending = true;
        }
        Ok(Some(context))
    }

    fn autoload_unit_bases(
        &mut self,
        unit: &Rc<CompiledUnit>,
    ) -> Result<(), VirtualMachineControl> {
        if self.engine.autoloader.is_none() {
            return Ok(());
        }

        for class in &unit.classes {
            let parent = class
                .parent
                .iter()
                .map(|base| (SymbolKind::Class, &base.name));
            let interfaces = class
                .interfaces
                .iter()
                .map(|base| (SymbolKind::Interface, &base.name));
            for (kind, name) in parent.chain(interfaces) {
                if self.engine.tables.symbols.contains_key(name)
                    || unit.classes.iter().any(|other| other.name == *name)
                {
                    continue;
                }

                self.run_autoload_chain(kind, name.clone())?;
            }
        }

        Ok(())
    }

    /// Parses and compiles one file into a cache entry.
    fn compile_file(&mut self, canonical: &Path) -> Result<CachedUnit, VirtualMachineControl> {
        let source = match fs::read_to_string(canonical) {
            Ok(source) => source,
            Err(error) => {
                return Err(self.throw_well_known(
                    self.engine.tables.well_known.require_error,
                    format!("cannot read {}: {error}", canonical.display()),
                ));
            }
        };

        let arena = LocalArena::new();
        let path_text = canonical.display().to_string();
        let path_atom = self.heap.intern(&path_bytes(canonical));
        let retained_source = Rc::<str>::from(source.as_str());
        let program = match parser::parse(&arena, &source) {
            Ok(program) => program,
            Err(errors) => {
                let origin = DiagnosticOrigin {
                    path: path_atom,
                    source: retained_source,
                    labels: DiagnosticLabels::Single(DiagnosticLabel {
                        span: errors.span(),
                        message: errors.to_string(),
                    }),
                };

                return Err(self.throw_well_known_at(
                    self.engine.tables.well_known.parser_error,
                    errors.to_string(),
                    origin,
                ));
            }
        };

        let unit = match compiler::compile_with_path_bytes_configuration_and_built_in_functions(
            program,
            &path_text,
            &path_bytes(canonical),
            &self.heap,
            CompileConfiguration {
                optimization: OptimizationConfiguration {
                    enabled: self.engine.compiler_optimizes_units(),
                    ..OptimizationConfiguration::default()
                },
                trusted_return_types: false,
            },
            &self.engine.tables.built_in_function_declarations,
        ) {
            Ok(unit) => unit,
            Err(error) => {
                let origin = DiagnosticOrigin {
                    path: path_atom,
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

                return Err(self.throw_well_known_at(
                    self.engine.tables.well_known.compiler_error,
                    error.message,
                    origin,
                ));
            }
        };

        let mut unit = unit;
        self.engine.expand_declared_types(&mut unit);
        let (unit, lazy_callables) = self.engine.optimize_required_unit_against_world(unit);
        Ok(CachedUnit {
            unit: Rc::new(unit),
            source: retained_source,
            line_starts: line_starts_of(&source),
            lazy_callables,
        })
    }
    fn push_unit_frame(&mut self, context: &Rc<UnitContext>, destination: u16) {
        let chunk = context.main_chunk;
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let register_count = usize::from(unsafe { chunk.as_ref() }.register_count);
        let base = self.stack.len();
        self.resize_frame_stack(base + register_count);
        self.reset_uninitialized_locals(base, chunk);
        self.frames.push(Frame {
            chunk,
            cache: NonNull::from(&*context.main_cache),
            unit: NonNull::from(&**context),
            function: OptionalFuncId::NONE,
            ip: 0,
            base: base as u32,
            argc: 0,
            called_class: OptionalClassId::NONE,
            class_scope: OptionalClassId::NONE,
            stack_floor_offset: 0,
            reference_register_mask: context.unit.main.reference_register_mask,
            return_register: destination,
            flags: FrameFlags::new(false, false, false),
            type_environment: TypeEnvironmentId::default(),
        });
    }

    pub(crate) fn run_autoload_chain(
        &mut self,
        kind: SymbolKind,
        name: Atom,
    ) -> Result<bool, VirtualMachineControl> {
        let Some(autoloader) = self.engine.autoloader.clone() else {
            return Ok(false);
        };

        if !self.engine.autoload_in_flight.insert((kind, name.clone())) {
            return Ok(false);
        }

        let kind_value = Value::int(kind as i64);
        let name_value = Value::string(name.to_handle());
        let outcome = self.call_callee_reentrant(&autoloader, &[kind_value, name_value]);
        self.engine.autoload_in_flight.remove(&(kind, name));
        outcome?;
        Ok(true)
    }
}

impl VirtualMachine<'_> {
    /// Installs the one autoload callback, reporting `false` when the engine
    /// already holds one. Never re-enters the interpreter.
    pub(crate) fn install_autoloader(&mut self, autoloader: Value) -> bool {
        if self.engine.autoloader.is_some() {
            return false;
        }

        self.engine.autoloader = Some(autoloader);
        true
    }

    pub(crate) fn run_autoload(&mut self, kind: SymbolKind, name: Atom) -> Result<bool, Throw> {
        self.run_autoload_chain(kind, strip_leading_backslash(&self.heap, name))
            .map_err(|control| self.control_to_throw(control))
    }
}
