use super::run_both_modes;

#[test]
fn nested_compound_operators_preserve_results_and_snapshots() {
    for operator in ["+", "-", "*", "/", "%", "**", "&", "|", "^", "<<", ">>"] {
        let source = format!(
            r"
use Whim\Marker\NeverInline;
#[NeverInline]
function update(mixed $left, mixed $right): void {{
    $values = vec[dict['value' => $left]];
    $snapshot = $values;
    $expected = $left {operator} $right;
    $result = ($values[0u]['value'] {operator}= $right);
    assert!($result == $expected);
    assert!(($result is int) == ($expected is int));
    assert!(($result is uint) == ($expected is uint));
    assert!(($result is float) == ($expected is float));
    assert!($values[0]['value'] == $expected);
    assert!($snapshot[0]['value'] == $left);
}}
update(7, 2);
update(7u, 2u);
"
        );
        run_both_modes(&source, "/nested-compound-operators.whim");
    }
    for (operator, left, right) in [
        ("*", "3.5", "2"),
        ("/", "7u", "2"),
        ("**", "2", "-1"),
        ("**", "4.0", "0.5"),
        ("<<", "7u", "2"),
        (">>", "-8", "2u"),
    ] {
        run_both_modes(
            &format!(
                "$values = vec[vec[{left}]]; $result = ($values[0][0] {operator}= {right}); \
                 assert!($result == ({left} {operator} {right})); assert!($values[0][0] == $result);"
            ),
            "/nested-compound-numeric-types.whim",
        );
    }
}

#[test]
fn nested_compound_operator_errors_preserve_values_and_messages() {
    for (operator, left, right) in [
        ("*", "9223372036854775807", "2"),
        ("*", "18446744073709551615u", "2u"),
        ("/", "7", "0"),
        ("%", "7u", "0u"),
        ("**", "2", "64"),
        ("**", "0", "-1"),
        ("&", "1", "1u"),
        ("|", "1.0", "1"),
        ("^", "'a'", "1"),
        ("<<", "1u", "64"),
        (">>", "1", "-1"),
    ] {
        run_both_modes(
            &format!(
                r"
use Whim\Unwind\Error;
$values = vec[dict['value' => {left}]];
$snapshot = $values;
$expected = '';
$actual = '';
try {{ $value = {left}; $value {operator}= {right}; }} catch (Error $error) {{
    $expected = $error->getMessage();
}}
try {{ $values[0]['value'] {operator}= {right}; }} catch (Error $error) {{
    $actual = $error->getMessage();
}}
assert!($expected != '');
assert!($actual == $expected);
assert!($values == $snapshot);
"
            ),
            "/nested-compound-errors.whim",
        );
    }
}

#[test]
fn nested_writes_preserve_snapshots_keys_results_and_type_checks() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function update(dict $state, mixed $key): dict {
    $snapshot = $state;
    $row = $state['rows'][$key];
    $result = ($state['rows'][$key]['count'] += 3);
    assert!($result == 4);
    $state['rows'][$key]['count'] *= 2;
    $state['rows'][$key]['count'] -= 1;
    $state['rows'][$key]['fresh'] = 'added';
    assert!($snapshot['rows'][$key]['count'] == 1);
    assert!($row == dict['count' => 1]);
    assert!($state['rows'][$key]['count'] == 7);
    return $state;
}
foreach (vec[0, 0u, false, 'row', 'a key longer than inline string storage'] as $key) {
    $initial = dict['rows' => dict[$key => dict['count' => 1]]];
    assert!(update($initial, $key)['rows'][$key]['fresh'] == 'added');
    assert!($initial['rows'][$key] == dict['count' => 1]);
}
$matrix = vec[vec[1, 2], vec[3, 4]];
$saved = $matrix;
assert!($matrix is vec<vec<int>>);
$matrix[0u][1] = 'changed';
assert!(!($matrix is vec<vec<int>>));
assert!($saved is vec<vec<int>>);
assert!($saved[0][1u] == 2);
$integers = vec[vec[7]];
assert!($integers is vec<vec<int>>);
$integers[0][0] /= 2;
assert!($integers[0][0] is float);
assert!($integers is vec<vec<float>>);
assert!(!($integers is vec<vec<int>>));
$nested = dict['row' => dict['value' => 1]];
assert!($nested is dict<string, dict<string, int>>);
$nested['row']['value'] = 'changed';
assert!(!($nested is dict<string, dict<string, int>>));
$self = vec[vec[1]];
$self[0][0] = $self;
assert!($self == vec[vec[vec[vec[1]]]]);
$index = 0;
$data = vec[vec[1], vec[2]];
$data[$index++][0] = 9;
assert!($index == 1);
assert!($data == vec[vec[9], vec[2]]);
$unsigned = vec[vec[1u]];
assert!(($unsigned[0][0] += 2u) == 3u);
assert!(($unsigned[0][0] -= 1u) == 2u);
$floating = dict['row' => dict['count' => 1.5]];
assert!(($floating['row']['count'] += 2) == 3.5);
assert!(($floating['row']['count'] -= 0.25) == 3.25);
",
        "/nested-write-snapshots.whim",
    );
}

#[test]
fn nested_write_failures_preserve_values_and_fault_order() {
    run_both_modes(
        r"
use Whim\Unwind\OutOfBoundsError;
use Whim\Unwind\TypeError;
use Whim\Unwind\ArithmeticError;
$data = vec[vec[1]];
$saved = $data;
$caught = 0;
try { $data[0][9] = 2; } catch (OutOfBoundsError $_) { $caught++; }
try { $data[9][0] = 2; } catch (OutOfBoundsError $_) { $caught++; }
try { $data[0]['bad'] = 2; } catch (TypeError $_) { $caught++; }
assert!($data == $saved);
$record = dict['row' => dict['value' => 9223372036854775807]];
$savedRecord = $record;
try { $record['row']['value'] += 1; } catch (ArithmeticError $_) { $caught++; }
assert!($record == $savedRecord);
$unsigned = vec[vec[0u]];
try { $unsigned[0][0] -= 1u; } catch (ArithmeticError $_) { $caught++; }
try { $unsigned[0][0] += 1; } catch (TypeError $_) { $caught++; }
assert!($unsigned == vec[vec[0u]]);
try { $record['missing']['value'] = 2; } catch (OutOfBoundsError $_) { $caught++; }
assert!($record == $savedRecord);
$tuple = (vec[1],);
try { $tuple[0][0] = 2; } catch (TypeError $_) { $caught++; }
try { $tuple[0][8] = 2; } catch (OutOfBoundsError $_) { $caught++; }
try { $tuple[0][0] += 1; } catch (TypeError $_) { $caught++; }
try { $tuple[0][0] += 9223372036854775807; } catch (ArithmeticError $_) { $caught++; }
assert!($tuple == (vec[1],));
$wrapped = vec[(vec[1],)];
try { $wrapped[0][0][8] = 2; } catch (OutOfBoundsError $_) { $caught++; }
assert!($wrapped == vec[(vec[1],)]);
$text = 'abc';
try { $text[0][0] = 'd'; } catch (TypeError $_) { $caught++; }
assert!($text == 'abc');
assert!($caught == 13);
",
        "/nested-write-errors.whim",
    );
}

#[test]
fn nested_reads_preserve_index_evaluation_and_errors() {
    run_both_modes(
        r"
use Whim\Unwind\OutOfBoundsError;
use Whim\Unwind\UndefinedVariableError;
use Whim\Marker\NeverInline;
#[NeverInline]
function read(mixed $data, mixed $first, mixed $second): mixed {
    return $data[$first][$second];
}
assert!(read(vec[(11,)], 0u, 0) == 11);
assert!(read(dict[false => vec[12]], false, 0u) == 12);
assert!(read('abc', 1, 0u) == 'b');
$data = vec[vec[1, 2], vec[3, 4]];
$index = 0;
assert!($data[$index++][$index++] == 2);
assert!($index == 2);
$caught = false;
try { $_ = $data[9][$missing]; } catch (OutOfBoundsError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { $_ = $data[0][$missing]; } catch (UndefinedVariableError $_) { $caught = true; }
assert!($caught);
$object = null;
assert!($object?->data[0][0] == null);
",
        "/nested-read-order.whim",
    );
}
