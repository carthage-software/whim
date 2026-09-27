use std::path::Path;

use whim_bytecode::instruction::Instruction;
use whim_value::Value;
use whim_value::newtype::NewtypeValueId;
use whim_value::ops;

use super::BytecodeComparison;
use super::comparison_matches;
use super::mixed_integer_counter_step;
use super::step_by;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;

#[test]
fn mixed_steps_match_existing_arithmetic_for_extremes_and_tags() {
    let signed = [i64::MIN, -1, 0, 1, i64::MAX - 1, i64::MAX];
    let unsigned = [
        0,
        1,
        u64::try_from(i64::MAX).unwrap(),
        1 << 63,
        u64::MAX - 1,
        u64::MAX,
    ];
    for signed in signed {
        for unsigned in unsigned {
            let signed = Value::int(signed);
            let unsigned = Value::uint(unsigned);
            for (counter, limit) in [(&signed, &unsigned), (&unsigned, &signed)] {
                for counter_tag in [None, Some(NewtypeValueId(0))] {
                    for limit_tag in [None, Some(NewtypeValueId(1))] {
                        let counter = counter.clone_with_newtype(counter_tag);
                        let limit = limit.clone_with_newtype(limit_tag);
                        for comparison in [
                            BytecodeComparison::Equal,
                            BytecodeComparison::NotEqual,
                            BytecodeComparison::LessThan,
                            BytecodeComparison::LessThanOrEqual,
                            BytecodeComparison::GreaterThan,
                            BytecodeComparison::GreaterThanOrEqual,
                        ] {
                            let actual = mixed_integer_counter_step(comparison, &counter, &limit);
                            match step_by(&counter, 1) {
                                Ok(expected) => {
                                    let (next, matches) = actual.expect("a valid mixed step");
                                    assert!(ops::equals(&next, &expected));
                                    assert_eq!(next.newtype_id(), None);
                                    assert_eq!(
                                        Some(matches),
                                        comparison_matches(comparison, &expected, &limit).ok(),
                                    );
                                }
                                Err(_) => assert!(actual.is_none()),
                            }
                            assert_eq!(counter.newtype_id(), counter_tag);
                            assert_eq!(limit.newtype_id(), limit_tag);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn unmatched_and_aliased_operands_keep_the_original_path() {
    let values = [
        Value::null(),
        Value::bool(false),
        Value::int(-1),
        Value::uint(0),
        Value::float(0.5),
    ];
    for counter in &values {
        assert!(
            mixed_integer_counter_step(BytecodeComparison::LessThan, counter, counter).is_none()
        );
        for limit in &values {
            if (counter.is_int() && limit.is_uint()) || (counter.is_uint() && limit.is_int()) {
                continue;
            }
            assert!(
                mixed_integer_counter_step(BytecodeComparison::LessThan, counter, limit).is_none()
            );
        }
    }
}

#[test]
fn mixed_counter_loops_keep_types_overflow_state_and_fallback_errors() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(
            r"
use Whim\Marker\NeverInline;
use Whim\Unwind\Throwable;
newtype Signed = int;
newtype Unsigned = uint;
#[NeverInline]
function visit(): int { return 1; }
#[NeverInline]
function signed(int $start, uint $limit): (int, int, bool) {
    $counter = $start;
    $visits = 0;
    try {
        for (; $counter < $limit; $counter++) { $visits += visit(); }
    } catch (Whim\Unwind\OverflowError $_) { return ($counter, $visits, true); }
    return ($counter, $visits, false);
}
#[NeverInline]
function unsigned(uint $start, int $limit): (uint, int) {
    $counter = $start;
    $visits = 0;
    for (; $counter < $limit; $counter++) { $visits += visit(); }
    return ($counter, $visits);
}
#[NeverInline]
function unsigned_greater(uint $counter, int $limit): (uint, int, bool) {
    $visits = 0;
    try {
        for (; $counter > $limit; $counter++) { $visits += visit(); }
    } catch (Whim\Unwind\OverflowError $_) { return ($counter, $visits, true); }
    return ($counter, $visits, false);
}
#[NeverInline]
function changed_limit(int $counter, mixed $replacement): (int, bool) {
    $limit = 18446744073709551615u;
    try {
        for (; $counter < $limit; $counter++) { $limit = $replacement; }
    } catch (Whim\Unwind\IncompatibleOperandsError $_) { return ($counter, true); }
    return ($counter, false);
}
#[NeverInline]
function checked(mixed $counter, mixed $limit): (mixed, bool) {
    try {
        do { $counter++; } while ($counter < $limit);
    } catch (Throwable $_) { return ($counter, true); }
    return ($counter, false);
}
#[NeverInline]
function equal(mixed $counter, mixed $limit, mixed $replacement): mixed {
    for (; $counter == $limit; $counter++) { $limit = $replacement; }
    return $counter;
}
assert!(signed(-2, 2u) == (2, 4, false));
assert!(signed(0, 0u) == (0, 0, false));
assert!(signed(9223372036854775805, 9223372036854775807u) == (9223372036854775807, 2, false));
assert!(signed(9223372036854775806, 9223372036854775808u) == (9223372036854775807, 2, true));
assert!(signed(9223372036854775807, 18446744073709551615u) == (9223372036854775807, 1, true));
assert!(unsigned(0u, -1) == (0u, 0));
assert!(unsigned(0u, 2) == (2u, 2));
assert!(unsigned(9223372036854775808u, 9223372036854775807) == (9223372036854775808u, 0));
assert!(unsigned_greater(18446744073709551614u, -1) == (18446744073709551615u, 2, true));
assert!(changed_limit(3, vec[]) == (4, true));
$start = Signed(-2);
$limit = Unsigned(2u);
assert!(signed($start, $limit) == (2, 4, false));
assert!($start is Signed && $limit is Unsigned);
$stepped = checked(Signed(-1), Unsigned(0u));
assert!($stepped == (0, false) && !($stepped[0] is Signed));
$stepped = checked(Unsigned(0u), Signed(0));
assert!($stepped == (1u, false) && !($stepped[0] is Unsigned));
assert!(checked(18446744073709551615u, -1) == (18446744073709551615u, true));
assert!(checked(9223372036854775807, 18446744073709551615u) == (9223372036854775807, true));
assert!(checked(Unsigned(18446744073709551615u), -1)[0] is Unsigned);
assert!(checked(Signed(9223372036854775807), 18446744073709551615u)[0] is Signed);
assert!(checked(3, vec[]) == (4, true));
assert!(checked(false, 1u) == (false, true));
assert!(checked(0.5, 2u) == (2.5, false));
assert!(equal(0, 0, 1u) == 1);
assert!(equal(0u, 0u, 1) == 1u);
",
            Path::new("/mixed-counter-steps.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        if optimize {
            for name in [
                b"signed".as_slice(),
                b"unsigned",
                b"unsigned_greater",
                b"changed_limit",
                b"equal",
            ] {
                let function = engine
                    .tables
                    .functions
                    .iter()
                    .find(|function| function.name.as_bytes() == name)
                    .unwrap();
                // SAFETY: the idle engine owns this function chunk for the read.
                let chunk = unsafe { function.chunk.as_ref() };
                assert!(
                    chunk
                        .code
                        .iter()
                        .any(|instruction| matches!(instruction, Instruction::CounterLoop { .. })),
                    "{name:?}: {:?}",
                    chunk.code
                );
            }
        }
    }
}
