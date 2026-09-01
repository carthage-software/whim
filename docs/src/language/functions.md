# Functions

A function declaration has a name, optional type parameters, parameters, an
optional return type, and a body.

```whim
function area(float $width, float $height): float {
  return $width * $height;
}

assert!(area(4.0, 2.5) == 10.0);
```

## Parameter and return types

Whim checks each typed argument before the function starts. It checks a typed
result before the caller receives it.

```whim
function identity<T>(T $value): T {
  return $value;
}

assert!(identity::<string>('value') == 'value');
```

An omitted parameter or return type means `mixed`. Write the type when the
function has a useful contract.

A `void` function returns no value:

```whim
function announce(string $message): void {
  write_line!($message);
}

announce('ready');
```

A `never` function cannot return:

```whim,norun
function fail(string $message): never {
  throw new Whim\Unwind\RuntimeException($message);
}
```

## Optional parameters

A default makes a parameter optional:

```whim
function greet(string $name, string $greeting = 'Hello'): string {
  return $greeting . ', ' . $name;
}

assert!(greet('Ada') == 'Hello, Ada');
assert!(greet('Ada', 'Welcome') == 'Welcome, Ada');
```

Required parameters must come before optional parameters. Whim evaluates an
omitted default when the call enters the function. It then checks the default
against the parameter type.

Defaults use constant expressions. They may use literals, arrays, constants,
named object construction, and calls whose receiver and arguments are also
constant expressions.

## Named arguments

An argument may name its parameter:

```whim
function box(string $label, int $width = 3, int $height = 1): string {
  return $label . ':' . $width . ':' . $height;
}

assert!(box('panel', height: 9) == 'panel:3:9');
assert!(
  box(
    height: 2,
    label: 'card',
    width: 4,
  )
  == 'card:4:2',
);
```

Named arguments may skip optional parameters and may appear out of declaration
order. An unknown or repeated name throws `ArgumentCountError`.

## Argument count

Whim has no implicit variadic parameter. A function receives exactly its
declared parameters. Use a vec when a call should pass a list:

```whim
function sum(vec<int> $values): int {
  $total = 0;
  foreach ($values as $value) {
    $total += $value;
  }

  return $total;
}

assert!(sum(vec[1, 2, 3]) == 6);
```

Too few or too many arguments throw `ArgumentCountError`.

## Local scope

Parameters and assignments belong to the function call. A function cannot read
variables from the file that declared it. Use parameters, constants, static
properties, or a closure capture to pass data in.

Calls may recurse. `[runtime].call-depth` sets the frame limit. The
`WHIM_CALL_DEPTH` environment variable overrides it for one run. Exceeding the
limit throws `StackOverflowError`.

## Generic functions

A function may declare reified type parameters:

```whim
function singleton<T>(T $value): vec<T> {
  return vec[$value];
}

assert!(singleton::<int>(7) is vec<int>);
```

Whim does not infer `T` from an argument. Supply `::<...>` or give `T` a
default. The [Generics](generics.md) chapter covers bounds, defaults, variance,
and forwarding.
