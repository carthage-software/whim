//! Inlining of proven calls to tiny straight-line leaf functions.

use std::mem;

use hashbrown::HashMap;
use whim_span::Span;

use crate::bytecode::aliases::alias_bindings;
use crate::bytecode::aliases::substitute;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::PropertyValueMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::Visibility;
use crate::bytecode::unit::is_always_inline;
use crate::bytecode::unit::is_external;
use crate::bytecode::unit::is_never_inline;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::liveness::effect::effect_on;
use crate::optimizer::passes::inline_leaf_calls::generic::generic_call_sites;
use crate::optimizer::passes::inline_leaf_calls::generic::inline_generic_statics;
use crate::optimizer::passes::inline_leaf_calls::generic::splice_generic_sites;
use crate::optimizer::passes::inline_leaf_calls::jumping::self_inline_function;
use crate::optimizer::passes::inline_leaf_calls::leaf::LeafCallee;
use crate::optimizer::passes::inline_leaf_calls::leaf::inline_into_chunk;
use crate::optimizer::passes::inline_leaf_calls::leaf::leaf_callee;
use crate::optimizer::passes::inline_leaf_calls::methods::inline_direct_methods;
use crate::optimizer::rewrite::splice::splice_replace;
use crate::optimizer::rewrite::splice::splice_replace_many;
use crate::optimizer::type_flow::IndexedUnit;
use crate::optimizer::type_flow::TypeFlow;
use crate::optimizer::type_flow::World;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

pub(super) mod generic;
pub(in crate::optimizer) mod leaf;

mod jumping;
mod methods;

const CALLEE_INSTRUCTION_LIMIT: usize = 16;
const CALLER_CODE_LIMIT: usize = 8192;
const REGISTER_LIMIT: u16 = 192;

pub(super) struct InlineCandidates<'a> {
    local: Vec<Option<LeafCallee>>,
    local_positions: HashMap<Atom, usize>,
    donors: Vec<&'a CompiledFunction>,
    donor_callees: Vec<Option<LeafCallee>>,
    donor_positions: HashMap<Atom, usize>,
}

impl<'a> InlineCandidates<'a> {
    fn new(functions: &[CompiledFunction], donors: Vec<&'a CompiledFunction>) -> Self {
        let mut local = Vec::with_capacity(functions.len());
        let mut local_positions = HashMap::with_capacity(functions.len());
        for (position, function) in functions.iter().enumerate() {
            local.push(leaf_callee(function));
            local_positions
                .entry(function.name.clone())
                .or_insert(position);
        }

        let mut donor_callees = Vec::with_capacity(donors.len());
        let mut donor_positions = HashMap::with_capacity(donors.len());
        for (position, function) in donors.iter().enumerate() {
            donor_callees.push(leaf_callee(function));
            donor_positions
                .entry(function.name.clone())
                .or_insert(position);
        }

        Self {
            local,
            local_positions,
            donors,
            donor_callees,
            donor_positions,
        }
    }

    pub(super) fn local_position(&self, name: &Atom) -> Option<usize> {
        self.local_positions.get(name).copied()
    }

    pub(super) fn donor_position(&self, name: &Atom) -> Option<usize> {
        self.donor_positions.get(name).copied()
    }
}

/// Splices the bodies of small, straight-line, unchecked-return leaf
/// functions into their proven call sites, and reports whether any chunk
/// changed so the caller can re-run its local pipeline.
/// The chunks an inlining run modified, so only those re-enter the local
/// pipeline afterwards.
pub(in crate::optimizer) struct InlineChanges {
    pub(in crate::optimizer) main: bool,
    pub(in crate::optimizer) functions: Vec<usize>,
    pub(in crate::optimizer) methods: Vec<(usize, usize)>,
}

impl InlineChanges {
    pub(in crate::optimizer) fn any(&self) -> bool {
        self.main || !self.functions.is_empty() || !self.methods.is_empty()
    }

    fn mark(&mut self, location: Location) {
        match location {
            Location::Main => self.main = true,
            Location::Function(position) => {
                if !self.functions.contains(&position) {
                    self.functions.push(position);
                }
            }
            Location::Method { class, method } => {
                if !self.methods.contains(&(class, method)) {
                    self.methods.push((class, method));
                }
            }
        }
    }
}

pub(in crate::optimizer) fn optimize_unit(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    heap: &Heap,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) -> InlineChanges {
    let mut changes = InlineChanges {
        main: false,
        functions: Vec::new(),
        methods: Vec::new(),
    };

    if !configuration.inline_leaf_calls {
        return changes;
    }

    let function_count = unit.functions.len();
    let function_floor = configuration.function_floor(function_count);
    for (position, function) in unit.functions.iter_mut().enumerate().skip(function_floor) {
        if !is_external(&function.attributes)
            && !is_never_inline(&function.attributes)
            && self_inline_function(function, statistics)
        {
            changes.mark(Location::Function(position));
        }
    }

    // A world unit's body is already optimized and belongs to someone else:
    // only one it explicitly offers for inlining is spliced from it.
    let candidates = InlineCandidates::new(
        &unit.functions,
        world
            .units()
            .flat_map(|other| other.functions.iter())
            .filter(|function| {
                is_always_inline(&function.attributes) && !is_external(&function.attributes)
            })
            .collect(),
    );

    if inline_into_chunk(
        &mut unit.main,
        &unit.functions,
        &candidates,
        None,
        statistics,
    ) {
        changes.mark(Location::Main);
    }

    for caller in function_floor..function_count {
        let mut caller_chunk = mem::take(&mut unit.functions[caller].chunk);
        let inlined = inline_into_chunk(
            &mut caller_chunk,
            &unit.functions,
            &candidates,
            Some(caller),
            statistics,
        );

        unit.functions[caller].chunk = caller_chunk;
        if inlined {
            changes.mark(Location::Function(caller));
        }
    }

    let class_floor = configuration.class_floor(unit.classes.len());
    for (class_position, class) in unit.classes.iter_mut().enumerate().skip(class_floor) {
        for (method_position, method) in class.methods.iter_mut().enumerate() {
            if inline_into_chunk(
                &mut method.function.chunk,
                &unit.functions,
                &candidates,
                None,
                statistics,
            ) {
                changes.mark(Location::Method {
                    class: class_position,
                    method: method_position,
                });
            }
        }
    }

    for location in inline_direct_methods(unit, world, heap, configuration, statistics) {
        changes.mark(location);
    }

    for location in inline_generic_statics(unit, world, heap, configuration, statistics) {
        changes.mark(location);
    }

    if unit
        .functions
        .iter()
        .any(|function| !function.type_parameters.is_empty())
        && unit
            .main
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallNamed { .. }))
    {
        let sites = {
            let indexed = IndexedUnit::with_world(unit, world);
            let flow =
                TypeFlow::analyze_with_unit(&indexed.main, &[], false, None, &[], &indexed, heap);
            generic_call_sites(&indexed.main, &indexed.functions, &candidates, &flow, 0)
        };

        if splice_generic_sites(
            &mut unit.main,
            &unit.functions,
            &sites,
            REGISTER_LIMIT,
            statistics,
        ) {
            changes.mark(Location::Main);
        }
    }

    changes
}

struct MethodSite {
    location: Location,
    index: usize,
    class: usize,
    method: usize,
    discarded: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::optimizer) enum Location {
    Main,
    Function(usize),
    Method { class: usize, method: usize },
}
