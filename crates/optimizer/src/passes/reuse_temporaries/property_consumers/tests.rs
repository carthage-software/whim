use std::ptr;

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::CatchEntry;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayValueMode;
use whim_bytecode::instruction::operands::ImmediateInteger;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::PropertyReadMode;
use whim_bytecode::instruction::operands::PropertySlot;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::verify::verify_unit;
use whim_compiler::CompileConfiguration;
use whim_compiler::compile_with_configuration;
use whim_span::Span;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;
use whim_value::heap::Heap;

use super::optimize_unit;
use crate::OptimizationConfiguration;
use crate::analysis::Analysis;
use crate::candidates::CandidateSet;
use crate::type_flow::IndexedUnit;
use crate::type_flow::World;

const OBJECT: Register = Register::new(0);
const PREVIOUS: Register = Register::new(1);
const POSITION: Register = Register::new(2);
const CONTAINER: Register = Register::new(3);
const RESULT: Register = Register::new(4);
const COPY: Register = Register::new(5);

fn fixture(heap: &Heap, property: &str, indexed: bool) -> CompiledUnit {
    let source = format!(
        "final class Store {{ public {property} $items; public function read(mixed $previous, uint $position): mixed {{ return null; }} }}"
    );
    let arena = LocalArena::new();
    let program = parse(&arena, &source).unwrap();
    let mut configuration = CompileConfiguration::default();
    configuration.optimization.enabled = false;
    let mut unit =
        compile_with_configuration(program, "/property-consumers.whim", heap, configuration)
            .unwrap();
    let mut chunk = Chunk::new();
    chunk.register_count = 6;
    chunk.local_register_count = 3;
    chunk.parameter_register_start = 1;
    chunk.parameter_register_count = 2;
    chunk.reference_register_mask = 0b11_1111;
    for instruction in [
        Instruction::PropertyGetUnchecked {
            destination: CONTAINER,
            object: OBJECT,
            slot: PropertySlot::new(0),
            value_mode: PropertyReadMode::Clone,
        },
        if indexed {
            Instruction::VecIndexGet {
                destination: RESULT,
                container: CONTAINER,
                index: POSITION,
                value_mode: ArrayValueMode::Generic,
            }
        } else {
            Instruction::Length {
                destination: RESULT,
                source: CONTAINER,
            }
        },
        Instruction::ReturnUnchecked { source: RESULT },
    ] {
        chunk.emit(instruction, Span::zero());
    }
    *chunk_mut(&mut unit) = chunk;
    unit
}

fn chunk_mut(unit: &mut CompiledUnit) -> &mut Chunk {
    &mut unit.classes[0]
        .methods
        .iter_mut()
        .find(|method| method.name.as_bytes() == b"read")
        .unwrap()
        .function
        .chunk
}

fn rewrite(unit: &mut CompiledUnit, heap: &Heap) {
    verify_unit(unit).unwrap();
    optimize_unit(
        unit,
        &World::new(&[], &[]),
        heap,
        OptimizationConfiguration::default(),
    );
    verify_unit(unit).unwrap();
}

fn insert(chunk: &mut Chunk, index: usize, instruction: Instruction) {
    chunk.code.insert(index, instruction);
    chunk.spans.insert(index, Span::zero());
}

fn local_fixture(heap: &Heap, property: &str, indexed: bool) -> CompiledUnit {
    let mut unit = fixture(heap, property, indexed);
    let chunk = chunk_mut(&mut unit);
    chunk.local_register_count = 5;
    let Instruction::PropertyGetUnchecked { destination, .. } = &mut chunk.code[0] else {
        unreachable!()
    };
    *destination = COPY;
    match &mut chunk.code[1] {
        Instruction::Length { source, .. } => *source = COPY,
        Instruction::VecIndexGet { container, .. } => *container = COPY,
        _ => unreachable!(),
    }
    insert(
        chunk,
        0,
        Instruction::LoadNull {
            destination: RESULT,
        },
    );
    unit
}

fn first_assignment_fixture(heap: &Heap, property: &str) -> CompiledUnit {
    let mut unit = local_fixture(heap, property, false);
    let chunk = chunk_mut(&mut unit);
    chunk.code.remove(0);
    chunk.spans.remove(0);
    unit
}

fn assert_unknown_length_destination(
    unit: &CompiledUnit,
    chunk: &Chunk,
    index: usize,
    heap: &Heap,
) {
    let world = World::new(&[], &[]);
    let indexed = IndexedUnit::with_world(unit, &world);
    let analysis =
        Analysis::of_property_consumers(&indexed, OptimizationConfiguration::default(), heap);
    let analyzed = analysis
        .chunks()
        .iter()
        .find(|analyzed| ptr::eq(analyzed.chunk, chunk))
        .unwrap();
    assert!(analyzed.flow.register_may_release_observably(index, RESULT));
    assert!(analyzed.flow.proves_collection(index, COPY));
}

fn integer_load(destination: Register) -> Instruction {
    Instruction::LoadInteger {
        destination,
        immediate: ImmediateInteger::unsigned(0),
        kind: IntegerKind::U64,
    }
}

fn literal_fixture(heap: &Heap) -> CompiledUnit {
    let mut unit = fixture(heap, "vec<(int, uint)>", true);
    let chunk = chunk_mut(&mut unit);
    let Instruction::VecIndexGet { index, .. } = &mut chunk.code[1] else {
        unreachable!()
    };
    *index = COPY;
    insert(chunk, 1, integer_load(COPY));
    unit
}

#[test]
fn adjacent_scalar_collections_consume_the_property_copy() {
    let heap = Heap::new();
    for property in ["vec<int>", "vec<(int, uint)>", "vec<vec<string>>"] {
        for indexed in [false, true] {
            let mut unit = fixture(&heap, property, indexed);
            rewrite(&mut unit, &heap);
            let code = &chunk_mut(&mut unit).code;
            assert!(
                matches!(
                    code[0],
                    Instruction::PropertyGetUnchecked {
                        destination: RESULT,
                        ..
                    }
                ),
                "{property}: {code:?}"
            );
            assert!(
                matches!(
                    code[1],
                    Instruction::Length {
                        destination: RESULT,
                        source: RESULT
                    } | Instruction::VecIndexGet {
                        destination: RESULT,
                        container: RESULT,
                        ..
                    }
                ),
                "{property}: {code:?}"
            );
        }
    }
}

#[test]
fn each_observable_release_blocks_reuse() {
    let heap = Heap::new();
    for indexed in [false, true] {
        for previous in [CONTAINER, RESULT] {
            let mut unit = fixture(&heap, "vec<(int, uint)>", indexed);
            insert(
                chunk_mut(&mut unit),
                0,
                Instruction::Move {
                    destination: previous,
                    source: PREVIOUS,
                },
            );
            let before = chunk_mut(&mut unit).code.clone();
            rewrite(&mut unit, &heap);
            assert_eq!(chunk_mut(&mut unit).code, before);
        }
        for property in [
            "vec",
            "vec<object>",
            "vec<fn(): void>",
            "vec<(int, object)>",
        ] {
            let mut unit = fixture(&heap, property, indexed);
            let before = chunk_mut(&mut unit).code.clone();
            rewrite(&mut unit, &heap);
            assert_eq!(chunk_mut(&mut unit).code, before, "{property}");
        }
    }
}

#[test]
fn live_local_and_trace_registers_keep_their_values() {
    let heap = Heap::new();
    for indexed in [false, true] {
        for protected in [CONTAINER, RESULT] {
            let mut unit = fixture(&heap, "vec<int>", indexed);
            let chunk = chunk_mut(&mut unit);
            chunk.local_register_count = protected.index() + 1;
            chunk.trace_argument_registers = vec![protected, Register::NONE];
            let before = chunk_mut(&mut unit).code.clone();
            rewrite(&mut unit, &heap);
            assert_eq!(chunk_mut(&mut unit).code, before);
        }
        let mut unit = fixture(&heap, "vec<int>", indexed);
        chunk_mut(&mut unit).local_register_count = 5;
        let before = chunk_mut(&mut unit).code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);

        let mut unit = fixture(&heap, "vec<int>", indexed);
        insert(
            chunk_mut(&mut unit),
            2,
            Instruction::Move {
                destination: COPY,
                source: CONTAINER,
            },
        );
        let before = chunk_mut(&mut unit).code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);
    }
}

#[test]
fn branches_catches_and_nonadjacent_reads_keep_the_pair() {
    let heap = Heap::new();
    for indexed in [false, true] {
        let mut unit = fixture(&heap, "vec<int>", indexed);
        insert(
            chunk_mut(&mut unit),
            2,
            Instruction::JumpIfTrue {
                condition: PREVIOUS,
                offset: JumpOffset::new(-1),
            },
        );
        let before = chunk_mut(&mut unit).code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);

        let mut unit = fixture(&heap, "vec<int>", indexed);
        let chunk = chunk_mut(&mut unit);
        let type_descriptor = chunk.add_type_descriptor(TypeDescriptor::Mixed).unwrap();
        chunk.catch_table.push(CatchEntry {
            start: 0,
            end: 2,
            handler: 2,
            type_descriptor,
            temporary_floor: 3,
            binding: None,
        });
        let before = chunk.code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);

        let mut unit = fixture(&heap, "vec<int>", indexed);
        insert(
            chunk_mut(&mut unit),
            1,
            Instruction::LoadNull { destination: COPY },
        );
        let before = chunk_mut(&mut unit).code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);
    }
}

#[test]
fn aliased_index_and_receiver_inputs_are_not_overwritten() {
    let heap = Heap::new();
    let mut unit = fixture(&heap, "vec<int>", true);
    let chunk = chunk_mut(&mut unit);
    insert(
        chunk,
        0,
        Instruction::Move {
            destination: RESULT,
            source: POSITION,
        },
    );
    let Instruction::VecIndexGet { index, .. } = &mut chunk.code[2] else {
        unreachable!()
    };
    *index = RESULT;
    let before = chunk.code.clone();
    rewrite(&mut unit, &heap);
    assert_eq!(chunk_mut(&mut unit).code, before);

    let mut unit = fixture(&heap, "vec<int>", false);
    let chunk = chunk_mut(&mut unit);
    insert(
        chunk,
        0,
        Instruction::Move {
            destination: RESULT,
            source: OBJECT,
        },
    );
    let Instruction::PropertyGetUnchecked { object, .. } = &mut chunk.code[1] else {
        unreachable!()
    };
    *object = RESULT;
    let before = chunk.code.clone();
    rewrite(&mut unit, &heap);
    assert_eq!(chunk_mut(&mut unit).code, before);
}

#[test]
fn adjacent_collection_lengths_can_replace_nonincoming_locals() {
    let heap = Heap::new();
    for property in [
        "vec<int>",
        "dict<string, int>",
        "(int, uint)",
        "vec<int>|dict<string, int>",
    ] {
        let mut unit = local_fixture(&heap, property, false);
        rewrite(&mut unit, &heap);
        let code = &chunk_mut(&mut unit).code;
        assert!(
            matches!(
                code[1],
                Instruction::PropertyGetUnchecked {
                    destination: RESULT,
                    ..
                }
            ),
            "{property}: {code:?}"
        );
        assert!(
            matches!(
                code[2],
                Instruction::Length {
                    destination: RESULT,
                    source: RESULT
                }
            ),
            "{property}: {code:?}"
        );
    }
}

#[test]
fn first_collection_lengths_can_initialize_method_and_function_locals() {
    let heap = Heap::new();
    for property in [
        "vec<int>",
        "dict<string, int>",
        "(int, uint)",
        "vec<int>|dict<string, int>",
    ] {
        let mut unit = first_assignment_fixture(&heap, property);
        let chunk = &unit.classes[0]
            .methods
            .iter()
            .find(|method| method.name.as_bytes() == b"read")
            .unwrap()
            .function
            .chunk;
        assert_unknown_length_destination(&unit, chunk, 1, &heap);
        rewrite(&mut unit, &heap);
        let code = &chunk_mut(&mut unit).code;
        assert!(
            matches!(
                code[0],
                Instruction::PropertyGetUnchecked {
                    destination: RESULT,
                    ..
                }
            ),
            "{property}: {code:?}"
        );
        assert_eq!(
            code[1],
            Instruction::Length {
                destination: RESULT,
                source: RESULT
            }
        );
    }

    let mut unit = first_assignment_fixture(&heap, "vec<int>");
    let arena = LocalArena::new();
    let program = parse(
        &arena,
        "function read(Store $store, mixed $previous, uint $position): mixed { return null; }",
    )
    .unwrap();
    let mut configuration = CompileConfiguration::default();
    configuration.optimization.enabled = false;
    let mut functions =
        compile_with_configuration(program, "/first-local.whim", &heap, configuration).unwrap();
    let mut function = functions.functions.remove(0);
    function.chunk = chunk_mut(&mut unit).clone();
    function.chunk.parameter_register_start = 0;
    function.chunk.parameter_register_count = 3;
    unit.functions.push(function);
    assert_unknown_length_destination(&unit, &unit.functions[0].chunk, 1, &heap);
    rewrite(&mut unit, &heap);
    assert!(matches!(
        unit.functions[0].chunk.code[0],
        Instruction::PropertyGetUnchecked {
            destination: RESULT,
            ..
        }
    ));
    assert_eq!(
        unit.functions[0].chunk.code[1],
        Instruction::Length {
            destination: RESULT,
            source: RESULT
        }
    );
}

#[test]
fn first_assignment_keeps_main_parameters_and_trace_locals() {
    let heap = Heap::new();
    let mut unit = first_assignment_fixture(&heap, "vec<int>");
    unit.main = chunk_mut(&mut unit).clone();
    unit.main.parameter_register_start = 0;
    unit.main.parameter_register_count = 0;
    let descriptor = unit
        .main
        .add_type_descriptor(TypeDescriptor::Named {
            name: unit.classes[0].name.clone(),
            arguments: None,
            recursive: false,
        })
        .unwrap();
    insert(
        &mut unit.main,
        0,
        Instruction::NewTyped {
            destination: OBJECT,
            descriptor,
        },
    );
    assert_unknown_length_destination(&unit, &unit.main, 2, &heap);
    let before = unit.main.code.clone();
    rewrite(&mut unit, &heap);
    assert_eq!(unit.main.code, before);

    for protected in [PREVIOUS, POSITION, RESULT] {
        let mut unit = first_assignment_fixture(&heap, "vec<int>");
        let chunk = chunk_mut(&mut unit);
        if protected == RESULT {
            chunk.trace_argument_registers = vec![RESULT, Register::NONE];
        }
        chunk.code[1] = Instruction::Length {
            destination: protected,
            source: COPY,
        };
        chunk.code[2] = Instruction::ReturnUnchecked { source: protected };
        let before = chunk.code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);
    }
}

#[test]
fn first_assignment_requires_an_untouched_straight_line_local() {
    let heap = Heap::new();
    for prior in [
        Instruction::Move {
            destination: CONTAINER,
            source: RESULT,
        },
        Instruction::Move {
            destination: RESULT,
            source: PREVIOUS,
        },
        Instruction::Jump {
            offset: JumpOffset::new(1),
        },
    ] {
        let mut unit = first_assignment_fixture(&heap, "vec<int>");
        insert(chunk_mut(&mut unit), 0, prior);
        let before = chunk_mut(&mut unit).code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before, "{prior:?}");
    }
    for target in [0, 1] {
        let mut unit = first_assignment_fixture(&heap, "vec<int>");
        let chunk = chunk_mut(&mut unit);
        insert(
            chunk,
            0,
            Instruction::LoadNull {
                destination: CONTAINER,
            },
        );
        insert(
            chunk,
            3,
            Instruction::JumpIfTrue {
                condition: PREVIOUS,
                offset: JumpOffset::new(target - 3),
            },
        );
        let before = chunk.code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);
    }
}

#[test]
fn locals_reject_throwing_consumers_and_protected_or_observable_values() {
    let heap = Heap::new();
    for (property, indexed) in [
        ("vec<int>", true),
        ("vec<int>|bool", false),
        ("string", false),
        ("mixed", false),
    ] {
        let mut unit = local_fixture(&heap, property, indexed);
        let before = chunk_mut(&mut unit).code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before, "{property}, {indexed}");
    }
    for protected in [PREVIOUS, POSITION, RESULT] {
        let mut unit = local_fixture(&heap, "vec<int>", false);
        let chunk = chunk_mut(&mut unit);
        if protected == RESULT {
            chunk.trace_argument_registers = vec![RESULT, Register::NONE];
        }
        chunk.code[0] = Instruction::LoadNull {
            destination: protected,
        };
        chunk.code[2] = Instruction::Length {
            destination: protected,
            source: COPY,
        };
        chunk.code[3] = Instruction::ReturnUnchecked { source: protected };
        let before = chunk.code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);
    }
    let mut unit = local_fixture(&heap, "vec<int>", false);
    chunk_mut(&mut unit).code[0] = Instruction::Move {
        destination: RESULT,
        source: PREVIOUS,
    };
    let before = chunk_mut(&mut unit).code.clone();
    rewrite(&mut unit, &heap);
    assert_eq!(chunk_mut(&mut unit).code, before);

    let mut unit = local_fixture(&heap, "vec<int>", false);
    insert(chunk_mut(&mut unit), 2, integer_load(CONTAINER));
    let before = chunk_mut(&mut unit).code.clone();
    rewrite(&mut unit, &heap);
    assert_eq!(chunk_mut(&mut unit).code, before);
}

#[test]
fn one_integer_index_load_keeps_its_value_and_consumes_the_copy() {
    let heap = Heap::new();
    let mut unit = literal_fixture(&heap);
    rewrite(&mut unit, &heap);
    let code = &chunk_mut(&mut unit).code;
    assert!(matches!(
        code[0],
        Instruction::PropertyGetUnchecked {
            destination: RESULT,
            ..
        }
    ));
    assert_eq!(code[1], integer_load(COPY));
    assert!(matches!(
        code[2],
        Instruction::VecIndexGet {
            destination: RESULT,
            container: RESULT,
            index: COPY,
            ..
        }
    ));
}

#[test]
fn literal_loads_keep_release_alias_and_entry_boundaries() {
    let heap = Heap::new();
    let mut unit = literal_fixture(&heap);
    insert(
        chunk_mut(&mut unit),
        0,
        Instruction::Move {
            destination: COPY,
            source: PREVIOUS,
        },
    );
    let before = chunk_mut(&mut unit).code.clone();
    rewrite(&mut unit, &heap);
    assert_eq!(chunk_mut(&mut unit).code, before);

    for alias in [OBJECT, CONTAINER, RESULT] {
        let mut unit = literal_fixture(&heap);
        let chunk = chunk_mut(&mut unit);
        chunk.code[1] = integer_load(alias);
        chunk.code[2] = Instruction::Length {
            destination: RESULT,
            source: CONTAINER,
        };
        let before = chunk.code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);
    }
    for target in [1, 2] {
        let mut unit = literal_fixture(&heap);
        insert(
            chunk_mut(&mut unit),
            3,
            Instruction::JumpIfTrue {
                condition: PREVIOUS,
                offset: JumpOffset::new(target - 3),
            },
        );
        let before = chunk_mut(&mut unit).code.clone();
        rewrite(&mut unit, &heap);
        assert_eq!(chunk_mut(&mut unit).code, before);
    }
    let mut unit = literal_fixture(&heap);
    insert(chunk_mut(&mut unit), 1, integer_load(POSITION));
    let before = chunk_mut(&mut unit).code.clone();
    rewrite(&mut unit, &heap);
    assert_eq!(chunk_mut(&mut unit).code, before);
}

#[test]
fn minimal_length_pair_does_not_need_array_element_tracking() {
    let heap = Heap::new();
    let mut unit = fixture(&heap, "vec<(int, uint)>", false);
    let configuration = OptimizationConfiguration {
        specialize_arrays: false,
        const_fold: false,
        elide_type_checks: false,
        ..OptimizationConfiguration::default()
    };
    let candidates = CandidateSet::of(chunk_mut(&mut unit), configuration);
    assert!(candidates.contains(CandidateSet::PROPERTY_CONSUMER));
    assert!(!candidates.contains(CandidateSet::TYPE_CHECK));
    assert!(!candidates.needs_array_elements());
    verify_unit(&unit).unwrap();
    optimize_unit(&mut unit, &World::new(&[], &[]), &heap, configuration);
    verify_unit(&unit).unwrap();
    let code = &chunk_mut(&mut unit).code;
    assert!(matches!(
        code[0],
        Instruction::PropertyGetUnchecked {
            destination: RESULT,
            ..
        }
    ));
    assert!(matches!(
        code[1],
        Instruction::Length {
            destination: RESULT,
            source: RESULT
        }
    ));
}

#[test]
fn a_captured_scalar_is_not_a_local_length_destination() {
    let heap = Heap::new();
    let arena = LocalArena::new();
    let source = r"
        final class Store { public vec<int> $items = vec[]; }
        function capture(Store $store, uint $saved): fn(): uint {
            return fn(): uint => length!($store->items) + $saved;
        }
    ";
    let program = parse(&arena, source).unwrap();
    let mut configuration = CompileConfiguration::default();
    configuration.optimization.enabled = false;
    let mut unit = compile_with_configuration(
        program,
        "/captured-property-consumer.whim",
        &heap,
        configuration,
    )
    .unwrap();
    let capture_types = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"capture")
        .unwrap()
        .parameters
        .iter()
        .map(|parameter| parameter.declared_type.clone())
        .collect::<Vec<_>>();
    let position = unit
        .functions
        .iter()
        .position(|function| !function.capture_names.is_empty())
        .unwrap();
    let function = &mut unit.functions[position];
    assert!(!function.captures_this && function.parameters.is_empty());
    let object = Register::new(
        u16::try_from(
            function
                .capture_names
                .iter()
                .position(|name| name.as_bytes() == b"$store")
                .unwrap(),
        )
        .unwrap(),
    );
    let destination = Register::new(
        u16::try_from(
            function
                .capture_names
                .iter()
                .position(|name| name.as_bytes() == b"$saved")
                .unwrap(),
        )
        .unwrap(),
    );
    function.capture_types = function
        .capture_names
        .iter()
        .map(|name| {
            if name.as_bytes() == b"$store" {
                capture_types[0].clone()
            } else {
                capture_types[1].clone()
            }
        })
        .collect();
    let incoming = function.incoming_register_count(false);
    assert!(destination.index() < incoming);
    assert!(incoming <= function.chunk.local_register_count);
    let temporary = Register::new(function.chunk.local_register_count);
    let chunk = &mut function.chunk;
    chunk.register_count = chunk.register_count.max(temporary.index() + 1);
    chunk.reference_register_mask = (1u64 << chunk.register_count) - 1;
    chunk.code.clear();
    chunk.spans.clear();
    for instruction in [
        Instruction::PropertyGetUnchecked {
            destination: temporary,
            object,
            slot: PropertySlot::new(0),
            value_mode: PropertyReadMode::Clone,
        },
        Instruction::Length {
            destination,
            source: temporary,
        },
        Instruction::ReturnUnchecked {
            source: destination,
        },
    ] {
        chunk.emit(instruction, Span::zero());
    }
    let before = chunk.code.clone();
    let configuration = OptimizationConfiguration::default();
    let world = World::new(&[], &[]);
    {
        let indexed = IndexedUnit::with_world(&unit, &world);
        let analysis = Analysis::of_property_consumers(&indexed, configuration, &heap);
        let analyzed = analysis
            .chunks()
            .iter()
            .find(|analyzed| ptr::eq(analyzed.chunk, &unit.functions[position].chunk))
            .unwrap();
        assert!(
            !analyzed
                .flow
                .register_may_release_observably(1, destination)
        );
        assert!(analyzed.flow.proves_collection(1, temporary));
        assert!(destination.index() < analyzed.incoming_register_count);
    }
    verify_unit(&unit).unwrap();
    optimize_unit(&mut unit, &world, &heap, configuration);
    verify_unit(&unit).unwrap();
    assert_eq!(unit.functions[position].chunk.code, before);
}
