# Enums

An enum defines a fixed set of values. Each case is one shared value.

```whim
enum Direction {
  case North;
  case South;
}

$direction = Direction::North;
write_line!($direction->name); // North
```

Code cannot create an enum with `new` and cannot clone an enum case.

## Unit enums

An enum without a backing type is a unit enum. Its cases have names but no
backing values.

```whim
enum State {
  case Waiting;
  case Running;
  case Done;
}
```

Every enum case has a public readonly `name` property. Every enum has a static
`cases()` method that returns its cases in source order.

```whim
enum State {
  case Waiting;
  case Running;
  case Done;
}

$states = State::cases();
write_line!($states[0]->name); // Waiting
```

All enums implement `Whim\Enum\UnitEnum`.

## Backed enums

A backed enum uses `int` or `string`. A type alias that resolves to one of those
types also works.

```whim
enum Status: string {
  case Ready = 'ready';
  case Waiting = 'waiting';
}

enum Code: int {
  case Ok = 200;
  case Missing = 404;
}
```

Every case must have a value, and no two cases may have the same value. A backed
case has a public readonly `value` property.

Backed enums also implement `Whim\Enum\BackedEnum<int>` or
`Whim\Enum\BackedEnum<string>`.

## Looking up backed cases

`from` returns the case with a given value. It throws
`Whim\Unwind\ValueError` when no case has that value.

`tryFrom` returns the case or `null`.

```whim
enum Status: string {
  case Ready = 'ready';
  case Waiting = 'waiting';
}

$ready = Status::from('ready');
$missing = Status::tryFrom('missing');

write_line!($ready->name);
assert!($missing == null);
```

## Methods and interfaces

An enum may implement interfaces and define concrete methods.

```whim
interface Labelled {
  public function label(): string;
}

enum Level: int implements Labelled {
  case Low = 1;
  case High = 10;

  public function label(): string {
    return match ($this) {
      self::Low => 'low',
      self::High => 'high',
    };
  }
}
```

An enum cannot:

- extend a class or enum;
- declare type parameters;
- declare properties or a constructor;
- declare abstract methods;
- replace its built-in enum methods.

Its methods, constants, and cases share one member name set.

## Matching enum cases

An enum case is also a type that matches only that case. It can appear directly
in a `match` arm:

```whim
enum Status: string {
  case Ready = 'ready';
  case Waiting = 'waiting';
}

function label(Status $status): string {
  return match ($status) {
    Status::Ready => 'ready now',
    Status::Waiting => 'not yet',
  };
}
```

Whim does not require a match over an enum to list every case at compile time.
Code can load types after the first file compiles. If no arm matches at runtime,
Whim throws `Whim\Unwind\UnhandledMatchError`.
