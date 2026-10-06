# State Machines

`Whim\State` provides typed finite state machines. `Machine<S, E>` holds one
current state of type `S` and accepts events of type `E`. These may be enums,
strings, or other Whim types; the machine compares states and events with
Whim equality.

## Declaring transitions

Use `MachineBuilder<S, E>` to set the initial state and declare allowed moves:

```whim
use Whim\State\MachineBuilder;

enum OrderState {
  case Draft;
  case Placed;
  case Shipped;
}

enum OrderEvent {
  case Place;
  case Ship;
}

$builder = new MachineBuilder::<OrderState, OrderEvent>(OrderState::Draft);
$builder->allow(OrderState::Draft, OrderEvent::Place, OrderState::Placed);
$builder->allow(OrderState::Placed, OrderEvent::Ship, OrderState::Shipped);

$order = $builder->build();
assert!($order->getState() == OrderState::Draft);
assert!($order->can(OrderEvent::Place));
assert!($order->peek(OrderEvent::Place) == OrderState::Placed);
assert!($order->getState() == OrderState::Draft);
assert!($order->apply(OrderEvent::Place) == OrderState::Placed);
assert!($order->apply(OrderEvent::Ship) == OrderState::Shipped);
assert!(!$order->can(OrderEvent::Place));

$another = $builder->build();
assert!($another->getState() == OrderState::Draft);
```

| Method | Effect |
| --- | --- |
| `getState()` | Returns the current state. |
| `peek(event)` | Returns the first allowed target, or `null` when none matches. |
| `can(event)` | Reports whether a transition accepts the event. |
| `apply(event)` | Changes state, calls observers, and returns the target. Throws `TransitionException` when no transition accepts the event. |

Each `build()` creates a new machine with the builder's current transition
and observer lists. Later additions to the builder do not change machines
already built. Guard and observer objects can still be shared between machines.

States may include `null`. In that case, `peek()` returning `null` can mean
either a valid null target or no transition. Use `can()` to tell them apart.

## Guards and observers

`Guard<S, E>::allows(from, event, to)` decides whether a matching transition
may run. `FunctionGuard` wraps a `fn(S, E, S): bool`.

The machine checks transitions in declaration order. When a guard returns
`false`, it tries the next transition for the same state and event. The first
accepted transition wins. A transition without a guard always accepts the
matching state and event.

`Observer<S, E>::onTransition(from, event, to)` receives a completed move.
`FunctionObserver` wraps a `fn(S, E, S): void`. Observers run in registration
order after the machine sets its new state:

```whim
use Whim\Reference\Strong;
use Whim\State\FunctionGuard;
use Whim\State\FunctionObserver;
use Whim\State\MachineBuilder;

$ready = new Strong::<bool>(false);
$moves = new Strong::<vec<(string, string, string)>>(vec[]);
$builder = new MachineBuilder::<string, string>('queued');
$builder->allow('queued', 'start', 'running',
  new FunctionGuard::<string, string>(
    fn(string $_from, string $_event, string $_to): bool => $ready->value,
  ),
);
$builder->observe(new FunctionObserver::<string, string>(
  fn(string $from, string $event, string $to): void {
    $moves->value[] = ($from, $event, $to);
  },
));

$machine = $builder->build();
assert!(!$machine->can('start'));
$ready->value = true;
assert!($machine->apply('start') == 'running');
assert!($moves->value == vec[('queued', 'start', 'running')]);
```

`peek()` and `can()` run guards but do not change state or call observers.
A guard error passes to the caller before any state change. An observer error
passes to the caller after the state change and stops the remaining observers;
it does not undo the move.

Callbacks may suspend. Coordinate callers that share a machine when callbacks
can yield; the machine does not lock transitions.

You can also construct `Machine<S, E>` directly with an initial state, a
`vec<Transition<S, E>>`, and a `vec<Observer<S, E>>`. Each `Transition` holds
public `from`, `event`, `to`, and `guard` fields.
