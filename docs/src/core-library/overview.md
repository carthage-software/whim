# Resources and Cleanup

Whim frees an object when code removes its last strong reference. A class can
use `__destruct(): void` for final cleanup. Code that needs a strict lifetime
can use `using` or `drop!`.

## Ordinary lifetime

An object stays alive while a local, property, collection, closure, call frame,
or trace holds it. Assigning over the last local reference may run its
destructor at that statement.

```whim,norun
final class Lease {
  public function __destruct(): void {
    write_line!('closed');
  }
}

$lease = new Lease();
$lease = null; // may print closed here
```

Arguments are strong references for the length of the call. Returning an object
passes a strong reference to the caller.

## `using`

`using` binds one or more values to a block and requires the block to hold the
last strong reference to each value when it ends.

```whim
use Whim\IO\MemoryHandle;

using ($handle = new MemoryHandle()) {
  $handle->writeAll('temporary data');
}
```

The check runs on normal fallthrough, `return`, `break`, `continue`, and throw.
If another strong reference still holds the value, Whim throws
`LeakedResourceError`.

```whim,norun
$escaped = null;

using ($handle = new Whim\IO\MemoryHandle()) {
  $escaped = $handle;
} // throws: $escaped still holds the handle
```

When a throw was already leaving the block, the leak error keeps that throwable
as its previous error.

The binding is local to the `using` block. If an outer local has the same name,
Whim restores it after the block. Reassigning the bound local does not change
which original value the block owns.

## Destructuring in `using`

A binding may use tuple, vec, or dict destructuring.

```whim,norun
using (
  ($reader, $writer) = open_pair(),
) {
  copy($reader, $writer);
}
```

Whim checks each bound value as the block ends. Every value must have no other
strong owner.

## `drop!`

`drop!($local)` requires the local to hold the last strong reference, releases
it, and makes the local undefined.

```whim,norun
$resource = open_resource();
drop!($resource);
```

If another strong reference exists, Whim throws `LeakedResourceError` and keeps
the local unchanged.

`drop!` accepts several locals. It checks all of them before dropping any, so a
failure leaves every local intact.

```whim,norun
drop!($first, $second, $third);
```

A self-cycle or another object cycle counts as another strong reference.

## `finally`

Use `finally` when cleanup is an action rather than ownership of one value.

```whim
function guarded(bool $fail): void {
  try {
    if ($fail) {
      throw new Whim\Unwind\RuntimeException('failed');
    }
  } finally {
    write_line!('finished');
  }
}

guarded(false);
```

`using` checks ownership. `finally` always runs an action. Choose the rule the
operation needs.

## Explicit close methods

I/O handles and other closeable objects provide `close()`. Call it when code
must handle a close error or release an outside resource before the object's
last reference dies.

The destructor remains a fallback. A safe close method should allow a second
call, and its destructor should not mask an earlier failure.

## Cycles

Strong references can form an unreachable cycle. Whim's cycle collector finds
such cycles and runs their destructors.

`Whim\GC\collect_cycles()` requests a collection and returns the number of
boxes it freed. The runtime also starts collection after its cycle threshold.
Set that threshold under `[runtime]` in `whim.toml`, or set
`WHIM_CYCLE_THRESHOLD` for one run.

Do not depend on the exact order in which unrelated cycle destructors run.
