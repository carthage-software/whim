use super::run_both_modes;

const FIXTURE: &str = include_str!("../../../../tests/_fixtures/uint-numeric-loops.whim");

#[test]
fn uint_loops_preserve_high_bits_overflow_and_exit_state() {
    let mut source = String::from(FIXTURE);
    source.push_str(r"
assert!(sum_lengths('abc', 9223372036854775808u, 4) == (9223372036854775820u, 4));
assert!(sum_lengths('', 18446744073709551615u, 5) == (18446744073709551615u, 5));
assert!(sum_lengths('abc', 42u, 0) == (42u, 0));
assert!(sum_values(vec[9223372036854775808u, 3u, 4u], 0u, 3) == (9223372036854775815u, 3));
assert!(overflow_state(18446744073709551611u, 2u, 4) == (18446744073709551615u, 2));
assert!(overflow_state(18446744073709551615u, 1u, 1) == (18446744073709551615u, 0));
assert!(break_after_write(vec[1u, 2u, 3u, 4u], 9223372036854775808u, 2, 4) == (9223372036854775814u, 2));
assert!(return_after_write(vec[1u, 2u, 3u, 4u], 9223372036854775808u, 2, 4) == 9223372036854775814u);

newtype Wrapped = uint;
$tagged = Wrapped(9223372036854775808u);
assert!(sum_lengths('abc', $tagged, 1) == (9223372036854775811u, 1));
assert!($tagged is Wrapped);
$values = vec[$tagged, 2u, 3u];
assert!(sum_values($values, 1u, 3) == (9223372036854775814u, 3));
assert!($values[0] is Wrapped);

#[Whim\Marker\NeverInline]
function replace_values(vec<uint> $values, int $count): vec<uint> {
    for ($index = 0; $index < $count; $index++) {
        if ($index == 0) { $values[$index] = 3u; }
        else { $values[$index] = 18446744073709551615u; }
    }
    return $values;
}
$original = vec[0u, 0u, 0u];
assert!(replace_values($original, 3) == vec[3u, 18446744073709551615u, 18446744073709551615u]);
assert!($original == vec[0u, 0u, 0u]);
");
    run_both_modes(&source, "/uint-loop-state.whim");
}

#[test]
fn uint_loops_flush_before_unsupported_comparisons_and_arithmetic() {
    let mut source = String::from(FIXTURE);
    source.push_str(
        r"
$values = vec[1u, 2u, 3u];
assert!(equal_after_write($values, 9223372036854775808u, 9223372036854775814u, 3)
    == (9223372036854775814u, 3, true));
assert!(equal_after_write($values, 0u, 6, 3) == (6u, 3, false));
assert!(equal_after_write($values, 0u, 7u, 3) == (6u, 3, false));
assert!(order_after_write($values, 9223372036854775808u, 9223372036854775815u, 3)
    == (9223372036854775814u, 3, true));
assert!(order_after_write(vec[1u], 9007199254740992u, 9007199254740992.0, 1)
    == (9007199254740993u, 1, false));
assert!(arithmetic_after_write($values, 0u, 9223372036854775808u, 1u, 3)
    == (6u, 3, 9223372036854775809u));
assert!(arithmetic_after_write($values, 9223372036854775808u, 1u, 1, 3)
    == (9223372036854775809u, 0, 'the + operator is not defined for uint and int'));
assert!(arithmetic_after_write($values, 0u, 1, 1u, 3)
    == (1u, 0, 'the + operator is not defined for int and uint'));
",
    );
    run_both_modes(&source, "/uint-loop-deoptimization.whim");
}

#[test]
fn uint_loops_keep_high_bits_in_vector_and_dictionary_builds() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function append_values(uint $value, int $count): vec<uint> {
    $values = vec[];
    for ($index = 0; $index < $count; $index++) { $values[] = $value; }
    return $values;
}
#[NeverInline]
function fill_values(uint $value, int $count): dict<int, uint> {
    $values = dict[];
    for ($index = 0; $index < $count; $index++) { $values[$index] = $value; }
    return $values;
}
$vector = append_values(18446744073709551615u, 70000);
assert!(length!($vector) == 70000u);
assert!($vector[0] == 18446744073709551615u);
assert!($vector[65536] == 18446744073709551615u);
assert!($vector[69999] == 18446744073709551615u);
$dictionary = fill_values(9223372036854775808u, 70000);
assert!(length!($dictionary) == 70000u);
assert!($dictionary[0] == 9223372036854775808u);
assert!($dictionary[65536] == 9223372036854775808u);
assert!($dictionary[69999] == 9223372036854775808u);
",
        "/uint-loop-builds.whim",
    );
}
