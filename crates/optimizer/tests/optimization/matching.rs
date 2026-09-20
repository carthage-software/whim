use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

#[path = "../../../../tests/_fixtures/constant_branches.rs"]
mod constant_branches;

#[test]
fn constant_conditions_and_matches_keep_only_the_selected_arm() {
    for body in constant_branches::bodies() {
        let source =
            format!("function folded(): string {{ {body} }} assert!(folded() == 'chosen');");
        let unit = compile(&source, OptimizationConfiguration::default());
        verify_unit(&unit).expect("folded branches verify");
        let chunk = &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == b"folded")
            .unwrap()
            .chunk;
        assert!(chunk.switch_tables.is_empty(), "{body}: {:?}", chunk.code);
        let mut loaded = Vec::new();
        for instruction in &chunk.code {
            match instruction {
                Instruction::LoadConstant { constant, .. } => {
                    let Literal::String(value) = &chunk.constants[usize::from(constant.index())]
                    else {
                        panic!("{body}: {instruction:?}")
                    };
                    loaded.push(value.as_bytes());
                }
                Instruction::ReturnReferenceUnchecked { .. }
                | Instruction::ReturnUnchecked { .. }
                | Instruction::ReturnNull => {}
                _ => panic!("{body}: {:?}", chunk.code),
            }
        }
        assert_eq!(loaded, [b"chosen".as_slice()], "{body}: {:?}", chunk.code);
    }
}

#[test]
fn equivalent_string_dispatches_have_unchecked_literal_returns() {
    let unit = compile(
        include_str!("../../../../tests/_fixtures/matching.whim"),
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).expect("rewritten switches verify");
    for name in [b"literal_match".as_slice(), b"typed_match", b"conditional"] {
        let function = unit
            .functions
            .iter()
            .find(|f| f.name.as_bytes() == name)
            .unwrap();
        let code = &function.chunk.code;
        assert!(
            code.iter()
                .any(|i| matches!(i, Instruction::SwitchString { .. })),
            "{name:?}: {code:?}"
        );
        assert!(
            !code.iter().any(|i| matches!(
                i,
                Instruction::Is { .. }
                    | Instruction::SwitchPattern { .. }
                    | Instruction::StringJumpUnless { .. }
                    | Instruction::ThrowUnhandledMatch { .. }
                    | Instruction::Return { .. }
                    | Instruction::Move { .. }
                    | Instruction::MoveOwned { .. }
            )),
            "{name:?}: {code:?}"
        );
        assert_eq!(function.chunk.register_count, 1, "{name:?}: {code:?}");
        for value in [1, 2, 3] {
            assert!(code.iter().any(|i| matches!(i, Instruction::ReturnIntUnchecked { immediate } if immediate.value() == value)), "{name:?}: {code:?}");
        }
    }
    for name in [b"invalid_return".as_slice(), b"incomplete"] {
        let code = &unit
            .functions
            .iter()
            .find(|f| f.name.as_bytes() == name)
            .unwrap()
            .chunk
            .code;
        assert!(
            code.iter().any(|i| matches!(
                i,
                Instruction::Return { .. } | Instruction::ThrowUnhandledMatch { .. }
            )),
            "{name:?}: {code:?}"
        );
    }
}

#[test]
fn pattern_tables_drop_impossible_types_without_reordering_overlap() {
    let unit = compile(
        include_str!("../../../../tests/_fixtures/pattern-order.whim"),
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).expect("filtered pattern tables verify");
    for function in &unit.functions {
        let code = &function.chunk.code;
        if function.name.as_bytes() == b"unhandled" {
            assert!(
                code.iter().any(|instruction| matches!(
                    instruction,
                    Instruction::ThrowUnhandledMatch { .. }
                ))
            );
            continue;
        }
        assert!(
            !code.iter().any(|instruction| matches!(
                instruction,
                Instruction::SwitchPattern { .. }
                    | Instruction::Is { .. }
                    | Instruction::ThrowUnhandledMatch { .. }
            )),
            "{}: {code:?}",
            function.name
        );
    }
}

#[test]
fn string_chain_dispatch_preserves_existing_call_inlining() {
    let unit = compile(
        r"
        function small(string $value): int {
            if ($value == 'foo') { return 1; }
            if ($value == 'bar') { return 2; }
            return 3;
        }
        function caller(string $value): int { return small($value); }
        ",
        OptimizationConfiguration::default(),
    );

    verify_unit(&unit).expect("inlined dispatch verifies");
    let code = &unit
        .functions
        .iter()
        .find(|f| f.name.as_bytes() == b"caller")
        .unwrap()
        .chunk
        .code;
    assert!(
        !code.iter().any(|i| matches!(
            i,
            Instruction::CallNamed { .. }
                | Instruction::CallNamedUnchecked { .. }
                | Instruction::CallNamedDirect { .. }
        )),
        "{code:?}"
    );
}
