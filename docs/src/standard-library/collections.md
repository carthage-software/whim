# Collections and Data Structures

## Vec functions

`Whim\Vec` reads any `Iterable<_, T>` when keys do not matter and returns a
packed vec.

Creation and access include `values`, `keys`, `fill`, `reproduce`, and integer
`range`. Transform tools include `map`, `map_with_key`, `map_nonnull`,
`flat_map`, `enumerate`, `reductions`, and `reduce`.

Filtering includes `filter`, key-aware filters, null removal, partitioning, and
callback forms that keep non-null results.

Order and slice tools include `reverse`, `unique`, `unique_by`, `sort`,
`sort_by`, `shuffle`, `take`, `drop`, `slice`, and `chunk`. `equals` compares
vecs whose values implement `Comparison\Equal<T>`. `concat`,
`flatten`, and `zip` combine inputs.

```whim
use Whim\Vec;

$values = Vec\map::<int, int>(vec[1, 2, 3], fn(int $n): int => $n * 2);
$even = Vec\filter::<int>($values, fn(int $n): bool => $n % 4 == 0);
assert!($even == vec[4]);
```

These functions return new vecs. Indexed assignment, append assignment,
`remove!`, and `swap_remove!` change an existing vec variable. `remove!`
preserves order; `swap_remove!` moves the last item into the removed index.

## Dict functions

`Whim\Dict` keeps keys. It creates dicts from iterables, entries, keys, value
selectors, groups, and counts.

Transform tools map values or keys, flatten nested keyed values, reindex, and
flip. Filters preserve keys. Slice tools take, drop, pull, select keys, and
apply while predicates.

`merge`, `diff`, and `intersect` have value and key forms. `equal` accepts a
custom equality callback. Dict sorting can sort by value, selected value, or
key while preserving keys.

## Lazy iteration

`Whim\Iterate` holds `Iterator`, `ToIterator`, and `ArrayIterator`. Use it when
the caller may stop early or the source is a stream. See
[Iterators](../core-library/iteration.md).

## Queue, stack, deque, and heap

`Whim\DataStructure` provides mutable focused containers:

- `Queue<T>` adds at the back and removes from the front.
- `Stack<T>` adds and removes at the top.
- `Deque<T>` adds, removes, and peeks at both ends.
- `BinaryHeap<T>` orders values with a comparator.
- `PriorityQueue<T, P>` stores a value with a separate priority and comparator.

Empty removals and peeks return `Option<T>` so a stored `null` remains distinct
from no item. Each type reports `count`, `isEmpty`, supports `clear`, converts
to a vec, and implements `ToIterator`.

The heap and priority queue do not share an implementation. Equal values or
priorities have no set relative order.
