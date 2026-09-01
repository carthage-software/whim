# Strings, Numbers, and Binary Data

## Byte strings

Whim strings hold bytes, so `Whim\Str` uses byte offsets and byte
lengths.

Inspection functions include `length`, `ord`, `chr`, `byte_at`, `compare`,
`compare_ci`, `search`, `search_last`, `contains`, `starts_with`, and
`ends_with`. Search and containment functions accept a byte offset. The byte
checks are `is_whitespace`, `is_digit`, `is_letter`, `is_alphanumeric`,
`is_hex_digit`, and `is_ascii_punctuation`. A `_ci` suffix means ASCII
case-insensitive matching.

Slice functions include `slice`, `splice`, `chunk`, `split`, `range`,
`before`, `after`, and their last and case-insensitive forms.

Transform functions include ASCII `lowercase`, `uppercase`, `capitalize`,
`reverse`, `repeat`, `rot13`, replacements, prefix and suffix stripping,
padding, trimming, shuffling, word splitting, and wrapping.

```whim
use Whim\Str;

$words = Str\split('one,two,three', ',');
assert!(Str\join(' + ', $words) == 'one + two + three');
assert!(Str\starts_with('whimsical', 'whim'));
assert!(Str\slice('abcdef', 1, 3) == 'bcd');
```

Use `Encoding\UTF8` before treating unknown bytes as Unicode text.

## Unicode text and code points

`Unicode\case_fold` applies full, locale-independent case folding to valid
UTF-8. It can expand one code point into several, such as `\u{df}` into `ss`.
It throws `EncodingException` when the string is not valid UTF-8.

`Unicode\code_point_at` reads a scalar value at a byte offset.
`Unicode\code_point_before` reads the value ending before an offset. They
return `null` at the matching string end and U+FFFD for malformed UTF-8.
`Str\from_code_point` encodes a `Unicode\ScalarValue` as UTF-8.

`Unicode\CodePoint` covers all code points from zero through U+10FFFF.
`Unicode\ScalarValue` excludes the surrogate range, which UTF-8 cannot encode.

The other `Whim\Unicode` functions test integer code points without decoding
a string. They check valid scalar values, whitespace, letters, marks, numbers,
decimal digits, punctuation, symbols, separators, controls, and case. Every
check returns `false` for an invalid code point.

```whim
use Whim\Unicode;

assert!(Unicode\case_fold("Stra\u{df}e") == 'strasse');
assert!(Unicode\code_point_at("\u{1f600}", 0) == 0x1f600);
assert!(Unicode\is_letter(0x4e2d));
assert!(Unicode\is_whitespace(0x3000));
assert!(Unicode\is_punctuation(0x3001));
```

## Integers and floats

`Int\try_parse($text)` and `Float\try_parse($text)` return `null` for invalid
input. They do not accept a partial number.

`Float` also tests NaN, finite, and infinite values. `to_bits` and `from_bits`
convert a 64-bit float to its integer bit pattern. `to_bytes` and `from_bytes`
use an explicit `Binary\Endianness`.

## Math

`Whim\Math` provides checked integer division, absolute value, clamp, square
root, exponent, logarithms, floor, ceiling, round, and trigonometry.

`sum` and `sum_floats` accept iterables. `min`, `max`, `min_by`, and `max_by`
return `null` for no input. `mean` and `median` accept arrays because they need
their size or more than one pass.

`to_base`, `from_base`, and `base_convert` support bases 2 through 36.

The namespace defines integer and float limits plus `NAN`, `INF`, `E`, and
`PI`. Read each limit by its full name: positive minima and lowest signed
values use different constants.

## Ranges

`Whim\Range` represents full, lower-bound, upper-bound, and two-bound integer
ranges. `full`, `from`, `to`, and `between` build them. Range objects expose
their bounds and can create an iterator.

These objects are useful when a runtime value must carry a range. Type ranges
such as `1..=10` remain part of the type system.

## Binary encoding

`Whim\Binary` reads and writes signed and unsigned integers of 8, 16, 32, and
64 bits, plus 32-bit and 64-bit floats. Multi-byte functions require
`Endianness::Big` or `Endianness::Little`.

One-shot `encode_*` functions return bytes; `decode_*` functions read bytes and
check their exact width. `MemoryReader`, `MemoryWriter`, `HandleReader`, and
`HandleWriter` provide moving cursors. Buffered readers report remaining data;
buffered writers return their bytes through `toString()`.

Use binary APIs for protocol fields and file formats. Do not reverse byte
strings by hand.
