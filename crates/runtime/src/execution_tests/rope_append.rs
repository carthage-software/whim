use super::run_both_modes;

#[test]
fn rope_accumulators_keep_rows_and_invalidate_cached_hashes() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function join(string $left, string $right): string { return $left . $right; }
#[NeverInline]
function hash_key(string $value): bool { return dict[$value => true][$value]; }
#[NeverInline]
function render(vec<string> $rows): string {
    $html = '<table>';
    foreach ($rows as $row) { $html .= '<tr><td>' . $row . '</td></tr>'; }
    return $html . '</table>';
}
$rows = vec[];
$expected = '<table>';
for ($index = 0; $index < 96; $index++) {
    $row = 'row-' . $index . ':abcdefghijklmnopqrstuvwxyz';
    $rows[] = $row;
    $expected = join($expected, join('<tr><td>' . $row, '</td></tr>'));
}
$expected = join($expected, '</table>');
assert!(render($rows) == $expected);
$value = join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ');
assert!(hash_key($value));
$value .= '0123456789';
assert!(dict['abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789' => true][$value]);
assert!(hash_key($value));
$extra = join('alpha-beta-gamma-delta-', 'epsilon-zeta-eta-theta');
$value .= $extra;
assert!(dict[join('abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789', $extra) => true][$value]);
",
        "/rope-append-rows.whim",
    );
}

#[test]
fn rope_appends_preserve_aliases_and_shared_children() {
    run_both_modes(
        r"
namespace Whim\_Private;
use Whim\Marker\NeverInline;
#[NeverInline]
function join(string $left, string $right): string { return $left . $right; }
#[NeverInline]
function append(string $value, string $extra): string { $value .= $extra; return $value; }
$left = 'abcdefghijklmnopqrstuvwxyz';
$right = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';
$value = join($left, $right);
$alias = $value;
$value .= $right;
assert!($alias == $left . $right);
assert!($value == $left . $right . $right);
$value = join($left, $right);
$value .= $value;
assert!($value == $left . $right . $left . $right);
$value = join($left, $right);
$value .= $left;
assert!($value == $left . $right . $left);
assert!($left == 'abcdefghijklmnopqrstuvwxyz');
$value = join($left, $right);
$part = string_slice($value, 3, 32);
$value .= $part;
assert!($part == 'defghijklmnopqrstuvwxyzABCDEFGHI');
assert!($value == $left . $right . $part);
$value = join($left, $right);
$extra = join($value, 'tail');
$value .= $extra;
assert!($extra == $left . $right . 'tail');
assert!($value == $left . $right . $extra);
$value = join($left, $right);
$extra = join($right, $left);
$value .= $extra;
assert!($extra == $right . $left);
assert!($value == $left . $right . $extra);
assert!(append($alias, '') == $alias);
",
        "/rope-append-aliases.whim",
    );
}

#[test]
fn rope_appends_preserve_slice_targets_scalars_and_newtypes() {
    run_both_modes(
        r"
namespace Whim\_Private;
use Whim\Marker\NeverInline;
newtype Wrapped = string;
#[NeverInline]
function join(string $left, string $right): string { return $left . $right; }
#[NeverInline]
function scalar_append(string $value, mixed $extra): string { $value .= $extra; return $value; }
#[NeverInline]
function loop(string $value, string $extra, int $count): string {
    for ($index = 0; $index < $count; $index++) { $value .= $extra; }
    return $value;
}
$base = join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ');
$value = string_slice($base, 3, 32);
$part = $value;
$value .= '0123456789';
$value .= 'abcdefghij';
assert!($part == 'defghijklmnopqrstuvwxyzABCDEFGHI');
assert!($value == $part . '0123456789abcdefghij');
assert!($base == 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ');
assert!(loop('x', 'y', 5) == 'xyyyyy');
assert!(loop(join('abcdefghijklmnop', 'qrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ'), '!', 4) == $base . '!!!!');
assert!(loop(join('abcdefghijklmnop', 'qrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ'), $base, 2) == $base . $base . $base);
assert!(scalar_append($base, -42) == $base . '-42');
assert!(scalar_append($base, 42u) == $base . '42');
assert!(scalar_append($base, 1.5) == $base . '1.5');
$wrapped = Wrapped(join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'));
assert!($wrapped is Wrapped);
$wrapped .= '0123456789';
assert!(!($wrapped is Wrapped));
assert!($wrapped == $base . '0123456789');
",
        "/rope-append-types.whim",
    );
}
