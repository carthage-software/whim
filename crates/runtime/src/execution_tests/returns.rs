use super::run_both_modes;

#[path = "../../../../tests/_fixtures/return_contracts.rs"]
mod fixtures;

#[test]
fn structural_returns_preserve_errors_and_copy_on_write() {
    let mut source = String::from(fixtures::RETURNS);
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
    for name in fixtures::CHECKED {
        let call = match *name {
            "unknown_key" => "unknown_key('wrong')".to_owned(),
            "callback_result" => "callback_result(fn(): dict => dict['a' => 0])".to_owned(),
            name => format!("{name}()"),
        };
        source.push_str(&format!(
            "$caught = false; try {{ {call}; }} catch (Whim\\Unwind\\TypeError $error) {{ $caught = true; }} assert!($caught);\n"
        ));
    }
    run_both_modes(&source, "/return-proofs.whim");
}
