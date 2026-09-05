use std::path::Path;

use super::compile;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::verify::verify_unit;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::optimizer::OptimizationConfiguration;

const SIMPLE: &str = r"
function literal_match(string $value): 1..=3 {
    return match ($value) { 'foo' => 1, 'bar' => 2, $_ => 3 };
}
function typed_match(string $value): 1..=3 {
    return match ($value) { 'foo' => 1, 'bar' => 2, string => 3 };
}
function conditional(string $value): 1..=3 {
    if ($value == 'foo') { return 1; }
    if ($value == 'bar') { return 2; }
    return 3;
}
function invalid_return(string $value): 1..=2 {
    return match ($value) { 'foo' => 1, 'bar' => 2, $_ => 3 };
}
function incomplete(mixed $value): 1..=3 {
    return match ($value) { 'foo' => 1, 'bar' => 2, string => 3 };
}
";

#[test]
fn equivalent_string_dispatches_have_unchecked_literal_returns() {
    let unit = compile(SIMPLE, OptimizationConfiguration::default());
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
fn match_order_and_type_errors_survive_specialization() {
    let source = format!(
        r"{SIMPLE}
        function duplicate(string $value): int {{
            if ($value == 'foo') {{ return 1; }}
            if ($value == 'foo') {{ return 99; }}
            if ($value == 'bar') {{ return 2; }}
            return 3;
        }}
        function changed(string $value): int {{
            if ($value == 'foo') {{ return 1; }}
            $value = 'bar';
            if ($value == 'bar') {{ return 2; }}
            return 3;
        }}
        function typed_binding(string $value): string {{
            return match ($value) {{ 'foo' => 'one', $text @ string => $text }};
        }}
        foreach (vec['foo', 'bar', 'other', '', 'é'] as $value) {{
            assert!(literal_match($value) == typed_match($value));
            assert!(literal_match($value) == conditional($value));
            assert!(literal_match($value) == duplicate($value));
        }}
        assert!(changed('other') == 2);
        assert!(typed_binding('other') == 'other');
        $caught = false;
        try {{ invalid_return('other'); }} catch (Whim\Unwind\TypeError $_) {{ $caught = true; }}
        assert!($caught);
        $caught = false;
        try {{ incomplete(1); }} catch (Whim\Unwind\UnhandledMatchError $_) {{ $caught = true; }}
        assert!($caught);
    "
    );
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        assert_eq!(
            engine
                .run_source(&source, Path::new("/project/matching.whim"))
                .exit_code(),
            0
        );
    }
}

#[test]
fn pattern_tables_drop_impossible_types_without_reordering_overlap() {
    let source = r"
        function guaranteed(string $value): int {
            return match ($value) { int => 1, bool => 2, string => 3, $_ => 4 };
        }
        function impossible(string $value): int {
            return match ($value) { int => 1, bool => 2, $_ => 3 };
        }
        function overlap(string $value): int {
            return match ($value) { 'foo' => 1, int => 2, string => 3, $_ => 4 };
        }
        function unhandled(string $value): int {
            return match ($value) { int => 1, bool => 2 };
        }
        assert!(guaranteed('foo') == 3);
        assert!(impossible('foo') == 3);
        assert!(overlap('foo') == 1);
        assert!(overlap('bar') == 3);
        $caught = false;
        try { unhandled('foo'); } catch (Whim\Unwind\UnhandledMatchError $_) { $caught = true; }
        assert!($caught);
    ";
    let unit = compile(source, OptimizationConfiguration::default());
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
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let outcome = engine.run_source(source, Path::new("/matching/impossible-types.whim"));
        assert_eq!(outcome.exit_code(), 0, "optimize={optimize}: {outcome:?}");
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
            Instruction::CallNamed { .. } | Instruction::CallNamedUnchecked { .. }
        )),
        "{code:?}"
    );
}
