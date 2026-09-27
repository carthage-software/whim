use whim_bytecode::chunk::descriptors::IcDescriptor;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::DescriptorIndex;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;
use whim_optimizer::World;

use super::compile;
use super::method;
use super::optimize_function;

const FIXTURE: &str = include_str!("../../../../tests/_fixtures/nominal-refinement.whim");

fn code<'a>(unit: &'a CompiledUnit, name: &str) -> &'a [Instruction] {
    &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == name.as_bytes())
        .unwrap()
        .chunk
        .code
}

#[test]
fn immediate_final_class_tests_prove_properties_and_pattern_captures() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for name in [
        "evaluate",
        "immediate",
        "aliases",
        "foreach_values",
        "selected",
        "after_call",
    ] {
        let code = code(&unit, name);
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::PropertyGetUnchecked { .. })),
            "{name}: {code:?}",
        );
        assert!(
            !code.iter().any(|instruction| matches!(
                instruction,
                Instruction::PropertyGet { .. } | Instruction::Return { .. }
            )),
            "{name}: {code:?}",
        );
    }
    assert!(
        code(&unit, "evaluate")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallSelfUnchecked { .. }))
    );
}

#[test]
fn stale_partial_generic_and_inaccessible_class_facts_keep_property_checks() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for name in [
        "saved_test",
        "reassigned",
        "failed_test",
        "one_path",
        "different_classes",
        "open_class",
        "generic_class",
        "hidden_property",
        "different_iterations",
    ] {
        let code = code(&unit, name);
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::PropertyGet { .. })),
            "{name}: {code:?}",
        );
    }
    for name in [
        "saved_test",
        "reassigned",
        "generic_class",
        "changed_return",
    ] {
        let code = code(&unit, name);
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            "{name}: {code:?}",
        );
    }
}

#[test]
fn nominal_descriptor_index_limit_keeps_property_checks() {
    for index in [u16::MAX - 1, u16::MAX] {
        let mut unit = compile(
            "final class Item { public int $value = 1; } function read(mixed $item): int { if ($item is Item) { return $item->value; } return 0; }",
            OptimizationConfiguration {
                enabled: false,
                ..OptimizationConfiguration::default()
            },
        );
        let chunk = &mut unit.unit.functions[0].chunk;
        let descriptor = chunk.type_descriptors[0].clone();
        chunk
            .type_descriptors
            .resize(usize::from(index) + 1, TypeDescriptor::Mixed);
        chunk.type_descriptors[usize::from(index)] = descriptor;
        for instruction in &mut chunk.code {
            if let Instruction::Is { descriptor, .. } = instruction {
                *descriptor = DescriptorIndex::new(index);
            }
        }
        let units = [&unit.unit];
        let world = World::new(&units, &[]);
        let (optimized, _) = optimize_function(
            &unit.unit,
            0,
            Vec::new(),
            &world,
            &unit._heap,
            OptimizationConfiguration::default(),
        );
        assert_eq!(
            optimized
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::PropertyGetUnchecked { .. })),
            index != u16::MAX,
            "descriptor {index}: {:?}",
            optimized.code,
        );
        unit.unit.functions[0].chunk = optimized;
        verify_unit(&unit).unwrap();
    }
}

#[test]
fn nominal_facts_do_not_prove_mutable_property_contents() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    let function = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"changed_shape")
        .unwrap();
    assert!(
        function.chunk.code.iter().any(|instruction| {
            let Instruction::CallNamed { cache, .. } = instruction else {
                return false;
            };
            matches!(
                &function.chunk.ic_descriptors[usize::from(cache.index())],
                IcDescriptor::Member { name, .. } if name.as_bytes() == b"accept_shape"
            )
        }),
        "{:?}",
        function.chunk.code,
    );
    verify_unit(&unit).unwrap();
    assert!(
        !method(&unit, b"Hidden::match_value")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyGetUnchecked { .. }))
    );
}
