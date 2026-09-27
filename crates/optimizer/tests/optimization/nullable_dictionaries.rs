use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::IntegerKind;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

use super::compile;

fn code<'a>(unit: &'a CompiledUnit, name: &str) -> &'a [Instruction] {
    &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == name.as_bytes())
        .unwrap()
        .chunk
        .code
}

fn adds(code: &[Instruction], kind: Option<IntegerKind>) -> bool {
    code.iter().any(|instruction| matches!(instruction,
        Instruction::Add { kind: actual, .. } | Instruction::AddImmediate { kind: actual, .. } | Instruction::Step { kind: actual, .. }
        if *actual == kind
    ))
}

#[test]
fn fresh_dictionary_feedback_selects_integer_addition() {
    let unit = compile(
        r"
function strings(vec<string> $keys): dict {
    $counts = dict[];
    foreach ($keys as $key) { $counts[$key] = ($counts[$key] ?? 0) + 1; }
    return $counts;
}
function integers(vec<int> $keys): dict {
    $counts = dict[];
    foreach ($keys as $key) { $counts[$key] = ($counts[$key] ?? 0) + 1; }
    return $counts;
}
function unsigned(vec<uint> $keys): dict {
    $counts = dict[];
    foreach ($keys as $key) { $counts[$key] = ($counts[$key] ?? 0) + 1; }
    return $counts;
}
function nullable(vec<string> $keys): dict {
    $counts = dict[];
    foreach ($keys as $key) {
        $counts[$key] = null;
        $counts[$key] = ($counts[$key] ?? 0) + 1;
    }
    return $counts;
}
",
        OptimizationConfiguration {
            fuse_index_add_assign: false,
            ..OptimizationConfiguration::default()
        },
    );
    verify_unit(&unit).unwrap();
    for name in ["strings", "integers", "unsigned", "nullable"] {
        let code = code(&unit, name);
        assert!(adds(code, Some(IntegerKind::I64)), "{name}: {code:?}");
        assert!(!adds(code, None), "{name}: {code:?}");
    }
}

#[test]
fn unknown_sources_alias_writes_and_spreads_keep_generic_addition() {
    let unit = compile(
        r"
function parameter(dict $counts, string $key): mixed { return ($counts[$key] ?? 0) + 1; }
function generic<T>(dict<string, T> $counts, string $key): mixed { return ($counts[$key] ?? 0) + 1; }
function called(fn(): dict $get, string $key): mixed { $counts = $get(); return ($counts[$key] ?? 0) + 1; }
function changed(vec<string> $keys, mixed $value): dict {
    $counts = dict[];
    foreach ($keys as $key) {
        $counts[$key] = ($counts[$key] ?? 0) + 1;
        $counts[$key] = $value;
    }
    return $counts;
}
function copied(vec<string> $keys, mixed $value): dict {
    $counts = dict[];
    foreach ($keys as $key) {
        $copy = $counts;
        $copy[$key] = $value;
        $counts[$key] = ($counts[$key] ?? 0) + 1;
    }
    return $counts;
}
function merged(dict $other, bool $choose, string $key): mixed {
    $counts = dict[];
    if ($choose) { $counts = $other; }
    return ($counts[$key] ?? 0) + 1;
}
function spread(dict $other, string $key): mixed {
    $counts = dict[];
    $counts = dict[...$counts, ...$other];
    return ($counts[$key] ?? 0) + 1;
}
",
        OptimizationConfiguration {
            inline_leaf_calls: false,
            fuse_index_add_assign: false,
            ..OptimizationConfiguration::default()
        },
    );
    verify_unit(&unit).unwrap();
    for name in [
        "parameter",
        "generic",
        "called",
        "changed",
        "copied",
        "merged",
        "spread",
    ] {
        let code = code(&unit, name);
        assert!(adds(code, None), "{name}: {code:?}");
    }
}
