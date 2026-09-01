# Generics

Whim generics keep their type arguments at runtime. Classes, interfaces,
functions, methods, closures, aliases, and newtypes may declare type parameters.

```whim
final class Box<T> {
  public function __construct(public T $value) {}
}

function identity<T>(T $value): T {
  return $value;
}

$box = new Box::<int>(42);
$value = identity::<string>('Whim');
```

## Declaring type parameters

Type parameters appear between `<` and `>` after a symbol name.

```whim
final class Pair<A, B> {
  public function __construct(public A $first, public B $second) {}
}
```

Each name must be unique in that list. A nested generic declaration may add its
own parameters:

```whim
final class Box<T> {
  public function __construct(public T $value) {}

  public function map<U>(fn(T): U $transform): Box<U> {
    return new Box::<U>($transform($this->value));
  }
}
```

## Supplying type arguments

A type annotation uses ordinary angle brackets:

```text
Box<int>
dict<string, Box<int>>
```

A constructor or call uses `::<...>`:

```whim
final class Box<T> {
  public function __construct(public T $value) {}
}

function identity<T>(T $value): T {
  return $value;
}

$box = new Box::<int>(1);
$value = identity::<int>(2);
```

The number of arguments must match the number of parameters that lack defaults.
Whim does not infer a missing type argument from a value. Supply it or declare a
default.

This applies to functions, methods, static methods, closures, and arrows.

```whim
$identity = fn<T>(T $value): T {
  return $value;
};

$value = $identity::<string>('text');
```

## Runtime reification

The type argument remains available during the call and in the object.

```whim
function matches<T>(mixed $value): bool {
  return $value is T;
}

assert!(matches::<vec<int>>(vec[1, 2]));
assert!(!matches::<vec<int>>(vec[1, 'two']));
```

An object test includes its type arguments:

```whim
final class Box<T> {}

$box = new Box::<int>();
assert!($box is Box<int>);
assert!(!($box is Box<string>));
```

A generic class may use its parameter in properties, methods, parent types, and
implemented interfaces. Every write and call keeps that binding.

## Defaults

`= Type` gives a type parameter a default.

```whim
final class Box<T = int> {}

function identity<T = int>(T $value): T {
  return $value;
}

$box = new Box();
$value = identity(42);
```

Whim applies the default when the caller omits the argument. The default must
meet the parameter's bound. A default cannot depend on its own parameter before
that parameter has a binding.

Defaults also let a generic callable fit a non-generic callable type. A generic
callable with no defaults still needs type arguments when invoked.

## Bounds

`T: Bound` limits the type arguments accepted for `T`.

```whim
interface Named {
  public function name(): string;
}

function label<T: Named>(T $value): string {
  return $value->name();
}
```

Whim checks each supplied or defaulted type argument against the bound. Bounds
can use any type expression, including unions, ranges, negation, constants, and
other type parameters.

Use `+` when one parameter must meet several bounds:

```whim
interface Named {}
interface Stored {}

function save<T: Named + Stored>(T $value): void {}
```

A bound may depend on another parameter:

```whim
type Weaken<T: W, W> = W;
```

Here `T` must fit `W`, and the alias exposes only `W` at runtime.

## Constructing a type parameter

Code may create a reified type parameter when its bound supplies a constructor.

```whim
interface Constructable {
  public function __construct();
}

function create<T: Constructable>(): T {
  return new T();
}
```

A bound may also supply static methods:

```whim
interface Buildable {
  public static function build(int $seed): static;
}

function build<T: Buildable>(int $seed): T {
  return T::build($seed);
}
```

Calling a method through a type parameter with no matching bound fails. Whim
does not allow class-constant access through a type parameter.

## Variance

Variance controls how one generic type relates to another.

### Covariance

`out T` marks an output type parameter.

```whim
interface Source<out T> {
  public function read(): T;
}
```

If `int` fits `int|string`, then `Source<int>` fits `Source<int|string>`.

A covariant parameter may appear in return types and readonly properties. It
may not appear where the caller can send a value in, such as a writable
property or method parameter.

### Contravariance

`in T` marks an input type parameter.

```whim
interface Sink<in T> {
  public function write(T $value): void;
}
```

If `int` fits `mixed`, then `Sink<mixed>` fits `Sink<int>`.

A contravariant parameter may appear in method parameters. It may not appear as
an output type.

### Invariance

An unmarked parameter is invariant. `Cell<int>` and `Cell<int|string>` are then
different types, and neither fits the other only because their arguments do.

```whim
final class Cell<T> {
  public function __construct(public T $value) {}
}
```

Whim checks variance in aliases, classes, interfaces, functions, methods,
closures, and arrows. A negation reverses the position while Whim checks it.

## Generic inheritance

A child binds its parent's parameters in its `extends` or `implements` clause.

```whim
interface Source<out T> {
  public function read(): T;
}

abstract class Base<T> {
  public function __construct(protected T $value) {}
}

final class IntegerSource extends Base<int> implements Source<int> {
  public function read(): int {
    return $this->value;
  }
}
```

The object is an `IntegerSource`, a `Base<int>`, and a `Source<int>`.

Whim rejects a class that reaches the same generic interface with incompatible
type arguments.

## Generic first-class callables

A first-class callable may remain unbound:

```whim
function identity<T>(T $value): T {
  return $value;
}

$generic = identity(...);
$value = $generic::<int>(42);
```

Calling `$generic(42)` fails because `T` has no binding or default.

Bind the type argument while creating the callable when all calls should use
one type:

```whim
function identity<T>(T $value): T {
  return $value;
}

$integers = identity::<int>(...);
assert!($integers(42) == 42);
```

A bound callable keeps that type and rejects another binding.

## Empty types and covariance

`never` is useful for variant types. A value such as `Result<int, never>` can
fit `Result<int, string>` when the error parameter is covariant: it cannot hold
an error, so the wider error type is safe.

The same rule explains why an empty `vec<never>` fits `vec<int>`.
