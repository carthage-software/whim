use std::env::consts;
use std::path::Path;

use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;

use super::compile;
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
fn constant_conditions_and_matches_keep_only_the_selected_arm() {
    let mut bodies = vec![
        "if ('linux' == 'linux') { return 'chosen'; } else { return 'discarded'; }".to_string(),
        "if ('linux' != 'linux') { return 'discarded'; } return 'chosen';".to_string(),
        "if (false) { return 'discarded'; } if (true) { return 'chosen'; } return 'discarded';".to_string(),
        "if (1 == 1.0) { return 'discarded'; } return 'chosen';".to_string(),
        "if (9007199254740993 > 9007199254740992.0) { return 'chosen'; } return 'discarded';".to_string(),
        "return match ('linux') { 'macos' => 'discarded', 'linux' => 'chosen', 'linux' => 'discarded', _ => 'discarded' };".to_string(),
        "return match ('b') { 'a' => 'discarded', 'b' => 'chosen', 'c' => 'discarded', _ => 'discarded' };".to_string(),
        "return match (2) { 1 => 'discarded', 2 => 'chosen', 3 => 'discarded', _ => 'discarded' };".to_string(),
        "return match (-0.0) { 0.0 => 'chosen', 1.5 => 'discarded', _ => 'discarded' };".to_string(),
        "return match (false) { true => 'discarded', false => 'chosen' };".to_string(),
        "return match ('other') { 'linux' => 'discarded', 'macos' => 'discarded', _ => 'chosen' };".to_string(),
        format!("if (operating_system!() == '{}') {{ return 'chosen'; }} return 'discarded';", consts::OS),
        format!("return match (shared_library_suffix!()) {{ '{}' => 'chosen', _ => 'discarded' }};", consts::DLL_SUFFIX),
        format!("return match (operating_system!() . '/' . cpu_architecture!()) {{ '{}/{}' => 'chosen', _ => 'discarded' }};", consts::OS, consts::ARCH),
        "return match (operating_system!()) { $os @ string => match ($os == operating_system!()) { true => 'chosen', _ => 'discarded' } };".to_string(),
        "if ((('li' . 'nux') == 'linux' && length!(operating_system!()) > 0) || false) { return 'chosen'; } return 'discarded';".to_string(),
        "$value = match (operating_system!() == operating_system!()) { true => 'linux', _ => 'macos' }; return match ($value) { 'linux' => 'chosen', _ => 'discarded' };".to_string(),
        format!("return match ((operating_system!(), cpu_architecture!())) {{ ('{}', '{}') => 'chosen', _ => 'discarded' }};", consts::OS, consts::ARCH),
        "return match ((1, 'linux')) { (2, string) => 'discarded', (1..=3, ...string) => 'chosen', _ => 'discarded' };".to_string(),
        "return match (length!(operating_system!())) { 0 => 'discarded', 1.. => 'chosen', _ => 'discarded' };".to_string(),
        "return match (null) { true => 'discarded', false => 'discarded', _ => 'chosen' };".to_string(),
    ];
    for (construct, expected) in [
        ("cpu_architecture", consts::ARCH),
        ("operating_system", consts::OS),
        ("operating_system_family", consts::FAMILY),
        ("shared_library_prefix", consts::DLL_PREFIX),
        ("shared_library_suffix", consts::DLL_SUFFIX),
        ("shared_library_extension", consts::DLL_EXTENSION),
        ("executable_suffix", consts::EXE_SUFFIX),
        ("executable_extension", consts::EXE_EXTENSION),
    ] {
        bodies.push(format!(
            "if ({construct}!() == '{expected}') {{ return 'chosen'; }} return 'discarded';"
        ));
        bodies.push(format!(
            "return match ({construct}!()) {{ '{expected}' => 'chosen', _ => 'discarded' }};"
        ));
    }
    for body in bodies {
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
        for optimize in [false, true] {
            let mut engine = Engine::new(EngineConfiguration {
                optimize,
                ..EngineConfiguration::default()
            });
            let outcome = engine.run_source(&source, Path::new("/constant-branches.whim"));
            assert_eq!(
                outcome.exit_code(),
                0,
                "{body}, optimize={optimize}: {outcome:?}"
            );
        }
    }
}

#[test]
fn folded_branches_preserve_effects_dynamic_choices_and_errors() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function choose(bool $flag): int {
    if (operating_system!() == operating_system!() && $flag) { return 1; }
    return 2;
}
#[Whim\Marker\AlwaysInline]
function incomplete(string $value): int {
    return match ($value) { 'linux' => 1, 'macos' => 2 };
}
assert!(choose(true) == 1);
assert!(choose(false) == 2);
$seen = vec[];
if (($seen[] = 'linux') == 'linux') { $seen[] = 'chosen'; }
else { $seen[] = 'discarded'; }
assert!($seen == vec['linux', 'chosen']);
if (false && sequence!($seen[] = 'discarded', true)) { $seen[] = 'discarded'; }
if (true || sequence!($seen[] = 'discarded', false)) { $seen[] = 'short-circuit'; }
assert!($seen == vec['linux', 'chosen', 'short-circuit']);
foreach (vec[1, 2, 3] as $number) {
    if (false) { $seen[] = 'discarded'; }
}
assert!($seen == vec['linux', 'chosen', 'short-circuit']);
try {
    if ('linux' == 'linux') { $seen[] = 'try'; }
    else { panic!('discarded'); }
} finally { $seen[] = 'finally'; }
assert!($seen == vec['linux', 'chosen', 'short-circuit', 'try', 'finally']);
$caught = false;
try { if ('linux') { panic!('unreachable'); } }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { if ('linux' < 1) { panic!('unreachable'); } }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { $_ = match ('linux') { 'macos' => 1 }; }
catch (Whim\Unwind\UnhandledMatchError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { $_ = incomplete('other'); }
catch (Whim\Unwind\UnhandledMatchError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { $_ = incomplete(null); }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let outcome = engine.run_source(source, Path::new("/constant-branch-effects.whim"));
        assert_eq!(outcome.exit_code(), 0, "optimize={optimize}: {outcome:?}");
    }
}

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
            Instruction::CallNamed { .. }
                | Instruction::CallNamedUnchecked { .. }
                | Instruction::CallNamedDirect { .. }
        )),
        "{code:?}"
    );
}
