use super::run_both_modes;

const FIXTURE: &str = include_str!("../../../../tests/_fixtures/nominal-refinement.whim");

#[test]
fn nominal_tests_preserve_patterns_copies_joins_and_mutation() {
    let mut source = String::from(FIXTURE);
    source.push_str(
        r"
$first = new Number(7);
$second = new Number(9);
assert!(evaluate(new Sum($first, new Sum($first, $second))) == 23);
assert!(immediate($first) == 8);
assert!(immediate(new Text('x')) == 0);
assert!(aliases($first) == 14);
assert!(foreach_values(vec[$first, new Sum($first, $second), $second]) == 18);
assert!(selected($first, $second, true) == 7);
assert!(selected($first, $second, false) == 9);
assert!(selected(new Text('x'), $second, true) == 0);
assert!(after_call($first) == 41);
assert!($first->value == 41);
assert!(open_class(new OpenNumber(12)) == 12);
assert!(generic_class(new Box::<int>(13)) == 13);
assert!(generic_class(new Box::<string>('x')) == 0);
$hidden = new Hidden(5);
assert!($hidden->match_value($hidden) == 0);
",
    );
    run_both_modes(&source, "/nominal-refinement-values.whim");
}

#[test]
fn nominal_tests_do_not_survive_reassignment_failed_paths_or_loop_iterations() {
    let mut source = String::from(FIXTURE);
    source.push_str(
        r"
$number = new Number(7);
$text = new Text('long enough to require a counted string');
assert!(!overwritten_test($number));
assert!(!overwritten_test($text));
assert!(failed_test($number) == null);
assert!(failed_test($text) == $text->value);
assert!(one_path($text, false) == $text->value);
assert!(one_path($text, true) == null);
assert!(different_classes($number, $text, true) == 7);
assert!(different_classes($number, $text, false) == $text->value);
assert!(different_iterations(vec[$number, $text, new Number(9), new Text('x')]) == '7' . $text->value . '9x');
$caught = false;
try { discard!(saved_test($number, $text)); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
$caught = false;
try { discard!(reassigned($number, $text)); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
",
    );
    run_both_modes(&source, "/nominal-refinement-paths.whim");
}

#[test]
fn nominal_tests_keep_mutable_shape_and_return_checks() {
    let mut source = String::from(FIXTURE);
    source.push_str(
        r"
$item = new Slot(7);
$caught = false;
try { changed_shape($item); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
assert!($item->value == 'wrong');
$item->value = 7;
$caught = false;
try { discard!(changed_return($item)); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
",
    );
    run_both_modes(&source, "/nominal-refinement-shapes.whim");
}
