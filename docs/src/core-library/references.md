# References and Cycles

Object variables are strong references. They keep objects alive and preserve
identity across assignments, calls, collections, properties, and closure
captures.

The `Whim\Reference` namespace also provides weak references and weak maps.

## Weak references

`Weak<T>` points to an object without keeping it alive.

```whim
use Whim\Reference\Weak;

final class User {
  public function __construct(public string $name) {}
}

function watch_user(): Weak<User> {
  $user = new User('Ada');
  $weak = new Weak::<User>($user);
  assert!($weak->get() is User);
  return $weak;
}

$weak = watch_user();
assert!($weak->get() == null);
```

`get()` returns `null|T`. A non-null result is a new strong reference for as
long as the caller keeps it.

A weak reference does not stop `using` or `drop!` from releasing a value.

## Weak maps

`WeakMap<K, V>` stores values under object keys without keeping those keys
alive.

```whim
use Whim\Reference\WeakMap;

final class Request {}

$map = new WeakMap::<Request, string>();
$request = new Request();
$map->set($request, 'state');

assert!($map->has($request));
assert!($map->get($request) == 'state');
```

When code removes the last strong reference to a key, its entry leaves the map.

The main methods are:

- `set($key, $value): void`
- `has($key): bool`
- `get($key): V`
- `remove($key): V`
- `length(): int`

`get` and `remove` throw `OutOfBoundsError` when the key has no entry.

Use a weak map for caches or per-object data that must not own the key. Use a
dict when keys are scalar or when the map should own all of its data.

## Strong cycles

Two or more objects may keep each other alive after code removes all outside
references.
Whim records possible cycles and collects them later.

```whim,norun
final class Node {
  public null|Node $next = null;
}

$left = new Node();
$right = new Node();
$left->next = $right;
$right->next = $left;
$left = null;
$right = null;

Whim\GC\collect_cycles();
```

Weak references can break ownership cycles when one direction only needs to
observe the other object.
