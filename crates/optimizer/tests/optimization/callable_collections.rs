use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

#[test]
fn foreach_callbacks_keep_parameter_and_return_types() {
    let unit = compile(
        include_str!("../../../../tests/_fixtures/callable-collections.whim"),
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for name in [
        "dispatch_vector",
        "dispatch_dictionary",
        "dispatch_tuple",
        "sum_vector",
        "sum_dictionary",
        "sum_tuple",
    ] {
        let code = &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name.as_bytes())
            .unwrap()
            .chunk
            .code;
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::CallValueUnchecked { .. })),
            "{name}: {code:?}",
        );
        assert!(
            !code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallValue { .. }
                    | Instruction::CallValueDiscarded { .. }
                    | Instruction::CheckDiscardedResult { .. }
                    | Instruction::Return { .. }
            )),
            "{name}: {code:?}",
        );
    }
}

#[test]
fn foreach_callbacks_keep_unproven_argument_and_discard_checks() {
    let unit = compile(
        include_str!("../../../../tests/_fixtures/callable-collections.whim"),
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for (name, discarded) in [
        ("unknown_argument", false),
        ("changed_callback", false),
        ("unknown_callback", true),
        ("discard_value", true),
    ] {
        let code = &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name.as_bytes())
            .unwrap()
            .chunk
            .code;
        assert!(
            code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallValue { .. } | Instruction::CallValueDiscarded { .. }
            )),
            "{name}: {code:?}",
        );
        assert!(
            !code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CallValueUnchecked { .. })),
            "{name}: {code:?}",
        );
        assert_eq!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::CheckDiscardedResult { .. })),
            discarded,
            "{name}: {code:?}",
        );
    }
}

#[test]
fn foreach_key_moves_keep_distinct_output_registers() {
    let unit = compile(
        include_str!("../../../../tests/_fixtures/callable-collections.whim"),
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for name in [
        "callback_from_key",
        "dictionary_key",
        "vector_key",
        "tuple_key",
        "generic_key",
    ] {
        let code = &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name.as_bytes())
            .unwrap()
            .chunk
            .code;
        let outputs = code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::ForeachNext {
                    key_destination,
                    value_destination,
                    ..
                }
                | Instruction::VecForeachNext {
                    key_destination,
                    value_destination,
                    ..
                }
                | Instruction::DictForeachNext {
                    key_destination,
                    value_destination,
                    ..
                } => Some((key_destination, value_destination)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!outputs.is_empty(), "{name}: {code:?}");
        for (key, value) in outputs {
            assert_ne!(key, value, "{name}: {code:?}");
        }
        if name == "callback_from_key" {
            assert!(
                code.iter()
                    .any(|instruction| matches!(instruction, Instruction::Return { .. })),
                "{name}: {code:?}",
            );
        }
        if name == "dictionary_key" {
            assert!(
                !code.iter().any(|instruction| matches!(
                    instruction,
                    Instruction::ReturnScalarUnchecked { .. }
                )),
                "{name}: {code:?}",
            );
        }
    }
}
