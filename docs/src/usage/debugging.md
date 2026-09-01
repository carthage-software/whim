# Debugging and Tests

Whim has language constructs for checks and debug output.

## assert!

`assert!($condition)` requires a boolean. It throws `AssertionError` when the
value is false.

```whim
$answer = 6 * 7;
assert!($answer == 42);
```

The error shows the source expression. A comparison also shows its left and
right values.

Assertions always run. Whim has no release mode that removes them.

## debug!

`debug!($value)` prints the call site and a detailed form of the value.

```whim
$numbers = vec[1, 2, 3];
debug!($numbers);
```

Strings show their byte length and escaped bytes. Objects show their class,
generic arguments, visible property values, and hidden private values. Arrays
show their size and items.

Debug output stops after 64 collection items and 32 nested levels. This keeps a
large or cyclic value from filling the terminal. The path is relative to the
working directory when possible.

Do not parse debug output. Its form may change between Whim releases.

## Writing tests

Use `assert!` for each condition:

```whim
assert!(2 + 2 == 4);
assert!(length!('whim') == 4);
assert!(Whim\Str\contains('whim', 'him'));
assert!(contains_key!(vec[1, 2], 1));
```

You may add a message after the condition:

```whim
$status = 200;
assert!($status == 200, 'the request must succeed');
```

Whim does not impose a test file layout or test runner. A test is a normal Whim
program that throws on failure. A shell, build tool, or Whim script can run a
set of such files.

## Stack traces

An uncaught throwable prints its type, message, source, and call stack. Normal
traces stop at `TraceBoundary` frames. Set `WHIM_FULL_TRACE=true` when debugging
the standard library itself:

```console
WHIM_FULL_TRACE=true whim tests/example.whim
```

Parameters marked `SensitiveParameter` remain hidden in either trace mode.

`panic!('message')` prints the same kind of stack trace, then exits with status
255. It takes a literal string and cannot be caught. Use it to mark a state that
must never occur.

## Bytecode

Use disassembly when a result differs between optimized and unoptimized code,
or when checking whether the optimizer found a known type:

```console
whim disassemble example.whim > optimized.txt
WHIM_OPTIMIZATIONS=off whim disassemble example.whim > plain.txt
```

Bytecode is an implementation detail. It may change in any release.
