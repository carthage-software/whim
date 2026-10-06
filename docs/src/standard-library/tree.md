# Trees

`Whim\Tree` provides typed trees, traversal, search, mapping, filtering, and
folds. Each `Node<T>` holds a value and an ordered vector of children. The
sealed interface has two readonly classes: `TreeNode<T>` and `LeafNode<T>`.

## Building a tree

Use `tree()` for a value with children and `leaf()` for a value with no
children. Both classes expose `getValue()` and `getChildren()`:

```whim
use Whim\Tree;

$tree = Tree\tree::<string>('root', vec[
  Tree\leaf::<string>('left'),
  Tree\tree::<string>('right', vec[Tree\leaf::<string>('child')]),
]);

assert!(Tree\count::<string>($tree) == 4u);
assert!(Tree\depth::<string>($tree) == 2u);
assert!(Tree\pre_order::<string>($tree) == vec['root', 'left', 'right', 'child']);
assert!(Tree\post_order::<string>($tree) == vec['left', 'child', 'right', 'root']);
assert!(Tree\leaves::<string>($tree) == vec['left', 'child']);
assert!(Tree\path_to::<string>($tree,
  fn(string $value): bool => $value == 'child',
) == vec['root', 'right', 'child']);
assert!(Tree\at_index::<string>($tree, vec[1u, 0u])->unwrap() == 'child');
```

You can also construct `TreeNode<T>(value, children)` or `LeafNode<T>(value)`
directly. A `TreeNode` with no children counts as a leaf. Tree shape stays
fixed; a stored value may still hold a mutable object.

## Traversal and search

| Function | Result |
| --- | --- |
| `is_leaf(node)` | Whether the node has no children. |
| `count(node)` | The number of nodes, including the root. |
| `depth(node)` | The greatest depth; a root with no children has depth `0u`. |
| `leaves(node)` | Leaf values, in child order. |
| `pre_order(node)` | Each value before its children's values. |
| `post_order(node)` | Each value after its children's values. |
| `level_order(node)` | Values in breadth-first order. |
| `all(node, predicate)` | Whether every value matches. |
| `any(node, predicate)` | Whether any value matches. |
| `contains(node, value)` | Whether a value compares equal under Whim equality. |
| `find(node, predicate)` | The first breadth-first match as `Some<T>`, or `None`. |
| `path_to(node, predicate)` | The value path to the first breadth-first match, or `null`. |
| `at_index(node, indexPath)` | The value at a path of unsigned child indices as `Some<T>`, or `None`. |
| `to_index(node, predicate)` | The child-index path to the first breadth-first match, or `null`. |

`all()` and `any()` call predicates in pre-order and stop when the answer is
known. Value paths include the root. An empty index path refers to the root;
`to_index()` returns an empty vector when the root matches. `at_index()`
accepts an iterable of `uint` indices, including a vector, tuple, or iterator.

## Transforming and folding

| Function | Result |
| --- | --- |
| `reduce(node, callback, initial)` | An accumulator built in pre-order; the callback receives `(accumulator, value)`. |
| `map(node, callback)` | A tree with the same shape and transformed values. |
| `filter(node, predicate)` | A tree of matching nodes whose ancestors also match, or `null` if the root fails. |
| `fold(node, callback)` | A result built from child results; the callback receives `(value, vec<childResult>)`. |
| `traverse(node, callback)` | A result with lazy child results; the callback receives `(value, fn(): vec<childResult>)`. |
| `to_dict(node)` | A nested dictionary with `value` and `children` entries. |

`map()` and `fold()` process children before their parent. `filter()` tests a
node before its children; rejecting a node drops its whole subtree. It returns
`TreeNode<T>` objects for all retained nodes, including leaves.

`traverse()` lets a callback skip children. Calling its child supplier computes
and caches that node's child results. Later calls return the cached results:

```whim
use Whim\Math;
use Whim\Tree;

$tree = Tree\tree::<int>(1, vec[Tree\leaf::<int>(2), Tree\leaf::<int>(3)]);
$total = Tree\traverse::<int, int>(
  $tree,
  fn(int $value, fn(): vec<int> $children): int => $value + Math\sum($children()),
);
assert!($total == 6);

$root = Tree\traverse::<int, int>(
  $tree,
  fn(int $value, fn(): vec<int> $_children): int => $value,
);
assert!($root == 1);
```

## Building from flat data

`from_iterable<TItem, TId, TValue>()` accepts an iterable of items and three
callbacks: an item ID, its parent ID as `Option<TId>`, and its stored value.
`TId` must satisfy `Whim\Refine\ArrayKey`. `None` marks the root:

```whim
use Whim\Option\None;
use Whim\Option\Option;
use Whim\Option\Some;
use Whim\Tree;

type Row = (string, null|string);

$tree = Tree\from_iterable::<Row, string, string>(
  vec[('child', 'root'), ('root', null)],
  fn(Row $row): string => $row[0],
  fn(Row $row): Option<string> {
    if ($row[1] == null) {
      return new None();
    }
    return new Some::<string>($row[1]);
  },
  fn(Row $row): string => $row[0],
);
assert!(Tree\pre_order::<string>($tree) == vec['root', 'child']);
```

Items may appear before their parents. Children follow input order. The builder
reads the iterable once, then checks the IDs and parent paths before it builds
the tree. Use stable callbacks: the builder may call them more than once.

All input errors extend `Whim\Unwind\InvalidArgumentException`:

| Error | Cause |
| --- | --- |
| `NoRootNodeException` | No item has `None` as its parent, including empty input. |
| `MultipleRootNodesException` | More than one item has `None` as its parent. |
| `OrphanedNodeException` | An ID repeats, a parent is missing, or a parent chain forms a cycle. |
