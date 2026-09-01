# Tuples, Vecs, and Dicts

Whim arrays are values. Tuples are fixed and immutable. Vecs and dicts are
mutable.

## Tuples

A tuple may hold a different type at each position:

```whim
$user = (7, 'Ada', true);
assert!($user[0] == 7);
assert!($user is (int, string, bool));
```

A one-item tuple needs a trailing comma:

```whim
$one = ('only',);
assert!($one[0] == 'only');
```

`()` is not a value. Tuples may hold 1 through 12 items. Their indexes do not
change, and tuple elements reject writes.

A tuple type may end with a rest type:

```whim
function read_row((int, ...string) $row): int {
  return $row[0];
}

assert!(read_row((7, 'a', 'b')) == 7);
```

## Vecs

A vec has dense integer keys from zero:

```whim
$names = vec['Ada', 'Grace'];
$names[1] = 'Hopper';
$names[] = 'Linus';

assert!($names == vec['Ada', 'Hopper', 'Linus']);
```

Indexed assignment must replace an existing position. Use `$vec[] = $value` to
append.

The fill form evaluates its value once, then repeats it:

```whim
$zeros = vec[0; 4];
assert!($zeros == vec[0, 0, 0, 0]);
```

Spread a vec or tuple into a vec literal:

```whim
$middle = vec[2, 3];
assert!(vec[1, ...$middle, 4] == vec[1, 2, 3, 4]);
```

A dict cannot spread into a vec.

## Dicts

A dict accepts bool, int, and string keys. It keeps insertion order:

```whim
$scores = dict['Ada' => 10, 'Grace' => 12];
$scores['Ada'] = 11;
$scores['Linus'] = 9;

assert!($scores['Ada'] == 11);
assert!(length!($scores) == 3);
```

Keys do not convert. `1`, `'1'`, and `true` are three different keys.

Replacing a value keeps the key's place. Removing a key and adding it again
moves it to the end.

A duplicate literal or spread key keeps its first place and its last value:

```whim
$key = 'a';
$values = dict[$key => 1, 'b' => 2, $key => 3];
assert!($values == dict['a' => 3, 'b' => 2]);
```

A dict spread accepts a tuple, vec, or dict. A tuple or vec contributes integer
keys. A dict keeps its keys.

## Reading entries

An index must exist. Reading a missing vec position, tuple position, or dict key
throws `OutOfBoundsError`. `??` does not hide that error:

```whim
$values = dict['ready' => true];
assert!(!contains_key!($values, 'missing'));
```

Check with `contains_key!` before the read. `contains!` checks values:

```whim
$values = vec[10, 20];
assert!(contains_key!($values, 1));
assert!(contains!($values, 20));
```

`length!` works on every array and on strings.

## Removing entries

`remove!($array, $key)` removes and returns one entry. On a vec, it preserves
order by shifting every later item left. `remove_first!($vec)` is the same
ordered operation at index zero. Use these forms when remaining indexes and
iteration order must not change.

Both operations take time in proportion to the items after the removed one.
Repeated `remove_first!` calls therefore take quadratic time. Use
`Whim\DataStructure\Deque` for repeated FIFO removal.

`swap_remove!($vec, $index)` instead moves the last item into `$index`. It takes
constant time on an unshared vec, but changes order. Use it for unordered work
sets and pools. `remove_last!($vec)` also takes constant time and does not
reorder the remaining items.

```whim
$values = vec['a', 'b', 'c', 'd'];
$removed = swap_remove!($values, 1);

assert!($removed == 'b');
assert!($values == vec['a', 'd', 'c']);
```

All vec mutations may first copy storage when another vec shares it.

## Iteration

`foreach` walks keys and values in array order:

```whim
$seen = vec[];
foreach (dict['a' => 1, 'b' => 2] as $key => $value) {
  $seen[] = $key . $value;
}

assert!($seen == vec['a1', 'b2']);
```

The loop binds copies. Assigning `$key` or `$value` does not change the array.
The loop also keeps the set of entries with which it began, so changing the
source during the loop does not change the current walk.
