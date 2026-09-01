# Language Constructs

A language construct looks like `name!(...)`, but it is not a function. The
compiler knows its rules and may emit direct bytecode for it.

## Array and string constructs

- `length!($value)` returns the byte length of a string or the item count of an
  array.
- `contains!($array, $value)` checks array values with strict equality.
- `contains_key!($array, $key)` checks an array key or index.
- `remove!($array, $key)` removes and returns one entry.
- `swap_remove!($vec, $index)` removes and returns one item without preserving
  vec order.
- `remove_first!($vec)` removes and returns the first item.
- `remove_last!($vec)` removes and returns the last item.

The remove forms change their target. `remove!` accepts vecs and dicts. The
other forms accept vecs. They throw when no matching item exists. See
[Removing entries](collections.md#removing-entries) for their order and cost.

## Assertions

`assert!` requires a bool. A false result throws `AssertionError`:

```whim
$value = 42;
assert!($value > 0, 'value must be positive');
```

The message is optional. The error includes the failed expression and its
source location.

## Output

- `write!(...)` writes values to standard output.
- `write_line!(...)` also writes a line ending.
- `write_error!(...)` writes to standard error.
- `write_error_line!(...)` also writes a line ending.

Each argument must be a string, int, or float.

## Debug output

`debug!(...)` writes a source location and a structural view to standard error.
It shows types, string byte lengths, object properties, and collection values.
It hides private property values and sensitive callables. It stops after 64
items or 32 nested levels.

```whim
debug!(dict['ready' => true, 'count' => 2]);
```

Use `debug!` while working, then remove it from normal output paths.

## Explicit discard

`discard!($value)` evaluates and ignores a value. It is the explicit way to
ignore a result marked `#[MustUse]`.

## Object cloning

`clone!` copies an object and may replace named properties during the copy:

```whim
final readonly class Point {
  public function __construct(public int $x, public int $y) {}

  public function withY(int $y): Point {
    return clone!($this, y: $y);
  }
}

$first = new Point(1, 2);
$second = $first->withY(9);
assert!(($first->y, $second->y) == (2, 9));
```

Whim checks property visibility, readonly rules, and types during the clone.

## Lifetime and process control

`drop!($variable)` releases one local now. It throws `LeakedResourceError` if
another strong reference keeps a resource alive.

`exit!()` ends the process with status zero. `exit!($status)` requires an int
and uses its low eight bits. Exit is not an exception: `catch` cannot catch it,
and `finally` does not run after it.

`panic!('message')` reports a broken invariant. It takes one literal string,
writes `panic: message` and the current stack trace to standard error, and ends
the process with status 255. It is not an exception. `catch` cannot catch it,
and `finally` does not run after it. Shutdown destructors still run.

```whim,ignore
if (!contains_key!($states, $name)) {
  panic!('the state table is incomplete');
}
```

Panic traces hide `TraceBoundary` frames unless full traces are on. They also
hide parameters marked `SensitiveParameter`. Use `throw` for errors that a
caller may handle. Use `panic!` only when continuing would be wrong.

## Source paths and loading

- `file!()` returns the current source file path.
- `directory!()` returns its directory.
- `embed!('./file')` reads a file while compiling.
- `require!($path)` loads and runs a source file.
- `require_once!($path)` does that at most once per resolved path.

See [Loading Files](loading.md) for load and error rules.

## Compile-time file embedding

`embed!` takes one literal relative path. Whim resolves it from the directory
of the source file that contains the construct, reads the file while compiling,
and stores its exact bytes as a string:

```whim,ignore
const TEMPLATE = embed!('./template.html');
```

Whim does not decode text or change line endings. The running program does not
read the file. A source read from standard input cannot use `embed!` because it
has no directory. Absolute paths, missing files, unreadable files, and
directories cause compile errors.

The compiler reads an embedded file even when the construct appears in code
that will not run. It reads each resolved path once per compilation. Do not use
`embed!` for secrets: the bytes remain plain in bytecode and compiled artifacts.
