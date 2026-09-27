use std::path::Path;

use whim_bytecode::instruction::Instruction;
use whim_value::ValueView;

use super::run_both_modes;
use crate::engine::Engine;
use crate::engine::EngineConfiguration;

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

#[test]
fn inline_string_appends_keep_unique_results_flat() {
    let source = r"
use Whim\Marker\NeverInline;
#[NeverInline]
function join(string $left, string $right): string { return $left . $right; }
#[NeverInline]
function inline_bytes(): string {
    return Whim\Str\chr(0) . Whim\Str\chr(255) . Whim\Str\chr(128);
}
#[NeverInline]
function append_repeated(): string {
    $text = '';
    for ($index = 0; $index < 24; $index++) {
        $extra = inline_bytes();
        $text .= $extra;
    }
    return $text;
}
#[NeverInline]
function append_flat(): string {
    $text = join('abcdefghijklmnop', 'qrstuvwxyzABCDEF');
    $extra = inline_bytes();
    $text .= $extra;
    return $text;
}
#[NeverInline]
function append_rope(): string {
    $text = join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ');
    $extra = inline_bytes();
    $text .= $extra;
    return $text;
}
final class InlineResults {
    public static string $repeated = '';
    public static string $flat = '';
    public static string $rope = '';
    public static string $inline = '';
}
InlineResults::$repeated = append_repeated();
InlineResults::$flat = append_flat();
InlineResults::$rope = append_rope();
InlineResults::$inline = inline_bytes();
";
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/inline-string-appends.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let class = engine
            .tables
            .classes
            .iter()
            .find(|class| class.name.as_bytes() == b"InlineResults")
            .unwrap();
        let values = class.statics.borrow();
        for (name, expected) in [
            (b"repeated".as_slice(), [0, 255, 128].repeat(24)),
            (
                b"flat",
                [
                    b"abcdefghijklmnopqrstuvwxyzABCDEF".as_slice(),
                    &[0, 255, 128],
                ]
                .concat(),
            ),
            (
                b"rope",
                [
                    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ".as_slice(),
                    &[0, 255, 128],
                ]
                .concat(),
            ),
        ] {
            let slot = class
                .statics_info
                .iter()
                .position(|property| property.name.as_bytes() == name)
                .unwrap();
            let ValueView::String(string) = values[slot].transparent() else {
                panic!("{name:?}: the result exceeds inline storage");
            };
            assert!(string.is_flat(), "optimization {optimize}: {name:?}");
            assert_eq!(string.flatten(), expected);
            assert!(values[slot].newtype_id().is_none());
        }
        let inline = class
            .statics_info
            .iter()
            .position(|property| property.name.as_bytes() == b"inline")
            .unwrap();
        assert!(matches!(
            values[inline].transparent(),
            ValueView::ShortString(_)
        ));
        for name in [
            b"append_repeated".as_slice(),
            b"append_flat",
            b"append_rope",
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
                chunk.code.iter().any(|instruction| {
                    matches!(instruction, Instruction::Concatenate { destination, left, right }
                        if destination == left && left != right)
                }),
                "optimization {optimize}: {name:?}: {:?}",
                chunk.code
            );
        }
    }
}

#[test]
fn inline_string_appends_preserve_aliases_tags_and_overlaps() {
    run_both_modes(
        r#"
use Whim\Marker\NeverInline;
newtype Wrapped = string;
#[NeverInline]
function join(string $left, string $right): string { return $left . $right; }
#[NeverInline]
function inline_bytes(): string {
    return Whim\Str\chr(0) . Whim\Str\chr(255) . Whim\Str\chr(128);
}
#[NeverInline]
function hash_key(string $value): bool { return dict[$value => true][$value]; }
#[NeverInline]
function append_to_right(string $left, string $right): string {
    $right = $left . $right;
    return $right;
}
$extra = inline_bytes();
assert!($extra == "\0\xff\x80");
foreach (vec[
    join('abcdefghijklmnop', 'qrstuvwxyzABCDEF'),
    join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'),
] as $value) {
    $alias = $value;
    $value .= $extra;
    assert!($value == $alias . $extra);
    assert!(length!($value) == length!($alias) + 3u);
}
$base = join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ');
$slice = Whim\_Private\string_slice($base, 3, 32);
$alias = $slice;
$slice .= $extra;
assert!($alias == 'defghijklmnopqrstuvwxyzABCDEFGHI');
assert!($slice == $alias . $extra);
assert!($base == 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ');
assert!(append_to_right($base, $extra) == $base . $extra);
$short = inline_bytes();
$short .= $short;
assert!($short == "\0\xff\x80\0\xff\x80");
$long = join('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ');
$long .= $long;
assert!($long == $base . $base);
$wrapped = Wrapped(join('abcdefghijklmnop', 'qrstuvwxyzABCDEF'));
$wrapped .= $extra;
assert!(!($wrapped is Wrapped));
assert!($wrapped == 'abcdefghijklmnopqrstuvwxyzABCDEF' . $extra);
$tagged_extra = Wrapped(inline_bytes());
$value = join('abcdefghijklmnop', 'qrstuvwxyzABCDEF');
assert!(hash_key($value));
$value .= $tagged_extra;
assert!(!($value is Wrapped) && $tagged_extra is Wrapped);
assert!($value == 'abcdefghijklmnopqrstuvwxyzABCDEF' . $extra);
assert!(dict['abcdefghijklmnopqrstuvwxyzABCDEF' . $extra => true][$value]);
$empty = Whim\_Private\string_slice($extra, 0, 0);
$value .= $empty;
assert!($value == 'abcdefghijklmnopqrstuvwxyzABCDEF' . $extra);
$wrapped = Wrapped(join('abcdefghijklmnop', 'qrstuvwxyzABCDEF'));
$wrapped .= $empty;
assert!(!($wrapped is Wrapped));
assert!($wrapped == 'abcdefghijklmnopqrstuvwxyzABCDEF');
"#,
        "/inline-string-append-aliases.whim",
    );
}
