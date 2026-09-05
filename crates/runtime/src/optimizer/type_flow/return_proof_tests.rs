use std::path::Path;

use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::verify::verify_unit;
use crate::compiler::CompileConfiguration;
use crate::compiler::compile_with_configuration;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::optimizer::OptimizationConfiguration;
use crate::value::heap::Heap;

const RETURNS: &str = r"
use Whim\Marker\NeverInline;

#[NeverInline]
function record(): dict['a' => 1.., 'b' => string&!'', 'c' => (1, 1..=10), ...<string, int>] {
    return dict['a' => 1, 'b' => 'hello', 'c' => (1, 2)];
}
#[NeverInline]
function record_rest(): dict['a' => 1, ...<string&!'', 1..=3>] {
    return dict['a' => 1, 'extra' => 2];
}
#[NeverInline]
function duplicate_key(): dict['a' => 1] {
    $key = 'a';
    return dict[$key => 'discarded', 'a' => 1];
}
#[NeverInline]
function bool_rest(): dict['a' => 1, ...<bool, 1..=3>] {
    return dict['a' => 1, true => 2, false => 3];
}
#[NeverInline]
function distinct_keys(): dict[1 => 1, '1' => 2] { return dict[1 => 1, '1' => 2]; }
#[NeverInline]
function vector_tail(): vec[1, ...1..=3] { return vec[1, 2, 3]; }
#[NeverInline]
function tuple_tail(): (1, ...1..=3) { return (1, 2, 3); }
#[NeverInline]
function tuple_array(): array<0..=1, 1..=2> { return (1, 2); }
#[NeverInline]
function cow_alias(): dict['a' => 1] {
    $value = dict['a' => 1];
    $copy = $value;
    $value['a'] = 0;
    return $copy;
}
#[NeverInline]
function nested_cow_alias(): vec[dict['a' => 1]] {
    $value = vec[dict['a' => 1]];
    $copy = $value;
    $value[0]['a'] = 0;
    return $copy;
}
#[NeverInline]
function nested_child_alias(): vec[dict['a' => 1]] {
    $child = dict['a' => 1];
    $value = vec[$child];
    $child['a'] = 0;
    return $value;
}
#[NeverInline]
function change_copy(dict $value): void { $value['a'] = 0; }
#[NeverInline]
function escaped_child(): vec[dict['a' => 1]] {
    $child = dict['a' => 1];
    $value = vec[$child];
    change_copy($child);
    return $value;
}

#[NeverInline]
function invalid_missing(): dict['a' => 1] { return dict[]; }
#[NeverInline]
function invalid_extra(): dict['a' => 1] { return dict['a' => 1, 'extra' => 2]; }
#[NeverInline]
function invalid_rest_key(): dict['a' => 1, ...<string, int>] { return dict['a' => 1, 2 => 2]; }
#[NeverInline]
function invalid_rest_value(): dict['a' => 1, ...<string, int>] { return dict['a' => 1, 'b' => 'bad']; }
#[NeverInline]
function invalid_duplicate(): dict['a' => 1] {
    $key = 'a';
    return dict[$key => 1, 'a' => 0];
}
#[NeverInline]
function invalid_range(): dict['a' => 1..] { return dict['a' => 0]; }
#[NeverInline]
function invalid_empty_string(): dict['a' => string&!''] { return dict['a' => '']; }
#[NeverInline]
function invalid_tuple_member(): dict['a' => (1, 1..=10)] { return dict['a' => (1, 11)]; }
#[NeverInline]
function invalid_tuple_arity(): dict['a' => (1, 2)] { return dict['a' => (1, 2, 3)]; }
#[NeverInline]
function invalid_vector_tail(): vec[1, ...1..=3] { return vec[1, 4]; }
#[NeverInline]
function invalid_vector_length(): vec[1, 2] { return vec[1]; }
#[NeverInline]
function invalid_tuple_tail(): (1, ...1..=3) { return (1, 4); }
#[NeverInline]
function invalid_tuple_array_key(): array<string, int> { return (1, 2); }
#[NeverInline]
function invalid_mutation(): dict['a' => 1] {
    $value = dict['a' => 1];
    $value['a'] = 0;
    return $value;
}
#[NeverInline]
function invalid_nested_mutation(): vec[dict['a' => 1]] {
    $value = vec[dict['a' => 1]];
    $value[0]['a'] = 0;
    return $value;
}
#[NeverInline]
function invalid_copy_mutation(): dict['a' => 1] {
    $value = dict['a' => 1];
    $copy = $value;
    $copy['a'] = 0;
    return $copy;
}
#[NeverInline]
function invalid_duplicate_shape(): dict['a' => 1, 'a' => 1] { return dict['a' => 1]; }
#[NeverInline]
function invalid_bool_key(): dict[1 => 1] { return dict[true => 1]; }
newtype Key = string;
#[NeverInline]
function invalid_rest_newtype(): dict['a' => 1, ...<Key, int>] {
    return dict['a' => 1, 'b' => 2];
}
#[NeverInline]
function unknown_key(string $key): dict['a' => 1] { return dict[$key => 1]; }
#[NeverInline]
function callback_result(fn(): dict $callback): vec[dict['a' => 1]] {
    return vec[$callback()];
}
";

const PROVEN: &[&str] = &[
    "record",
    "record_rest",
    "duplicate_key",
    "bool_rest",
    "distinct_keys",
    "vector_tail",
    "tuple_tail",
    "tuple_array",
    "cow_alias",
    "nested_cow_alias",
    "nested_child_alias",
    "escaped_child",
];

const CHECKED: &[&str] = &[
    "invalid_missing",
    "invalid_extra",
    "invalid_rest_key",
    "invalid_rest_value",
    "invalid_duplicate",
    "invalid_range",
    "invalid_empty_string",
    "invalid_tuple_member",
    "invalid_tuple_arity",
    "invalid_vector_tail",
    "invalid_vector_length",
    "invalid_tuple_tail",
    "invalid_tuple_array_key",
    "invalid_mutation",
    "invalid_nested_mutation",
    "invalid_copy_mutation",
    "invalid_duplicate_shape",
    "invalid_bool_key",
    "invalid_rest_newtype",
    "unknown_key",
    "callback_result",
];

#[test]
fn structural_return_proofs_elide_only_statically_valid_checks() {
    let arena = LocalArena::new();
    let program = parse(&arena, RETURNS).expect("the return proof fixtures parse");
    let heap = Heap::new();
    let unit = compile_with_configuration(
        program,
        "/return-proofs.whim",
        &heap,
        CompileConfiguration {
            optimization: OptimizationConfiguration::default(),
            trusted_return_types: false,
        },
    )
    .expect("the return proof fixtures compile");
    verify_unit(&unit).expect("optimized structural returns verify");

    for (names, checked) in [(PROVEN, false), (CHECKED, true)] {
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

#[test]
fn structural_return_proofs_preserve_runtime_errors_and_copy_on_write() {
    let mut source = String::from(RETURNS);
    source.push_str(
        r"
assert!(record() == dict['a' => 1, 'b' => 'hello', 'c' => (1, 2)]);
assert!(record_rest() == dict['a' => 1, 'extra' => 2]);
assert!(duplicate_key() == dict['a' => 1]);
assert!(bool_rest() == dict['a' => 1, true => 2, false => 3]);
assert!(distinct_keys() == dict[1 => 1, '1' => 2]);
assert!(vector_tail() == vec[1, 2, 3]);
assert!(tuple_tail() == (1, 2, 3));
assert!(tuple_array() == (1, 2));
assert!(cow_alias() == dict['a' => 1]);
assert!(nested_cow_alias() == vec[dict['a' => 1]]);
assert!(nested_child_alias() == vec[dict['a' => 1]]);
assert!(escaped_child() == vec[dict['a' => 1]]);
assert!(unknown_key('a') == dict['a' => 1]);
assert!(callback_result(fn(): dict => dict['a' => 1]) == vec[dict['a' => 1]]);
",
    );
    for name in CHECKED {
        let call = match *name {
            "unknown_key" => "unknown_key('wrong')".to_owned(),
            "callback_result" => "callback_result(fn(): dict => dict['a' => 0])".to_owned(),
            name => format!("{name}()"),
        };
        source.push_str(&format!(
            "$caught = false; try {{ {call}; }} catch (Whim\\Unwind\\TypeError $error) {{ $caught = true; }} assert!($caught);\n"
        ));
    }
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(&source, Path::new("/return-proofs.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}
