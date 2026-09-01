# Strings

A Whim string is a sequence of bytes. It may contain text, encoded data, or
arbitrary binary data.

## Single quotes

Single quotes do not interpolate variables. They recognize `\\` and `\'`.
Other backslash pairs stay as written:

```whim
$name = 'Ada';
assert!('Hello, $name' == 'Hello, $name');
assert!('it\'s' == "it's");
assert!('\n' == "\\n");
```

Use single quotes when the value needs no interpolation or control-byte
escape.

## Double quotes

Double quotes support escapes and interpolation:

```whim
$name = 'Ada';
$next = 41;

assert!("Hello, $name" == 'Hello, Ada');
assert!("answer: {$next + 1}" == 'answer: 42');
```

The short form accepts one variable. Braces accept any expression. An
interpolated value must be a string, int, or float.

Escape `$`, `{`, or `}` when you need the byte itself:

```whim
assert!("\$name \{value\}" == '$name {value}');
```

## Escape sequences

Double-quoted strings support these escapes:

| Escape | Byte or text |
| --- | --- |
| `\\` | backslash |
| `\"` | double quote |
| `\$`, `\{`, `\}` | interpolation marker as text |
| `\n`, `\r`, `\t` | line feed, carriage return, tab |
| `\v`, `\f`, `\e` | vertical tab, form feed, escape |
| `\xH`, `\xHH` | one byte from one or two hex digits |
| `\O`, `\OO`, `\OOO` | one byte from up to three octal digits |
| `\u{H...}` | one Unicode scalar encoded as UTF-8 |

An octal escape must fit in one byte. A Unicode escape cannot name a surrogate
or a value above `10FFFF`.

An unknown escape keeps its backslash:

```whim
assert!("\x" == '\x');
```

## Length and indexing

`length!` counts bytes. Indexing returns a one-byte string:

```whim
$text = 'abc';
assert!(length!($text) == 3);
assert!($text[1] == 'b');
```

An index must be an in-range integer. An invalid index throws
`OutOfBoundsError`. Strings are immutable, so indexed assignment fails.

## Concatenation

`.` joins strings, ints, and floats:

```whim
assert!('count=' . 3 == 'count=3');
assert!('value=' . 1.5 == 'value=1.5');
```

It does not convert bool, null, arrays, objects, or callables. Use an explicit
conversion for those values.

## Text and bytes

`Whim\Str` works on bytes. Its case functions handle ASCII. Use
`Whim\Encoding\UTF8` to check or repair UTF-8, and use `Whim\Binary` for fixed
binary formats.
