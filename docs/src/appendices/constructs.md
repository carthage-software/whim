# Appendix C: Language Constructs

A construct uses function-like syntax, but the compiler applies its own rules.
This table lists every construct.

| Construct | Result | Main rule |
| --- | --- | --- |
| `assert!($test)` | `void` | throws when a bool test is false |
| `assert!($test, $message)` | `void` | adds a string message |
| `clone!($object, ...)` | object | clones an object and may replace fields |
| `contains!($array, $value)` | `bool` | checks array values with strict equality |
| `contains_key!($array, $key)` | `bool` | checks an array key or index |
| `debug!(...)` | `null` | prints source sites and bounded value details |
| `directory!()` | `string` | returns the current source directory |
| `discard!($value)` | `void` | marks a discarded result as deliberate |
| `drop!($local, ...)` | `void` | releases locals that hold the last strong references |
| `embed!('./file')` | `string` | embeds a file's exact bytes while compiling |
| `exit!()` | `never` | exits with status zero |
| `exit!($status)` | `never` | exits with the low eight bits of an int |
| `file!()` | `string` | returns the current source file |
| `length!($value)` | `int` | counts string bytes or array items |
| `panic!('message')` | `never` | prints a trace and exits with status 255 |
| `remove!($array, $key)` | value | removes and returns one entry |
| `swap_remove!($vec, $index)` | value | removes a vec item without keeping order |
| `remove_first!($vec)` | value | removes and returns the first item |
| `remove_last!($vec)` | value | removes and returns the last item |
| `require!($path)` | `null` | loads and runs a source file |
| `require_once!($path)` | `null` | loads a resolved path at most once |
| `write!(...)` | `void` | writes to standard output |
| `write_line!(...)` | `void` | writes to standard output, then ends the line |
| `write_error!(...)` | `void` | writes to standard error |
| `write_error_line!(...)` | `void` | writes to standard error, then ends the line |

## Mutable targets

The remove constructs require a writable array place, not an arbitrary
expression. `drop!` accepts locals and makes them undefined.

```whim
$values = vec[10, 20, 30];
$removed = remove!($values, 1);

assert!($removed == 20);
assert!($values == vec[10, 30]);
```

`remove!` preserves order when its target is a vec, so removing a non-final
item shifts each later item left. `remove_first!` does the same at index zero.
Use either form when later indexes or iteration order must stay predictable.
Removing near the front of a vec takes time in proportion to the items that
follow it, so repeatedly draining a vec with `remove_first!` takes quadratic
time. Use `Whim\DataStructure\Deque` for a FIFO queue.

`swap_remove!` does not shift. It moves the last item into the removed item's
index:

```whim
$values = vec[10, 20, 30, 40];
$removed = swap_remove!($values, 1);

assert!($removed == 20);
assert!($values == vec[10, 40, 30]);
```

Use it for an unordered vec when removal speed matters. Its removal step takes
constant time. As with any vec mutation, changing a shared vec may first copy
its storage. `remove_last!` also takes constant time and keeps the order of all
remaining items.

`remove!` also accepts a dict. It removes the requested key without changing
the other entries. `swap_remove!`, `remove_first!`, and `remove_last!` accept
only vecs. A missing key, invalid index, or empty vec throws
`OutOfBoundsError`.

## Output arguments

Write constructs accept any number of string, int, or float expressions. They
evaluate arguments from left to right. The line forms add the host line ending
after the last argument.

```whim
write_line!('count: ', 3);
```

They reject bool, null, arrays, objects, and callables. Convert such values
first.

## Debug arguments

`debug!` accepts any number of values. It writes one source location and value
view for each argument to standard error, then returns `null`.

```whim
$value = debug!(42);
assert!($value == null);
```

The view stops after 64 collection items and 32 nested levels.

## Process exit

`exit!` stops the process. It does not throw. A `catch` cannot intercept it,
and pending `finally` blocks do not run.

`panic!` has the same control flow. It takes one literal string, prints the
message and current stack trace to standard error, and uses status 255. Trace
boundaries and sensitive parameter markers apply. Both forms run shutdown
destructors.

## Source paths

`file!()` and `directory!()` take no arguments. Their values belong to the
source file that contains the construct, not the process entry file.

`embed!` takes one literal relative path. It resolves from that same source
directory and reads the file while compiling. It accepts any bytes and performs
no file access when the program runs. It rejects absolute paths and source read
from standard input.

See [Language Constructs](../language/constructs.md) for examples and
[Resources and Cleanup](../core-library/overview.md) for `drop!`.
