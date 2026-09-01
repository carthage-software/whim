# Symbols as Types

Every Whim symbol has a type meaning. A name in a type position does not always
mean a class.

This rule lets types name specific constants, enum cases, functions, and
methods as well as objects.

## Classes and interfaces

A class name accepts instances of that class and its children.

```whim
class Message {}
final class Notice extends Message {}

function send(Message $message): void {}

send(new Message());
send(new Notice());
```

An interface name accepts objects that implement it. Generic arguments remain
part of either type.

## Enums and cases

An enum name accepts every case of that enum. An enum case name accepts only
that case.

```whim
enum Colour {
  case Red;
  case Blue;
}

function paint(Colour $colour): void {}
function paint_red(Colour::Red $colour): void {}
```

Case types work in unions, bounds, casts, and match arms.

## Constants

A constant name accepts values equal to that constant.

```whim
const OK = 200;
const NOT_FOUND = 404;

function success(OK $status): void {}

success(200);
```

The same rule applies to class-like constants:

```whim
final class Codes {
  public const int SUCCESS = 200;
}

function success(Codes::SUCCESS $status): void {}
```

Two constants with the same value denote the same set of values, even when they
have different names.

This also gives a bare constant its expected meaning in a match:

```whim
const OK = 200;

function label(int $status): string {
  return match ($status) {
    OK => 'ok',
    $_ => 'other',
  };
}
```

The arm compares the subject with the constant's value. It does not treat `OK`
as an undeclared class.

## Functions

A function name as a type accepts first-class callables made from that function.

```whim
function add(int $left, int $right): int {
  return $left + $right;
}

function run_add(add $callable): int {
  return $callable(20, 22);
}

$callable = add(...);
assert!(run_add($callable) == 42);
```

Another callable with the same signature does not fit `add`. A partial call of
`add` does not fit it either. Use `fn(...)` when origin does not matter.

For a generic function, the bare name describes the whole function family. A
type argument selects one binding:

```whim
function identity<T>(T $value): T {
  return $value;
}

$integer = identity::<int>(...);
assert!($integer is identity);
assert!($integer is identity<int>);
assert!(!($integer is identity<string>));
```

A non-generic function takes no type arguments in a type.

## Methods

`Type::method` accepts first-class callables made from that method family.

```whim
interface Mapper<T> {
  public function map<U>(T $value, U $fallback): U;
}

final class IntegerMapper implements Mapper<int> {
  public function map<U>(int $value, U $fallback): U {
    return $fallback;
  }
}

function call_map(Mapper<int>::map<string> $callable): string {
  return $callable(42, 'value');
}

$map = new IntegerMapper()->map::<string>(...);
assert!(call_map($map) == 'value');
```

The receiver must belong to the named class or interface family with compatible
type arguments. A method on an unrelated class does not fit merely because its
signature matches. A partial method call does not fit.

Inherited static method callables may fit both the parent and child method
families.

## Aliases and newtypes

A type alias expands to its target type. A newtype keeps a distinct tag while
also fitting its backing type.

```whim
type Identifier = int;
newtype UserId = int;

assert!(1 is Identifier);
assert!(UserId(1) is UserId);
assert!(UserId(1) is int);
assert!(!(1 is UserId));
```

See [Aliases and Newtypes](../core-library/functions-and-constants.md) for the
full rules.

## One symbol name set

At namespace scope, classes, interfaces, enums, constants, functions, aliases,
and newtypes share one name set. Code cannot declare two kinds under one name.

Within a class-like body, methods, constants, and enum cases share one name set.
Properties have `$` names and use a separate set.

This rule removes any guess from a type name. Once Whim resolves a symbol, its
kind defines the type meaning above.

## Type identifiers

When code needs one key for any type, use `Whim\Type\id::<T>()`. For a value's
runtime type, use `Whim\Type\of($value)`. These functions avoid trying to turn a
type into source text, which cannot preserve aliases, bindings, and loaded
symbol identity without ambiguity.
