use std::path::Path;

use whim_bytecode::instruction::Instruction;
use whim_value::Value;
use whim_value::function::CallTarget;
use whim_value::function::FuncId;
use whim_value::function::FunctionObject;

use super::run_both_modes;
use crate::builtin::throw::Throw;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::engine::builtins::BuiltInCallable;
use crate::symbols::ExactBuiltInFunctionEntry;
use crate::symbols::RuntimeFunction;
use crate::vm::VirtualMachine;

fn function<'engine>(engine: &'engine Engine, name: &[u8]) -> &'engine RuntimeFunction {
    engine
        .tables
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == name)
        .unwrap()
}

fn byte_site(engine: &Engine, name: &[u8]) -> Option<ExactBuiltInFunctionEntry> {
    let function = function(engine, name);
    // SAFETY: this test owns the idle engine and only reads its cache.
    let sites = unsafe { &*function.cache.exact_built_in_functions() };
    sites.iter().flatten().copied().next()
}

#[test]
fn native_byte_at_preserves_strings_tags_and_argument_windows() {
    let source = r#"
use Whim\Marker\NeverInline;
newtype TaggedString = string;
newtype TaggedOffset = 0..;
#[NeverInline]
function join(string $left, string $right): string { return $left . $right; }
#[NeverInline]
function borrowed(string $text, 0.. $offset): (int, string, int) {
    $byte = Whim\Str\byte_at($text, $offset);
    return ($byte, $text, $offset);
}
#[NeverInline]
function owned(0.. $offset, string $text): int {
    $byte = Whim\Str\byte_at($text, $offset);
    return $byte + 1;
}
#[NeverInline]
function checked(mixed $text, mixed $offset): int {
    return Whim\Str\byte_at($text, $offset);
}
$base = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ';
$slice = Whim\_Private\string_slice($base, 3, 32);
$base = 'changed';
$rope = join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ');
foreach (vec["\0\xff\x80\x7f", 'a flat string longer than seven bytes', $slice, $rope,
    TaggedString('tagged'), TaggedString($slice)] as $text) {
    for ($offset = 0; $offset < length!($text); $offset++) {
        $actual = borrowed($text, $offset);
        $expected = checked($text, $offset);
        assert!($actual == ($expected, $text, $offset));
        assert!(owned($offset, $text) == $expected + 1);
        $tagged = borrowed($text, TaggedOffset($offset));
        assert!($tagged[0] == $expected && !($tagged[0] is TaggedOffset));
        assert!($tagged[2] is TaggedOffset);
    }
}
assert!($slice == 'defghijklmnopqrstuvwxyzABCDEFGHI');
assert!($rope == 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ');
assert!(owned(0, join('a temporary string longer ', 'than seven bytes')) == 98);
"#;
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/native-byte-at-values.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        assert!(byte_site(&engine, b"checked").is_none());
        if optimize {
            assert!(byte_site(&engine, b"borrowed").unwrap().string_byte_at);
            assert!(byte_site(&engine, b"owned").unwrap().string_byte_at);
            // SAFETY: the idle engine owns the function chunks for these reads.
            let borrowed = unsafe { function(&engine, b"borrowed").chunk.as_ref() };
            assert!(
                borrowed.code.iter().any(|instruction| {
                    matches!(instruction, Instruction::CallNamedDirect { .. })
                })
            );
            // SAFETY: the idle engine owns the function chunks for these reads.
            let owned = unsafe { function(&engine, b"owned").chunk.as_ref() };
            assert!(owned.code.iter().any(|instruction| {
                matches!(instruction, Instruction::CallNamedUnchecked { .. })
            }));
        }
    }
}

#[test]
fn native_byte_at_keeps_bounds_type_errors_traces_and_discard_checks() {
    run_both_modes(
        r#"
use Whim\Marker\NeverInline;
use Whim\Marker\TrackCaller;
#[NeverInline]
#[TrackCaller]
function read(string $text, 0.. $offset): int {
    $byte = Whim\Str\byte_at($text, $offset);
    return $byte + 1;
}
#[NeverInline]
function checked(mixed $text, mixed $offset): int {
    return Whim\Str\byte_at($text, $offset);
}
#[NeverInline]
function discard_byte(string $text, 0.. $offset): void {
    Whim\Str\byte_at($text, $offset);
}
assert!(read('valid', 0) == 119);
foreach (vec[('', 0), ('abc', 3), ('a long string', 9223372036854775807)] as $case) {
    $caught = false;
    try { discard!(read($case[0], $case[1])); }
    catch (Whim\Unwind\OutOfBoundsError $error) {
        assert!($error->getMessage() == 'string offset is out of bounds');
        $trace = $error->getTrace();
        assert!($trace[0]->function == 'Whim\Str\byte_at');
        assert!($trace[0]->arguments[0] == $case[0]);
        assert!($trace[0]->arguments[1] == $case[1]);
        assert!($trace[1]->function == 'read');
        $caught = true;
    }
    assert!($caught);
    $caught = false;
    try { discard!(checked($case[0], $case[1])); }
    catch (Whim\Unwind\OutOfBoundsError $_) { $caught = true; }
    assert!($caught);
}
foreach (vec[(123, 0), ('abc', -1), ('abc', 0u), ('abc', '0'), ('abc', null),
    ('abc', 1.5)] as $case) {
    $caught = false;
    try { discard!(checked($case[0], $case[1])); }
    catch (Whim\Unwind\TypeError $_) { $caught = true; }
    assert!($caught);
}
$caught = false;
try { discard_byte('valid', 0); }
catch (Whim\Unwind\DiscardedResultError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { discard_byte('', 0); }
catch (Whim\Unwind\OutOfBoundsError $_) { $caught = true; }
assert!($caught);
discard!(Whim\Str\byte_at('valid', 0));
assert!(read("\xff", 0) == 256);
"#,
        "/native-byte-at-errors.whim",
    );
}

fn replacement_byte_at(_vm: &mut VirtualMachine<'_>, _arguments: &[Value]) -> Result<Value, Throw> {
    Ok(Value::int(17))
}

#[test]
fn native_byte_at_requires_the_registered_name_signature_and_handler() {
    for mismatch in ["name", "signature", "handler"] {
        let mut engine = Engine::new(EngineConfiguration::default());
        let BuiltInCallable::Function(spec) = engine
            .tables
            .built_in_functions
            .iter_mut()
            .find(|callable| {
                matches!(callable, BuiltInCallable::Function(spec) if spec.name == "Whim\\Str\\byte_at")
            })
            .unwrap()
        else {
            unreachable!();
        };
        match mismatch {
            "name" => spec.name = "different_byte_at",
            "signature" => spec.signature = "fn(string, int): int",
            "handler" => spec.direct_handler = Some(replacement_byte_at),
            _ => unreachable!(),
        }
        let expected = if mismatch == "handler" { 18 } else { 98 };
        let source = format!(
            r"
#[Whim\Marker\NeverInline]
function read(string $text, 0.. $offset): int {{
    $byte = Whim\Str\byte_at($text, $offset);
    return $byte + 1;
}}
assert!(read('abc', 0) == {expected});
"
        );
        let result = engine.run_source(&source, Path::new("/native-byte-at-identity.whim"));
        assert_eq!(result.exit_code(), 0, "mismatch {mismatch}: {result:?}");
        assert!(!byte_site(&engine, b"read").unwrap().string_byte_at);
    }
}

#[test]
fn native_byte_at_survives_failed_declarations() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(
            r"
#[Whim\Marker\NeverInline]
function read(string $text, 0.. $offset): int {
    $byte = Whim\Str\byte_at($text, $offset);
    return $byte + 1;
}
function stage(): string { assert!(read('staged', 0) == 116); return 'bad'; }
assert!(read('before', 0) == 99);
",
            Path::new("/native-byte-at-before-rollback.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let class_count = engine.tables.classes.len();
        let entry = byte_site(&engine, b"read");
        let result = engine.run_source(
            r"
final class Broken { public static int $value = stage(); }
",
            Path::new("/native-byte-at-failed-declaration.whim"),
        );
        assert_ne!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        assert_eq!(engine.tables.classes.len(), class_count);
        let result = engine.run_source(
            r"
final class Replacement {}
assert!(read('after', 0) == 98);
",
            Path::new("/native-byte-at-after-rollback.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        if optimize {
            let entry = entry.unwrap();
            let current = byte_site(&engine, b"read").unwrap();
            assert!(entry.string_byte_at && current.string_byte_at);
            assert_eq!(entry.function, current.function);
        }
    }
}

#[test]
fn native_byte_at_keeps_result_aliases_and_finalizer_order() {
    let source = r"
use Whim\Marker\NeverInline;
final class Events { public static vec<string> $items = vec[]; }
final class Released {
    public function __destruct(): void { Events::$items[] = 'drop'; }
}
final class Throwing {
    public function __destruct(): void { throw new Whim\Unwind\Exception('drop failed'); }
}
#[NeverInline]
function alias(string $text, 0.. $offset): int {
    $text = Whim\Str\byte_at($text, $offset);
    return $text + 1;
}
#[NeverInline]
function released(string $text, 0.. $offset): int {
    $result = new Released();
    $result = Whim\Str\byte_at($text, $offset);
    Events::$items[] = 'read';
    return $result + 1;
}
#[NeverInline]
function failed(string $text, 0.. $offset): void {
    $result = new Throwing();
    $result = Whim\Str\byte_at($text, $offset);
    Events::$items[] = 'unreachable';
}
for ($round = 0; $round < 3; $round++) {
    assert!(alias('a string with a heap buffer', 0) == 98);
    assert!(released('valid', 0) == 119);
}
assert!(Events::$items == vec['drop', 'read', 'drop', 'read', 'drop', 'read']);
$caught = false;
try { failed('valid', 0); }
catch (Whim\Unwind\Exception $error) {
    assert!($error->getMessage() == 'drop failed');
    $caught = true;
}
assert!($caught);
assert!(Events::$items == vec['drop', 'read', 'drop', 'read', 'drop', 'read']);
assert!(alias('after', 0) == 98);
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/native-byte-at-reload-finalizers.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        if optimize {
            assert!(byte_site(&engine, b"alias").unwrap().string_byte_at);
            // SAFETY: this test owns the idle engine and only reads its chunk.
            let chunk = unsafe { function(&engine, b"alias").chunk.as_ref() };
            assert!(chunk.code.iter().any(|instruction| {
                matches!(instruction, Instruction::CallNamedDirect {
                    destination,
                    first_argument,
                    ..
                } if destination == first_argument)
            }));
        }
    }
}

#[test]
fn native_byte_at_reload_keeps_error_locations_and_caller_state() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function read(string $text, 0.. $offset): int {
    $byte = Whim\Str\byte_at($text, $offset);
    return $byte + 1;
}
";
    let line = source
        .lines()
        .position(|line| line.contains("$byte = Whim\\Str\\byte_at"))
        .unwrap()
        + 1;
    let source = format!(
        r"{source}
$total = 0;
for ($round = 0; $round < 3; $round++) {{
    $total += read('valid', 0);
    $caught = false;
    try {{ discard!(read('bad', 3)); }}
    catch (Whim\Unwind\OutOfBoundsError $error) {{
        assert!($error->getFile() == '/native-byte-at-reload-errors.whim');
        assert!($error->getLine() == {line});
        $trace = $error->getTrace();
        assert!($trace[0]->function == 'Whim\Str\byte_at');
        assert!($trace[0]->arguments == vec['bad', 3]);
        assert!($trace[1]->function == 'read');
        $caught = true;
    }}
    assert!($caught);
    assert!($total == ($round + 1) * 119);
}}
assert!($total == 357);
"
    );
    run_both_modes(&source, "/native-byte-at-reload-errors.whim");
}

fn reentrant_byte_at(vm: &mut VirtualMachine<'_>, _arguments: &[Value]) -> Result<Value, Throw> {
    let position = vm
        .engine
        .tables
        .functions
        .iter()
        .position(|function| function.name.as_bytes() == b"grow")
        .unwrap();
    let signature = vm.engine.tables.functions[position].signature.clone();
    let callee = Value::function(FunctionObject::closure(
        vm.heap(),
        CallTarget::User(FuncId(u32::try_from(position).unwrap())),
        [],
        signature,
        None,
        Default::default(),
    ));
    let result = vm.call_function_value(&callee, &[Value::int(400)])?;
    assert_eq!(result.as_int(), Some(400));
    Ok(Value::int(17))
}

#[test]
fn native_byte_at_replacement_reloads_after_reentrant_stack_growth() {
    let mut engine = Engine::new(EngineConfiguration::default());
    let BuiltInCallable::Function(spec) = engine
        .tables
        .built_in_functions
        .iter_mut()
        .find(|callable| {
            matches!(callable, BuiltInCallable::Function(spec) if spec.name == "Whim\\Str\\byte_at")
        })
        .unwrap()
    else {
        unreachable!();
    };
    spec.direct_handler = Some(reentrant_byte_at);
    let source = r"
use Whim\Marker\NeverInline;
final class Calls { public static int $count = 0; }
#[NeverInline]
function grow(int $depth): int {
    $live = vec[$depth, $depth + 1, $depth + 2];
    if ($depth == 0) { Calls::$count++; return 0; }
    $result = grow($depth - 1);
    assert!($live == vec[$depth, $depth + 1, $depth + 2]);
    return $result + 1;
}
#[NeverInline]
function borrowed(string $text, 0.. $offset): int {
    $byte = Whim\Str\byte_at($text, $offset);
    assert!($text == 'valid' && $offset == 0);
    return $byte + 1;
}
#[NeverInline]
function owned(0.. $offset, string $text): int {
    $byte = Whim\Str\byte_at($text, $offset);
    return $byte + 2;
}
$total = 0;
for ($round = 0; $round < 3; $round++) {
    $total += borrowed('valid', 0);
    $total += owned(0, 'valid');
    assert!($total == ($round + 1) * 37);
}
assert!(Calls::$count == 6);
assert!($total == 111);
";
    let result = engine.run_source(source, Path::new("/native-byte-at-reload-reentrant.whim"));
    assert_eq!(result.exit_code(), 0, "{result:?}");
    for name in [b"borrowed".as_slice(), b"owned".as_slice()] {
        assert!(!byte_site(&engine, name).unwrap().string_byte_at);
    }
    // SAFETY: this test owns the idle engine and only reads its chunks.
    let borrowed = unsafe { function(&engine, b"borrowed").chunk.as_ref() };
    assert!(
        borrowed
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::CallNamedDirect { .. }) })
    );
    // SAFETY: this test owns the idle engine and only reads its chunks.
    let owned = unsafe { function(&engine, b"owned").chunk.as_ref() };
    assert!(
        owned
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::CallNamedUnchecked { .. }) })
    );
}
