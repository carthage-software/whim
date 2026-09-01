//! Safe refinement of bytecode that has not executed yet after autoloading.

use std::ptr::NonNull;

use hashbrown::HashSet;

use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::verify::verify;
use crate::engine::declare::prelink_exact_function_cache;
use crate::optimizer::LiveRefinement;
use crate::optimizer::World;
use crate::optimizer::refine_live_chunk;
use crate::symbols::FunctionLocator;
use crate::symbols::FunctionTable;
use crate::symbols::InlineCache;
use crate::symbols::SymbolKind;
use crate::value::function::FuncId;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;

impl VirtualMachine<'_> {
    pub(in crate::vm) fn refine_current_main_tail(
        &mut self,
        floor: usize,
    ) -> Result<bool, VirtualMachineControl> {
        if !self.world_refinement_pending || self.current_frame().function.get().is_some() {
            return Ok(false);
        }

        let (chunk_pointer, unit_pointer, base) = {
            let frame = self.current_frame();
            (frame.chunk, frame.unit, frame.base as usize)
        };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { chunk_pointer.as_ref() };
        // SAFETY: the frame retains its unit context for its complete lifetime.
        let unit = unsafe { unit_pointer.as_ref() }.unit.as_ref();
        let mut names = Vec::new();
        let mut seen_names = HashSet::new();
        for instruction in &chunk.code[floor..] {
            let cache = match instruction {
                Instruction::CallNamed { cache, .. }
                | Instruction::CallNamedDiscarded { cache, .. }
                | Instruction::CallNamedUnchecked { cache, .. } => cache,
                _ => continue,
            };
            let IcDescriptor::Member { name, .. } =
                &chunk.ic_descriptors[usize::from(cache.index())]
            else {
                continue;
            };
            if seen_names.insert(name.clone()) {
                names.push(name.clone());
            }
        }

        let mut identifiers = Vec::new();
        let mut seen_identifiers = HashSet::new();
        for name in &names {
            let Some(symbol) = self.engine.tables.symbols.get(name) else {
                continue;
            };
            if symbol.kind == SymbolKind::Function && symbol.table == FunctionTable::User {
                let function = FuncId(symbol.index);
                if seen_identifiers.insert(function) {
                    identifiers.push(function);
                }
            }
        }
        if identifiers.is_empty() {
            self.world_refinement_pending = false;
            return Ok(false);
        }

        for function in &identifiers {
            self.engine.optimize_callable_once(*function)?;
        }

        let mut functions = Vec::<CompiledFunction>::new();
        for function in identifiers {
            let runtime = &self.engine.tables.functions[function.0 as usize];
            let mut compiled = match runtime.locator {
                FunctionLocator::TopLevel(position) => {
                    runtime.unit.unit.functions[position as usize].clone()
                }
                FunctionLocator::Method { .. } => continue,
            };
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            compiled.chunk = unsafe { runtime.chunk.as_ref() }.clone();
            functions.push(compiled);
        }

        let Some(window) = self.stack.len().checked_sub(base) else {
            return Ok(false);
        };
        let Ok(register_cap) = u16::try_from(window) else {
            return Ok(false);
        };
        let register_end = base + usize::from(chunk.register_count);
        let Some(registers) = self.stack.get(base..register_end) else {
            return Ok(false);
        };
        let borrowed_units;
        let world = if let Some(cache) = &self.engine.optimizer_world {
            World::from_cache(cache, &self.engine.tables.built_in_function_declarations)
        } else {
            borrowed_units = self
                .engine
                .units
                .iter()
                .map(|context| context.unit.as_ref())
                .collect::<Vec<_>>();
            World::new(
                &borrowed_units,
                &self.engine.tables.built_in_function_declarations,
            )
        };
        let refined = {
            refine_live_chunk(LiveRefinement {
                chunk,
                unit,
                registers,
                functions: &functions,
                world: &world,
                heap: &self.heap,
                floor,
                register_cap,
            })
        };
        self.world_refinement_pending = false;
        let Some(refined) = refined else {
            return Ok(false);
        };
        if verify(&refined).is_err() {
            return Ok(false);
        }

        let cache = Box::new(InlineCache::new());
        if prelink_exact_function_cache(
            &cache,
            &refined,
            &self.engine.tables.symbols,
            &self.engine.tables.functions,
            &self.engine.tables.built_in_functions,
        )
        .is_err()
        {
            return Ok(false);
        }

        let refined = Box::new(refined);
        let chunk_pointer = NonNull::from(&*refined);
        let cache_pointer = NonNull::from(&*cache);
        let reference_register_mask = refined.reference_register_mask;
        self.refined_chunks.push(refined);
        self.refined_caches.push(cache);

        let frame = self.current_frame_mut();
        frame.chunk = chunk_pointer;
        frame.cache = cache_pointer;
        frame.reference_register_mask = reference_register_mask;
        Ok(true)
    }
}
