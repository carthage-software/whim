# Assignment and Indexing

Assignment is an expression. It stores a value and returns that value:

```whim
$target = 0;
$result = ($target = 5);

assert!($target == 5);
assert!($result == 5);
```

## Targets

An assignment target may be:

- a variable;
- an object property;
- a static property;
- a vec or dict index;
- a vec append target, `$values[]`;
- a tuple or dict destructuring pattern.

Literals, arithmetic results, strings, and tuple entries are not writable
targets. A property on an object returned by a call is writable because the
object keeps its identity.

## Compound assignment

Whim supports these compound forms:

```text
+=  -=  *=  /=  %=  **=  .=
&=  |=  ^=  <<=  >>=  &&=  ||=  ??=
```

The target expression runs once:

```whim
final class Counter {
  public int $calls = 0;

  public function index(): int {
    $this->calls++;
    return 0;
  }
}

$counter = new Counter();
$values = vec[10];
$values[$counter->index()] += 1;

assert!($counter->calls == 1);
assert!($values[0] == 11);
```

`&&=`, `||=`, and `??=` short-circuit like their non-assignment forms.

## Tuple and vec destructuring

A tuple target accepts a tuple or vec:

```whim
($name, $age) = ('Ada', 36);
assert!($name == 'Ada');
assert!($age == 36);
```

Without a rest target, the source size must match. A trailing rest target
collects the remaining values in a vec:

```whim
($head, ...$tail) = vec[1, 2, 3];
assert!($head == 1);
assert!($tail == vec[2, 3]);
```

Use bare `...` to allow and ignore the rest.

## Defaulted targets

A default runs only when its position is missing. A present null does not use
the default:

```whim
($name, $role = 'member') = vec['Ada'];
assert!($role == 'member');

($id, $label = 'fallback') = vec[1, null];
assert!($label == null);
```

A default may use an earlier binding. Once one target has a default, every
later fixed target must also have one. A rest target may follow them.

## Dict destructuring

A dict target selects named keys:

```whim
$source = dict['id' => 7, 'profile' => dict['name' => 'Ada']];
dict['id' => $id, 'profile' => dict['name' => $name]] = $source;

assert!(($id, $name) == (7, 'Ada'));
```

Each key expression runs before the source expression. A missing key throws.
The source must be a dict.

## Write order

Whim reads the full source before it writes any target. A target may reuse the
source variable without changing later reads:

```whim
$values = (1, 2, 3);
($values, $second, $third) = $values;
assert!(($values, $second, $third) == (1, 2, 3));
```

Whim writes targets from left to right. If one variable appears twice, the last
write wins. `$_` follows the same rule because it is an ordinary variable.
