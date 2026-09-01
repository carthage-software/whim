# Iterators

`foreach` can read an array, an `Iterator<K, V>`, or a
`ToIterator<K, V>`.

```whim
$letters = dict['a' => 1, 'b' => 2];
foreach ($letters as $letter => $position) {
  write_line!($letter . ': ' . $position);
}
```

The key is optional:

```whim
foreach (vec['Ada', 'Grace'] as $name) {
  write_line!($name);
}
```

## Iterator

An iterator owns one position. Its `next()` method returns the next key and
value, or `null` after the last item.

```text
interface Iterator<out K, out V> {
  public function next(): null|(K, V);
}
```

Passing an iterator to a second `foreach` does not rewind it. The second loop
starts at the iterator's current position.

## ToIterator

A value that can start a new pass implements `ToIterator<K, V>`:

```text
interface ToIterator<out K, out V> {
  public function toIterator(): Iterator<K, V>;
}
```

`foreach` calls `toIterator()` at the start of each pass. The method should
return a new iterator unless the type has a clear reason to share a position.

## Writing an iterable value

`ArrayIterator` turns any Whim array into an iterator.

```whim
use Whim\Iterate\ArrayIterator;
use Whim\Iterate\Iterator;
use Whim\Iterate\ToIterator;

final class Names implements ToIterator<int, string> {
  public function __construct(private vec<string> $names) {}

  public function toIterator(): Iterator<int, string> {
    return new ArrayIterator::<int, string>($this->names);
  }
}

$names = new Names(vec['Ada', 'Grace']);
foreach ($names as $index => $name) {
  write_line!($index . ': ' . $name);
}
```

## Iterable types

`Whim\Refine\Iterable<K, V>` is the union of:

- `Iterator<K, V>`
- `ToIterator<K, V>`
- `array<K, V>`

Library functions use this type when they only need to read a sequence. Such a
function accepts a tuple, vec, dict, iterator, or iterable object without first
copying it.

An iterator may yield data once. Do not assume that an `Iterable` can start a
second pass unless it is an array or implements `ToIterator`.

## Iterator helpers

`map`, `filter`, `take`, `drop`, and `join` return lazy iterators. They read an
entry only when their `next()` method needs one. `map` and `filter` keep the
source keys. `take` does not read past its limit.

Their concrete types are public: `MapIterator`, `FilterIterator`,
`TakeIterator`, `DropIterator`, and `JoinIterator`. Use the helper functions for
short pipelines, or construct these types when an API needs a concrete adapter.

`count`, `reduce`, `to_vec`, and `to_dict` consume the rest of an iterable.
`to_dict` keeps its keys; `to_vec` keeps only its values.

```whim
use Whim\Iterate;

$values = Iterate\take::<int, int>(
  Iterate\filter::<int, int>(
    Iterate\map::<int, int, int>(
      vec[1, 2, 3, 4],
      fn(int $value): int => $value * 2,
    ),
    fn(int $value): bool => $value > 4,
  ),
  1,
);

assert!(Iterate\to_vec::<int, int>($values) == vec[6]);
```

Functions in `Whim\Vec` and `Whim\Dict` return complete arrays instead.

Use a lazy iterator for a long stream or when the caller may stop early. Use a
vec or dict when the caller needs random access, a count, or more than one pass.
