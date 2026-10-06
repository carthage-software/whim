# Variation

`Whim\Variation` holds values that depend on named choices. A `fresh` identity
names a dimension. Every choice with that identity follows the same selection:
`false` takes its first branch; `true` takes its second.

```whim
use Whim\Variation;

$dimension = fresh!();
$x = Variation\choose::<int>($dimension, 10, 20);
$y = Variation\choose::<int>($dimension, 1, 2);
$sum = Variation\combine::<int, int, int>(
  $x,
  $y,
  fn(int $a, int $b): int => $a + $b,
);

assert!($sum->select(dict[$dimension => false]) == 11);
assert!($sum->select(dict[$dimension => true]) == 22);
```

The sum has two outcomes, `11` and `22`. It cannot mix the first branch of `$x`
with the second branch of `$y`. To make two choices independent, give them
different `fresh` identities.

## Building choices

The sealed interface `Variation<T>` has two forms:

- `Value<T>` holds a plain value in its readonly `value` property.
- `Choice<T>` holds a readonly `dimension`, `whenFalse`, and `whenTrue`.
  Both branches are `Variation<T>` values, so choices can nest and share branches.

`value::<T>($value)` creates a `Value<T>`.
`choose::<T>($dimension, $whenFalse, $whenTrue)` creates a `Choice<T>` from
two plain values. Use the `Choice<T>` constructor to join existing trees:

```whim
use Whim\Variation;
use Whim\Variation\Choice;

$size = fresh!();
$colour = fresh!();
$small = Variation\choose::<string>($colour, 'small red', 'small blue');
$large = Variation\choose::<string>($colour, 'large red', 'large blue');
$item = new Choice::<string>($size, $small, $large);

assert!($item->select(dict[$size => true, $colour => false]) == 'large red');
```

A repeated dimension stays linked even if it appears at different depths or
in a different order in another tree. Choices do not impose an order on
`fresh` values.

## Selecting and specializing

`select(dict<fresh, bool> $selection): T` follows one path and returns its value.
It throws `Unwind\InvalidArgumentException` if a dimension on that path has no
selection. Extra selections do no harm. A dimension that occurs only on a
branch the method does not visit needs no selection.

`specialize($selection): Variation<T>` resolves the supplied dimensions and
keeps the remaining choices. It returns a new tree when needed and shares
unchanged branches with the source. Supplying every dimension produces a
`Value<T>`:

```whim
use Whim\Variation;

$dimension = fresh!();
$number = Variation\choose::<int>($dimension, 10, 20);
$chosen = $number->specialize(dict[$dimension => true]);

assert!($chosen is Variation\Value<int>);
assert!($chosen->select(dict[]) == 20);
assert!($number->select(dict[$dimension => false]) == 10);
```

## Mapping and combining

`map::<U>(fn(T): U $function): Variation<U>` transforms each reachable leaf.
`combine::<A, B, C>($left, $right, fn(A, B): C $function): Variation<C>`
transforms each reachable pair. Both call the function during the operation.
Later calls to `select()` do not run it again. A thrown error stops the operation
and reaches the caller.

Callbacks should be free of side effects: the operation may call them for
several alternatives, including repeated values. Independent dimensions can
make the number of pairs grow quickly. These operations keep a tree of choices;
they do not build a list of every selection.

The tree is readonly. A value stored inside it can still be a mutable object;
the library does not clone or freeze that object. Arithmetic, branching, and
other operations use explicit callbacks rather than new language operators.

See [Logic Variables](logic.md) for shared values that become bound later,
and [Built-in Values](../language/types.md) for `fresh` identities.
