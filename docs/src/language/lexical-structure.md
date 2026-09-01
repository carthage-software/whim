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
comment. The sequence `#[` starts an attribute.

## Shebang line

A file may start with a Unix shebang:

```text
#!/usr/bin/env whim
```

The shebang must start at byte zero. Whim treats `#` anywhere else as an error
unless it begins an attribute.

## Number literals

Integers use signed 64-bit values. The source forms are:

```whim
assert!(42 == 4_2);
assert!(0xff == 255);
assert!(0b1010 == 10);
assert!(0o755 == 493);
```

Underscores may split digits. A decimal integer cannot start with `0` unless it
is zero. Write `0o` for octal.

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
