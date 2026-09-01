use std::cmp::Ordering;

use crate::bytecode::instruction::operands::Comparison as BytecodeComparison;
use crate::value::ops::compare_int_float;
use crate::vm::Fault;
use crate::vm::numeric_loop::NumericKind;
use crate::vm::numeric_loop::NumericValue;
use crate::vm::unreachable_invariant;

pub(in crate::vm::numeric_loop) fn stepped_loop_iterations(
    comparison: BytecodeComparison,
    counter: i64,
    limit: i64,
    step: i64,
) -> Option<usize> {
    let count = match comparison {
        BytecodeComparison::LessThan if step > 0 => {
            let distance = i128::from(limit) - i128::from(counter);
            (distance + i128::from(step) - 1) / i128::from(step)
        }
        BytecodeComparison::LessThanOrEqual if step > 0 => {
            (i128::from(limit) - i128::from(counter)) / i128::from(step) + 1
        }
        BytecodeComparison::GreaterThan if step < 0 => {
            let magnitude = -i128::from(step);
            let distance = i128::from(counter) - i128::from(limit);
            (distance + magnitude - 1) / magnitude
        }
        BytecodeComparison::GreaterThanOrEqual if step < 0 => {
            (i128::from(counter) - i128::from(limit)) / -i128::from(step) + 1
        }
        _ => return None,
    };

    usize::try_from(count).ok().filter(|count| *count != 0)
}

#[inline(always)]
pub(in crate::vm::numeric_loop) fn equals_numeric(
    left: NumericValue,
    right: NumericValue,
) -> Option<bool> {
    match (left.kind, right.kind) {
        (NumericKind::Int, NumericKind::Int) | (NumericKind::Bool, NumericKind::Bool) => {
            Some(left.bits == right.bits)
        }
        (NumericKind::Float, NumericKind::Float) => Some(left.float_value() == right.float_value()),
        (NumericKind::Other, _) | (_, NumericKind::Other) => None,
        _ => Some(false),
    }
}

#[inline(always)]
pub(in crate::vm::numeric_loop) fn int_comparison_matches_any(
    comparison: BytecodeComparison,
    left: i64,
    right: i64,
) -> bool {
    match comparison {
        BytecodeComparison::Equal => left == right,
        BytecodeComparison::NotEqual => left != right,
        BytecodeComparison::LessThan => left < right,
        BytecodeComparison::LessThanOrEqual => left <= right,
        BytecodeComparison::GreaterThan => left > right,
        BytecodeComparison::GreaterThanOrEqual => left >= right,
    }
}

pub(in crate::vm::numeric_loop) fn add(
    left: NumericValue,
    right: NumericValue,
) -> Option<Result<NumericValue, Fault>> {
    match (left.kind, right.kind) {
        (NumericKind::Int, NumericKind::Int) => {
            let left = left.int_value();
            let right = right.int_value();
            Some(
                left.checked_add(right)
                    .map(NumericValue::int)
                    .ok_or(if left >= 0 {
                        Fault::Overflow
                    } else {
                        Fault::Underflow
                    }),
            )
        }
        (NumericKind::Float, NumericKind::Float) => Some(Ok(NumericValue::float(
            left.float_value() + right.float_value(),
        ))),
        (NumericKind::Int, NumericKind::Float) => Some(Ok(NumericValue::float(
            left.int_value() as f64 + right.float_value(),
        ))),
        (NumericKind::Float, NumericKind::Int) => Some(Ok(NumericValue::float(
            left.float_value() + right.int_value() as f64,
        ))),
        (NumericKind::Other | NumericKind::Bool, _)
        | (_, NumericKind::Other | NumericKind::Bool) => None,
    }
}

pub(in crate::vm::numeric_loop) fn subtract(
    left: NumericValue,
    right: NumericValue,
) -> Option<Result<NumericValue, Fault>> {
    match (left.kind, right.kind) {
        (NumericKind::Int, NumericKind::Int) => {
            let left = left.int_value();
            let right = right.int_value();
            Some(
                left.checked_sub(right)
                    .map(NumericValue::int)
                    .ok_or(if left >= 0 {
                        Fault::Overflow
                    } else {
                        Fault::Underflow
                    }),
            )
        }
        (NumericKind::Float, NumericKind::Float) => Some(Ok(NumericValue::float(
            left.float_value() - right.float_value(),
        ))),
        (NumericKind::Int, NumericKind::Float) => Some(Ok(NumericValue::float(
            left.int_value() as f64 - right.float_value(),
        ))),
        (NumericKind::Float, NumericKind::Int) => Some(Ok(NumericValue::float(
            left.float_value() - right.int_value() as f64,
        ))),
        (NumericKind::Other | NumericKind::Bool, _)
        | (_, NumericKind::Other | NumericKind::Bool) => None,
    }
}

pub(in crate::vm::numeric_loop) fn multiply(
    left: NumericValue,
    right: NumericValue,
) -> Option<Result<NumericValue, Fault>> {
    match (left.kind, right.kind) {
        (NumericKind::Int, NumericKind::Int) => {
            let left = left.int_value();
            let right = right.int_value();
            Some(left.checked_mul(right).map(NumericValue::int).ok_or(
                if (left > 0) == (right > 0) {
                    Fault::Overflow
                } else {
                    Fault::Underflow
                },
            ))
        }
        (NumericKind::Float, NumericKind::Float) => Some(Ok(NumericValue::float(
            left.float_value() * right.float_value(),
        ))),
        (NumericKind::Int, NumericKind::Float) => Some(Ok(NumericValue::float(
            left.int_value() as f64 * right.float_value(),
        ))),
        (NumericKind::Float, NumericKind::Int) => Some(Ok(NumericValue::float(
            left.float_value() * right.int_value() as f64,
        ))),
        (NumericKind::Other | NumericKind::Bool, _)
        | (_, NumericKind::Other | NumericKind::Bool) => None,
    }
}

#[inline(always)]
pub(in crate::vm::numeric_loop) fn comparison_matches_numeric(
    comparison: BytecodeComparison,
    left: NumericValue,
    right: NumericValue,
) -> Option<bool> {
    if left.kind == NumericKind::Float && right.kind == NumericKind::Float {
        return Some(float_ordered_comparison_matches(
            comparison,
            left.float_value(),
            right.float_value(),
        ));
    }

    comparison_matches_non_float(comparison, left, right)
}

#[cold]
#[inline(never)]
fn comparison_matches_non_float(
    comparison: BytecodeComparison,
    left: NumericValue,
    right: NumericValue,
) -> Option<bool> {
    let ordering = match (left.kind, right.kind) {
        (NumericKind::Int, NumericKind::Int) => Some(left.int_value().cmp(&right.int_value())),
        // SAFETY: the surrounding invariant makes this path unreachable.
        (NumericKind::Float, NumericKind::Float) => unsafe {
            unreachable_invariant("float pairs take their direct comparison path")
        },
        (NumericKind::Int, NumericKind::Float) => {
            compare_int_float(left.int_value(), right.float_value())
        }
        (NumericKind::Float, NumericKind::Int) => {
            compare_int_float(right.int_value(), left.float_value()).map(Ordering::reverse)
        }
        (NumericKind::Other | NumericKind::Bool, _)
        | (_, NumericKind::Other | NumericKind::Bool) => return None,
    };
    Some(ordered_comparison_matches(comparison, ordering))
}

#[inline(always)]
pub(in crate::vm::numeric_loop) fn step_counter(
    comparison: BytecodeComparison,
    counter: NumericValue,
    limit: NumericValue,
) -> Option<Result<(NumericValue, bool), Fault>> {
    match (counter.kind, limit.kind) {
        (NumericKind::Int, NumericKind::Int) => {
            let counter = counter.int_value();
            let limit = limit.int_value();
            let Some(next) = counter.checked_add(1) else {
                return Some(Err(if counter >= 0 {
                    Fault::Overflow
                } else {
                    Fault::Underflow
                }));
            };
            Some(Ok((
                NumericValue::int(next),
                ordered_comparison_matches(comparison, Some(next.cmp(&limit))),
            )))
        }
        (NumericKind::Float, NumericKind::Float) => {
            let counter = counter.float_value();
            let limit = limit.float_value();
            let next = counter + 1.0;
            Some(Ok((
                NumericValue::float(next),
                ordered_comparison_matches(comparison, next.partial_cmp(&limit)),
            )))
        }
        (NumericKind::Int, NumericKind::Float) => {
            let counter = counter.int_value();
            let limit = limit.float_value();
            let Some(next) = counter.checked_add(1) else {
                return Some(Err(if counter >= 0 {
                    Fault::Overflow
                } else {
                    Fault::Underflow
                }));
            };
            Some(Ok((
                NumericValue::int(next),
                ordered_comparison_matches(comparison, compare_int_float(next, limit)),
            )))
        }
        (NumericKind::Float, NumericKind::Int) => {
            let counter = counter.float_value();
            let limit = limit.int_value();
            let next = counter + 1.0;
            Some(Ok((
                NumericValue::float(next),
                ordered_comparison_matches(
                    comparison,
                    compare_int_float(limit, next).map(Ordering::reverse),
                ),
            )))
        }
        (NumericKind::Other | NumericKind::Bool, _)
        | (_, NumericKind::Other | NumericKind::Bool) => None,
    }
}

#[inline(always)]
pub(in crate::vm::numeric_loop) fn ordered_comparison_matches(
    comparison: BytecodeComparison,
    ordering: Option<Ordering>,
) -> bool {
    match comparison {
        BytecodeComparison::LessThan => matches!(ordering, Some(Ordering::Less)),
        BytecodeComparison::LessThanOrEqual => {
            matches!(ordering, Some(Ordering::Less | Ordering::Equal))
        }
        BytecodeComparison::GreaterThan => matches!(ordering, Some(Ordering::Greater)),
        BytecodeComparison::GreaterThanOrEqual => {
            matches!(ordering, Some(Ordering::Greater | Ordering::Equal))
        }
        // SAFETY: the surrounding invariant makes this path unreachable.
        BytecodeComparison::Equal | BytecodeComparison::NotEqual => unsafe {
            unreachable_invariant("numeric loops use ordered comparisons")
        },
    }
}

#[inline(always)]
pub(in crate::vm::numeric_loop) fn float_ordered_comparison_matches(
    comparison: BytecodeComparison,
    left: f64,
    right: f64,
) -> bool {
    match comparison {
        BytecodeComparison::LessThan => left < right,
        BytecodeComparison::LessThanOrEqual => left <= right,
        BytecodeComparison::GreaterThan => left > right,
        BytecodeComparison::GreaterThanOrEqual => left >= right,
        // SAFETY: the surrounding invariant makes this path unreachable.
        BytecodeComparison::Equal | BytecodeComparison::NotEqual => unsafe {
            unreachable_invariant("numeric loops use ordered comparisons")
        },
    }
}

#[inline(always)]
pub(in crate::vm::numeric_loop) fn int_ordered_comparison_matches(
    comparison: BytecodeComparison,
    left: i64,
    right: i64,
) -> bool {
    match comparison {
        BytecodeComparison::LessThan => left < right,
        BytecodeComparison::LessThanOrEqual => left <= right,
        BytecodeComparison::GreaterThan => left > right,
        BytecodeComparison::GreaterThanOrEqual => left >= right,
        // SAFETY: the surrounding invariant makes this path unreachable.
        BytecodeComparison::Equal | BytecodeComparison::NotEqual => unsafe {
            unreachable_invariant("numeric loops use ordered comparisons")
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::instruction::operands::Comparison as BytecodeComparison;
    use crate::vm::Fault;
    use crate::vm::numeric_loop::NumericValue;
    use crate::vm::numeric_loop::arithmetic::add;
    use crate::vm::numeric_loop::arithmetic::comparison_matches_numeric;
    use crate::vm::numeric_loop::arithmetic::stepped_loop_iterations;

    #[test]
    fn stepped_loop_counts_match_strict_and_inclusive_bounds() {
        assert_eq!(
            stepped_loop_iterations(BytecodeComparison::LessThan, 1, 10, 3),
            Some(3)
        );
        assert_eq!(
            stepped_loop_iterations(BytecodeComparison::LessThanOrEqual, 1, 10, 3),
            Some(4)
        );
        assert_eq!(
            stepped_loop_iterations(BytecodeComparison::GreaterThan, 10, 1, -3),
            Some(3)
        );
        assert_eq!(
            stepped_loop_iterations(BytecodeComparison::GreaterThanOrEqual, 10, 1, -3),
            Some(4)
        );
    }

    #[test]
    fn stepped_loop_rejects_zero_wrong_way_and_empty_steps() {
        assert_eq!(
            stepped_loop_iterations(BytecodeComparison::LessThan, 0, 10, 0),
            None
        );
        assert_eq!(
            stepped_loop_iterations(BytecodeComparison::LessThan, 10, 0, 1),
            None
        );
        assert_eq!(
            stepped_loop_iterations(BytecodeComparison::GreaterThan, 0, 10, -1),
            None
        );
    }

    #[test]
    fn numeric_arithmetic_reports_overflow_and_type_misses() {
        assert!(matches!(
            add(NumericValue::int(i64::MAX), NumericValue::int(1)),
            Some(Err(Fault::Overflow))
        ));
        assert!(add(NumericValue::bool(true), NumericValue::int(1)).is_none());
    }

    #[test]
    fn numeric_comparisons_cover_mixed_ints_and_floats() {
        assert_eq!(
            comparison_matches_numeric(
                BytecodeComparison::LessThan,
                NumericValue::int(2),
                NumericValue::float(2.5),
            ),
            Some(true)
        );
        assert_eq!(
            comparison_matches_numeric(
                BytecodeComparison::GreaterThanOrEqual,
                NumericValue::float(2.0),
                NumericValue::int(2),
            ),
            Some(true)
        );
        assert_eq!(
            comparison_matches_numeric(
                BytecodeComparison::GreaterThan,
                NumericValue::int(9_007_199_254_740_993),
                NumericValue::float(9_007_199_254_740_992.0),
            ),
            Some(true)
        );
        assert_eq!(
            comparison_matches_numeric(
                BytecodeComparison::LessThan,
                NumericValue::float(9_007_199_254_740_992.0),
                NumericValue::int(9_007_199_254_740_993),
            ),
            Some(true)
        );
    }
}
