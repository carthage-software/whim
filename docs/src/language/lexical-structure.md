# Source Text and Comments

A Whim source file contains UTF-8 text. The usual file suffix is `.whim`.
ASCII spaces, tabs, line feeds, carriage returns, vertical tabs, and form feeds
separate tokens but have no other meaning. The formatter uses two spaces for
each level by default.

## Identifiers

An identifier starts with an ASCII letter, `_`, or a non-ASCII UTF-8 byte.
Later bytes may also be decimal digits. A backslash joins namespace segments.

```whim
$answer2 = 42;

function café(): string {
  return 'coffee';
}

assert!(café() == 'coffee');
```

Whim compares names by their UTF-8 bytes. It does not fold case or normalize
Unicode. Two spellings that look alike may still name different symbols.

## Statements and blocks

Most simple statements end with `;`:

```whim
$answer = 42;
write_line!($answer);
```

A block uses braces and does not take a trailing semicolon:

```whim
if (true) {
  write_line!('inside the block');
}
```

Control-flow headers use parentheses. This applies to `if`, `while`, `for`,
`foreach`, and `catch`.

## Comments

Whim has line comments, block comments, and doc comments:

```whim
// This comment ends with the line.

/* This comment may
   span several lines. */

/** Returns the answer. */
function answer(): int {
  return 42;
}
```

A doc comment belongs to the declaration that follows it. `#` does not start a
comment. The tokens `#[`, `#![`, and `#{` start a declaration attribute list,
a file attribute list, and an object shape, respectively. Each is a single
token: whitespace or comments cannot separate its characters.

## Shebang line

A file may start with a Unix shebang:

```text
#!/usr/bin/env whim
```

The shebang must start at byte zero. Whim treats `#` anywhere else as an error
unless it begins `#[`, `#![`, or `#{`.

## Number literals

Unsuffixed integers use signed 64-bit values. The source forms are:

```whim
assert!(42 == 4_2);
assert!(0xff == 255);
assert!(0b1010 == 10);
assert!(0o755 == 493);
```

Underscores may split digits. A decimal integer cannot start with `0` unless it
is zero. Write `0o` for octal.

Append `u` or `U` for an unsigned 64-bit integer. Both spellings have the same
meaning in every base. The optional `i` or `I` suffix marks a signed integer;
it has the same meaning as no suffix.

```whim
assert!(42u == 42U);
assert!(42 == 42i);
assert!(42i == 42I);
assert!(0xffu == 255u);
assert!(0b1010U == 10u);
assert!(0o755u == 493u);
assert!(18_446_744_073_709_551_615u == Whim\Math\UINT_MAX);
```

A suffix follows the digits without a gap. Float literals cannot take an
integer suffix. An out-of-range literal is a compile error; an unsuffixed
literal does not become unsigned when it exceeds the signed maximum.
The compiler rejects a negated nonzero unsigned literal such as `-24u`.
`-0u` is valid and equals `0u`.

Floats use decimal digits and may use an exponent. A decimal point may have
digits on only one side:

```whim
assert!(1.5 == 15e-1);
assert!(.5 == 0.5);
assert!(5. == 5.0);
assert!(1_000.0 == 1000.0);
```

The runtime stores floats as IEEE 754 double-precision values.

## String tokens

[Strings](strings.md) explains single-quoted and double-quoted strings.
Backticks do not quote strings.

## Top-level declarations

Functions, classes, interfaces, enums, constants, aliases, and newtypes may
appear only at file or namespace scope. They cannot appear inside a function or
control-flow block.

File-scope statements form that file's executable body. A loaded file may both
declare symbols and run code.
