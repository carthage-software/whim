# Logic Variables

`Whim\Logic\Variable<T>` is a shared variable that starts unbound. Binding it
gives every reader a value. Linking two variables means that binding either
also binds the other:

```whim
use Whim\Logic;

$x = new Logic\Variable::<int>();
$y = new Logic\Variable::<int>();
Logic\unify::<int>($x, $y);
$y->bind(42);

assert!($x->get() == 42);
assert!($y->get() == 42);
```

Assigning a variable object to another local keeps the same shared state.
`Variable<T>` is mutable; it is not a copy value.

## Binding and linking

`bind(T $value): void` sets the binding. Binding it again succeeds only if the
new value compares equal with `==`; otherwise it throws `ConflictException`.
That exception extends `Unwind\LogicException`.

Equality follows Whim's usual rules. Objects compare by identity. NaN does
not equal itself, so binding an already bound NaN again throws. A binding does
not freeze a mutable object stored as its value.

`unify::<T>($left, $right): void` links two variables. The instance method
`$left->unify($right)` does the same work. Both variables must have the same
value type `T`.

- Two unbound variables share their next binding.
- Linking an unbound variable to a bound one gives it that binding at once.
- Two bound variables can link when their values compare equal. They then share
  one of those values; the library does not promise which copy it keeps.
- Unequal bindings throw `ConflictException` and keep their separate bindings.
- Linking a variable to itself, or linking already linked variables, has no
  further effect.

`bind()` and `unify()` do not suspend. A later binding or link wakes readers
already waiting on either variable. Chains and repeated links cannot form a
cycle.

`isBound(): bool` reports whether a variable has a binding. A bound `null` is
distinct from an unbound variable:

```whim
use Whim\Logic\Variable;

$value = new Variable::<null>();
assert!(!$value->isBound());
$value->bind(null);
assert!($value->isBound());
assert!($value->get() == null);
```

## Waiting and cancellation

`get(null|Async\CancellationToken $cancellation = null): T` returns a bound
value at once, or suspends the current task until a binding arrives:

```whim
use Whim\Async;
use Whim\Logic\Variable;

$value = new Variable::<string>();
$reader = Async\spawn::<string>(fn(): string => $value->get());
Async\later();
$value->bind('ready');

assert!($reader->await() == 'ready');
```

Cancellation stops only that read. It leaves the variable, its binding, and
other readers intact. A token that has already cancelled makes `get()` throw
even if the variable is bound. An unbound read with no future binding waits
until cancellation or task shutdown.

This library links whole variables and compares bound values. It does not
unify fields inside data structures, undo bindings, or search through possible
answers. Reads remain explicit calls to `get()`.

See [Channels and Cancellation](channels.md) for cancellation tokens and
[Variation](variation.md) for values with explicit alternatives.
