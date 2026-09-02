//! One analysis of a unit, shared by every pass that reads it.

use std::collections::VecDeque;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::passes::FunctionLocation;
use crate::optimizer::rewrite::plan::RewritePlan;
use crate::optimizer::type_flow::IndexedUnit;
use crate::optimizer::type_flow::TypeFlow;
use crate::optimizer::type_flow::TypeFlowOptions;
use crate::optimizer::type_flow::World;
use crate::optimizer::type_flow::descriptor_options_equal;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

/// One chunk of the imaged unit, with the facts that hold before each of its
/// instructions.
pub(in crate::optimizer) struct AnalyzedChunk<'a> {
    pub(in crate::optimizer) position: usize,
    pub(in crate::optimizer) location: FunctionLocation,
    pub(in crate::optimizer) chunk: &'a Chunk,
    pub(in crate::optimizer) class_name: Option<&'a Atom>,
    /// The name of the function this chunk is the body of, for a plain
    /// top-level function; a method or a top level has none.
    pub(in crate::optimizer) function_name: Option<&'a Atom>,
    /// Whether register zero holds a receiver.
    pub(in crate::optimizer) has_receiver: bool,
    pub(in crate::optimizer) return_type: Option<&'a TypeDescriptor>,
    pub(in crate::optimizer) candidates: CandidateSet,
    pub(in crate::optimizer) flow: TypeFlow<'a>,
}

/// Every analyzed chunk of one unit.
pub(in crate::optimizer) struct Analysis<'a> {
    chunks: Vec<AnalyzedChunk<'a>>,
}

struct CaptureProducer<'a> {
    chunk: &'a Chunk,
    parameters: &'a [CompiledParameter],
    has_receiver: bool,
    class_name: Option<&'a Atom>,
    class_type_parameters: &'a [CompiledTypeParameter],
    source: CaptureSource<'a>,
}

#[derive(Clone, Copy)]
enum CaptureSource<'a> {
    Function(usize),
    Fixed(&'a [Option<TypeDescriptor>]),
}

#[derive(Clone, Copy)]
struct CaptureSite {
    instruction: usize,
    function: usize,
    first_capture: Register,
    skip: usize,
    count: usize,
}

struct CaptureGraph {
    sites: Vec<CaptureSite>,
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
}

impl<'a> Analysis<'a> {
    /// Analyzes every chunk of `indexed` that the pipeline may rewrite.
    pub(in crate::optimizer) fn of(
        indexed: &'a IndexedUnit<'a>,
        configuration: OptimizationConfiguration,
        heap: &'a Heap,
    ) -> Self {
        Self::of_required(indexed, configuration, heap, None)
    }

    /// Analyzes only chunks that still hold generic operations needed by local passes.
    pub(in crate::optimizer) fn of_early_operations(
        indexed: &'a IndexedUnit<'a>,
        configuration: OptimizationConfiguration,
        heap: &'a Heap,
    ) -> Self {
        Self::of_required(
            indexed,
            configuration,
            heap,
            Some(CandidateSet::EARLY_OPERATION),
        )
    }

    fn of_required(
        indexed: &'a IndexedUnit<'a>,
        configuration: OptimizationConfiguration,
        heap: &'a Heap,
        required: Option<CandidateSet>,
    ) -> Self {
        let image: &CompiledUnit = indexed;
        let mut chunks = Vec::with_capacity(1 + image.functions.len());
        let analyze = |location,
                       chunk: &'a Chunk,
                       parameters: &'a [CompiledParameter],
                       has_receiver,
                       class_name,
                       function_name,
                       return_type,
                       class_type_parameters: &'a [CompiledTypeParameter],
                       capture_types: Vec<Option<TypeDescriptor>>| {
            if chunk.code.is_empty() {
                return None;
            }

            let candidates = CandidateSet::of(chunk, configuration);
            let carries_capture = chunk.code.iter().any(|instruction| {
                matches!(instruction, Instruction::MakeClosure { capture_count, .. } if capture_count.value() != 0)
            });

            if !carries_capture
                && (candidates.is_empty()
                    || required.is_some_and(|required| !candidates.contains(required)))
            {
                return None;
            }

            Some(AnalyzedChunk {
                position: 0,
                location,
                chunk,
                class_name,
                function_name,
                has_receiver,
                return_type,
                candidates,
                flow: TypeFlow::analyze_with_unit_options(
                    chunk,
                    parameters,
                    indexed,
                    heap,
                    TypeFlowOptions {
                        has_receiver,
                        class_name,
                        class_type_parameters,
                        track_array_elements: candidates.needs_array_elements(),
                        cache_constants: candidates.needs_constant_cache(),
                        capture_types,
                    },
                ),
            })
        };

        chunks.extend(analyze(
            FunctionLocation::Main,
            &image.main,
            &[],
            false,
            None,
            None,
            None,
            &[],
            Vec::new(),
        ));

        for (index, function) in image
            .functions
            .iter()
            .enumerate()
            .skip(configuration.function_floor(image.functions.len()))
        {
            chunks.extend(analyze(
                FunctionLocation::Function(index),
                &function.chunk,
                &function.parameters,
                function.captures_this,
                None,
                Some(&function.name),
                function.return_type.as_ref(),
                &[],
                function.capture_types.clone(),
            ));
        }

        for (class, compiled) in image
            .classes
            .iter()
            .enumerate()
            .skip(configuration.class_floor(image.classes.len()))
        {
            for (method, compiled_method) in compiled.methods.iter().enumerate() {
                chunks.extend(analyze(
                    FunctionLocation::Method { class, method },
                    &compiled_method.function.chunk,
                    &compiled_method.function.parameters,
                    !compiled_method.is_static || compiled_method.function.captures_this,
                    Some(&compiled.name),
                    None,
                    compiled_method.function.return_type.as_ref(),
                    &compiled.type_parameters,
                    compiled_method.function.capture_types.clone(),
                ));
            }
        }

        for (position, chunk) in chunks.iter_mut().enumerate() {
            chunk.position = position;
        }

        refine_capture_types(&mut chunks, indexed, heap);

        Self { chunks }
    }

    pub(in crate::optimizer) fn chunks(&self) -> &[AnalyzedChunk<'a>] {
        &self.chunks
    }
}

pub(in crate::optimizer) fn annotate_capture_types(
    unit: &mut CompiledUnit,
    world: &World<'_>,
    heap: &Heap,
) {
    if !unit_contains_captures(unit) {
        return;
    }

    let inferred = {
        let indexed = IndexedUnit::with_world(unit, world);
        infer_capture_types(&indexed, heap)
    };
    for (function, inferred) in unit.functions.iter_mut().zip(inferred) {
        if let Some(inferred) = inferred {
            function.capture_types = inferred;
        }
    }
}

fn infer_capture_types<'a>(
    indexed: &'a IndexedUnit<'a>,
    heap: &'a Heap,
) -> Vec<Option<Vec<Option<TypeDescriptor>>>> {
    let unit: &CompiledUnit = indexed;
    let mut known: Vec<Option<Vec<Option<TypeDescriptor>>>> = unit
        .functions
        .iter()
        .map(|function| {
            (!function.capture_types.is_empty()).then(|| function.capture_types.clone())
        })
        .collect();

    let producers = capture_producers(indexed);
    let mut graph = CaptureGraph::new(producers.len(), unit.functions.len());
    let mut function_producers = vec![None; unit.functions.len()];
    for (position, producer) in producers.iter().enumerate() {
        graph.add(position, producer.chunk, indexed);
        if let CaptureSource::Function(function) = producer.source {
            function_producers[function] = Some(position);
        }
    }

    let mut contributions = vec![None; graph.sites.len()];
    let mut pending = VecDeque::from_iter(0..producers.len());
    let mut queued = vec![true; producers.len()];
    let mut dirty = vec![false; unit.functions.len()];
    let mut changed = Vec::new();
    while let Some(position) = pending.pop_front() {
        queued[position] = false;
        let producer = &producers[position];
        let capture_types = match producer.source {
            CaptureSource::Function(function) => known[function].clone().unwrap_or_default(),
            CaptureSource::Fixed(types) => types.to_vec(),
        };

        let flow = TypeFlow::analyze_with_unit_options(
            producer.chunk,
            producer.parameters,
            indexed,
            heap,
            TypeFlowOptions {
                has_receiver: producer.has_receiver,
                class_name: producer.class_name,
                class_type_parameters: producer.class_type_parameters,
                track_array_elements: false,
                cache_constants: false,
                capture_types,
            },
        );

        graph.update(
            position,
            &flow,
            &mut contributions,
            &mut dirty,
            &mut changed,
        );

        for function in changed.iter().copied() {
            let inferred = graph.merged(function, &contributions);
            if capture_inference_equal(&known[function], &inferred) {
                continue;
            }

            known[function] = inferred;
            if let Some(producer) = function_producers[function]
                && !queued[producer]
            {
                pending.push_back(producer);
                queued[producer] = true;
            }
        }
    }

    known
}

fn capture_producers<'a>(indexed: &'a IndexedUnit<'a>) -> Vec<CaptureProducer<'a>> {
    let unit: &CompiledUnit = indexed;
    let mut producers = Vec::new();
    let mut add = |producer: CaptureProducer<'a>| {
        if chunk_contains_captures(producer.chunk) {
            producers.push(producer);
        }
    };

    add(CaptureProducer {
        chunk: &unit.main,
        parameters: &[],
        has_receiver: false,
        class_name: None,
        class_type_parameters: &[],
        source: CaptureSource::Fixed(&[]),
    });

    for (function, compiled) in unit.functions.iter().enumerate() {
        add(CaptureProducer {
            chunk: &compiled.chunk,
            parameters: &compiled.parameters,
            has_receiver: compiled.captures_this,
            class_name: None,
            class_type_parameters: &[],
            source: CaptureSource::Function(function),
        });
    }

    for class in &unit.classes {
        for method in &class.methods {
            add(CaptureProducer {
                chunk: &method.function.chunk,
                parameters: &method.function.parameters,
                has_receiver: !method.is_static || method.function.captures_this,
                class_name: Some(&class.name),
                class_type_parameters: &class.type_parameters,
                source: CaptureSource::Fixed(&method.function.capture_types),
            });
        }
    }

    producers
}

impl CaptureGraph {
    fn new(producers: usize, functions: usize) -> Self {
        Self {
            sites: Vec::new(),
            outgoing: vec![Vec::new(); producers],
            incoming: vec![Vec::new(); functions],
        }
    }

    fn add(&mut self, producer: usize, chunk: &Chunk, indexed: &IndexedUnit<'_>) {
        for (instruction, candidate) in chunk.code.iter().copied().enumerate() {
            let Instruction::MakeClosure {
                capture_count,
                prototype,
                first_capture,
                ..
            } = candidate
            else {
                continue;
            };

            let Some(Literal::String(name)) = chunk.constants.get(usize::from(prototype.index()))
            else {
                continue;
            };

            let Some(function) = indexed.local_function_index(name) else {
                continue;
            };

            let skip = usize::from(indexed.functions[function].captures_this);
            let count = usize::from(capture_count.value());
            if count < skip {
                continue;
            }

            let site = self.sites.len();
            self.sites.push(CaptureSite {
                instruction,
                function,
                first_capture,
                skip,
                count,
            });
            self.outgoing[producer].push(site);
            self.incoming[function].push(site);
        }
    }

    fn update(
        &self,
        producer: usize,
        flow: &TypeFlow<'_>,
        contributions: &mut [Option<Vec<Option<TypeDescriptor>>>],
        dirty: &mut [bool],
        changed: &mut Vec<usize>,
    ) {
        changed.clear();
        for &site_index in &self.outgoing[producer] {
            let site = self.sites[site_index];
            let mut types = Vec::with_capacity(site.count - site.skip);
            for offset in site.skip..site.count {
                let descriptor = site
                    .first_capture
                    .index()
                    .checked_add(offset as u16)
                    .map(Register::new)
                    .and_then(|register| flow.register_type_at(site.instruction, register, 0));
                types.push(descriptor);
            }

            if contributions[site_index]
                .as_ref()
                .is_some_and(|existing| capture_types_equal(existing, &types))
            {
                continue;
            }

            contributions[site_index] = Some(types);
            if !dirty[site.function] {
                dirty[site.function] = true;
                changed.push(site.function);
            }
        }
        for function in changed.iter().copied() {
            dirty[function] = false;
        }
    }

    fn merged(
        &self,
        function: usize,
        contributions: &[Option<Vec<Option<TypeDescriptor>>>],
    ) -> Option<Vec<Option<TypeDescriptor>>> {
        let mut merged = None;
        for &site in &self.incoming[function] {
            if let Some(candidate) = &contributions[site] {
                merge_capture_types(&mut merged, candidate);
            }
        }

        merged
    }
}

fn unit_contains_captures(unit: &CompiledUnit) -> bool {
    chunk_contains_captures(&unit.main)
        || unit
            .functions
            .iter()
            .any(|function| chunk_contains_captures(&function.chunk))
        || unit.classes.iter().any(|class| {
            class
                .methods
                .iter()
                .any(|method| chunk_contains_captures(&method.function.chunk))
        })
}

fn chunk_contains_captures(chunk: &Chunk) -> bool {
    chunk.code.iter().any(|instruction| {
        matches!(instruction, Instruction::MakeClosure { capture_count, .. } if capture_count.value() != 0)
    })
}

fn refine_capture_types<'a>(
    chunks: &mut [AnalyzedChunk<'a>],
    indexed: &'a IndexedUnit<'a>,
    heap: &'a Heap,
) {
    let image: &CompiledUnit = indexed;
    let mut graph = CaptureGraph::new(chunks.len(), image.functions.len());
    let mut function_positions = vec![None; image.functions.len()];
    for (position, analyzed) in chunks.iter().enumerate() {
        graph.add(position, analyzed.chunk, indexed);
        if let FunctionLocation::Function(function) = analyzed.location {
            function_positions[function] = Some(position);
        }
    }

    let mut contributions = vec![None; graph.sites.len()];
    let mut pending = VecDeque::new();
    let mut queued = vec![false; chunks.len()];
    for (position, outgoing) in graph.outgoing.iter().enumerate() {
        if !outgoing.is_empty() {
            pending.push_back(position);
            queued[position] = true;
        }
    }

    let mut dirty = vec![false; image.functions.len()];
    let mut changed = Vec::new();
    while let Some(position) = pending.pop_front() {
        queued[position] = false;
        graph.update(
            position,
            &chunks[position].flow,
            &mut contributions,
            &mut dirty,
            &mut changed,
        );

        for function in changed.iter().copied() {
            let Some(capture_types) = graph.merged(function, &contributions) else {
                continue;
            };

            let Some(target) = function_positions[function] else {
                continue;
            };

            if capture_types_equal(chunks[target].flow.capture_types(), &capture_types) {
                continue;
            }

            let compiled = &image.functions[function];
            let class_name = chunks[target].class_name;
            let candidates = chunks[target].candidates;
            chunks[target].flow = TypeFlow::analyze_with_unit_options(
                &compiled.chunk,
                &compiled.parameters,
                indexed,
                heap,
                TypeFlowOptions {
                    has_receiver: compiled.captures_this,
                    class_name,
                    class_type_parameters: &[],
                    track_array_elements: candidates.needs_array_elements(),
                    cache_constants: candidates.needs_constant_cache(),
                    capture_types,
                },
            );

            if !graph.outgoing[target].is_empty() && !queued[target] {
                pending.push_back(target);
                queued[target] = true;
            }
        }
    }
}

fn merge_capture_types(
    inferred: &mut Option<Vec<Option<TypeDescriptor>>>,
    candidate: &[Option<TypeDescriptor>],
) {
    let Some(existing) = inferred else {
        *inferred = Some(candidate.to_vec());
        return;
    };

    if existing.len() != candidate.len() {
        existing.clear();
        return;
    }

    for (existing, candidate) in existing.iter_mut().zip(candidate) {
        if !descriptor_options_equal(existing.as_ref(), candidate.as_ref(), 0) {
            *existing = None;
        }
    }
}

fn capture_types_equal(left: &[Option<TypeDescriptor>], right: &[Option<TypeDescriptor>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| descriptor_options_equal(left.as_ref(), right.as_ref(), 0))
}

fn capture_inference_equal(
    left: &Option<Vec<Option<TypeDescriptor>>>,
    right: &Option<Vec<Option<TypeDescriptor>>>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => capture_types_equal(left, right),
        (None, None) => true,
        _ => false,
    }
}

impl AnalyzedChunk<'_> {
    /// Writes an instruction into the live unit, unless an earlier pass in
    /// this round already replaced the one the analysis describes. Reports
    /// whether it landed.
    pub(in crate::optimizer) fn write(
        &self,
        plan: &mut RewritePlan,
        index: usize,
        instruction: Instruction,
    ) -> bool {
        plan.replace(self, index, instruction)
    }
}
