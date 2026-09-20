use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

#[path = "../../../../tests/_fixtures/return_contracts.rs"]
mod fixtures;

const PROVEN: &[&str] = &[
    "record",
    "record_rest",
    "duplicate_key",
    "bool_rest",
    "distinct_keys",
    "boolean_shape",
    "vector_tail",
    "tuple_tail",
    "tuple_array",
    "cow_alias",
    "nested_cow_alias",
    "nested_child_alias",
    "escaped_child",
];

#[test]
fn structural_return_proofs_elide_only_statically_valid_checks() {
    let unit = compile(fixtures::RETURNS, OptimizationConfiguration::default());
    verify_unit(&unit).expect("optimized structural returns verify");

    for (names, checked) in [(PROVEN, false), (fixtures::CHECKED, true)] {
        for name in names {
            let function = unit
                .functions
                .iter()
                .find(|function| function.name.as_bytes() == name.as_bytes())
                .expect("the fixture function exists");
            let code = &function.chunk.code;
            assert_eq!(
                code.iter()
                    .any(|instruction| matches!(instruction, Instruction::Return { .. })),
                checked,
                "{name}: {code:#?}",
            );
            if !checked {
                assert!(
                    code.iter().any(|instruction| matches!(
                        instruction,
                        Instruction::ReturnReferenceUnchecked { .. }
                            | Instruction::ReturnPairUnchecked { .. }
                    )),
                    "{name}: {code:#?}",
                );
            }
        }
    }
}
