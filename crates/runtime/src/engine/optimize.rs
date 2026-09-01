//! Exactly-once optimization of callable bodies when execution first reaches them.

use std::ptr::NonNull;

use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::verify::verify;
use crate::engine::Engine;
use crate::engine::Rc;
use crate::engine::declare::prelink_exact_function_cache;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::World;
use crate::optimizer::WorldCache;
use crate::optimizer::optimize_callable_function;
use crate::optimizer::optimize_callable_method;
use crate::symbols::CallableOptimization;
use crate::symbols::FunctionLocator;
use crate::symbols::UnitContext;
use crate::unwrap_option_invariant;
use crate::value::function::FuncId;
use crate::vm::VirtualMachineControl;

impl Engine {
    /// Ensures one user callable has its final body and direct-call cache.
    #[expect(
        clippy::inline_always,
        reason = "the complete-state check is a hot exact-call fast path"
    )]
    #[inline(always)]
    pub(crate) fn optimize_callable_once(
        &mut self,
        function: FuncId,
    ) -> Result<(), VirtualMachineControl> {
        let position = function.0 as usize;
        if self.tables.functions[position].optimization == CallableOptimization::Complete {
            return Ok(());
        }

        self.optimize_pending_callable(function)
    }

    #[cold]
    fn optimize_pending_callable(&mut self, function: FuncId) -> Result<(), VirtualMachineControl> {
        let position = function.0 as usize;
        if self.tables.functions[position].optimization == CallableOptimization::Optimizing {
            return Ok(());
        }

        if self.tables.functions[position].frameless_literal.is_some() {
            self.tables.functions[position].optimization = CallableOptimization::Complete;
            return Ok(());
        }

        let context = Rc::clone(&self.tables.functions[position].unit);
        let locator = self.tables.functions[position].locator;
        // SAFETY: the table and prior lookup prove this pointer or index.
        let original = unsafe { self.tables.functions[position].chunk.as_ref() }.clone();
        let pending = Box::new(original);
        self.tables.functions[position].chunk = NonNull::from(&*pending);
        self.tables.functions[position].optimized_chunk = Some(pending);
        self.tables.functions[position].optimization = CallableOptimization::Optimizing;

        if self.optimizer_world.is_none() {
            let units = self
                .units
                .iter()
                .map(|context| Rc::clone(&context.unit))
                .collect();
            self.optimizer_world = Some(WorldCache::new(
                units,
                &self.tables.built_in_function_declarations,
            ));
        }

        let class_contexts = self.take_optimizer_class_contexts(&context, locator);

        let (optimized, class_contexts) = {
            // SAFETY: the cache is initialized immediately above.
            let cache = unsafe {
                unwrap_option_invariant(
                    self.optimizer_world.as_ref(),
                    "a callable optimizer world cache was initialized",
                )
            };
            let world = World::from_cache(cache, &self.tables.built_in_function_declarations);
            match locator {
                FunctionLocator::TopLevel(function) => optimize_callable_function(
                    &context.unit,
                    function as usize,
                    class_contexts,
                    &world,
                    &self.heap,
                    OptimizationConfiguration::default(),
                ),
                FunctionLocator::Method { class, method } => optimize_callable_method(
                    &context.unit,
                    class as usize,
                    method as usize,
                    class_contexts,
                    &world,
                    &self.heap,
                    OptimizationConfiguration::default(),
                ),
            }
        };
        for class in class_contexts {
            self.optimizer_class_contexts
                .insert(class.name.clone(), class);
        }

        if let Err(error) = verify(&optimized) {
            let name = self.tables.functions[position].name.clone();
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.compiler_error,
                format!(
                    "the optimized body {} failed verification: {error:?}",
                    name.to_string_lossy()
                ),
                &context.path,
            )));
        }

        // SAFETY: the surrounding invariant proves this option contains a value.
        let storage = unsafe {
            unwrap_option_invariant(
                self.tables.functions[position].optimized_chunk.as_mut(),
                "an optimizing callable owns stable chunk storage",
            )
        };
        **storage = optimized;
        self.tables.functions[position].optimization = CallableOptimization::Complete;

        let runtime = &self.tables.functions[position];
        if let Err(name) = prelink_exact_function_cache(
            &runtime.cache,
            // SAFETY: the table and prior lookup prove this pointer or index.
            unsafe { runtime.chunk.as_ref() },
            &self.tables.symbols,
            &self.tables.functions,
            &self.tables.built_in_functions,
        ) {
            return Err(self.invalid_exact_function_target(&context.path, &name));
        }

        Ok(())
    }

    fn take_optimizer_class_contexts(
        &mut self,
        context: &UnitContext,
        locator: FunctionLocator,
    ) -> Vec<CompiledClassLike> {
        let unit = &context.unit;
        let owner_class = match locator {
            FunctionLocator::TopLevel(_) => None,
            FunctionLocator::Method { class, .. } => Some(&unit.classes[class as usize]),
        };

        let destructor = context
            .optimizer_destructors
            .iter()
            .flatten()
            .map(|position| &unit.classes[*position as usize])
            .find(|candidate| owner_class.is_none_or(|owner| candidate.name != owner.name));

        owner_class
            .into_iter()
            .chain(destructor)
            .map(|class| {
                self.optimizer_class_contexts
                    .remove(&class.name)
                    .unwrap_or_else(|| class.clone())
            })
            .collect()
    }
}
