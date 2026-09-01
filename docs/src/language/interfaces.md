# Interfaces and Sealed Families

An interface states which members an object provides. It may require methods,
constructors, properties, and constants.

```whim
interface Record {
  public const string KIND = 'record';
  public readonly int $id;

  public function label(): string;
}

final class User implements Record {
  public function __construct(public readonly int $id) {}

  public function label(): string {
    return self::KIND . ' ' . $this->id;
  }
}
```

Every interface member states its visibility. Interface properties cannot have
storage or default values. A constructor parameter in an interface cannot use
property promotion because the interface cannot declare storage.

## Method requirements

A method without a body is a requirement:

```whim
interface Writable {
  public function write(string $bytes): void;
}
```

A concrete class must supply a compatible method. Staticness, visibility,
parameters, return type, and generic parameters form part of that contract.

An interface may give a method a body. A class then inherits that default unless
it supplies its own compatible method.

```whim
interface Named {
  public readonly string $name;

  public function displayName(): string {
    return $this->name;
  }
}

final class Person implements Named {
  public function __construct(public readonly string $name) {}
}

write_line!(new Person('Ada')->displayName());
```

## Constructor requirements

An interface can require a constructor:

```whim
interface Buildable {
  public function __construct(int $id, int $revision);
}

final class Widget implements Buildable {
  public function __construct(public int $id, public int $revision) {}
}
```

The class constructor must accept every call allowed by the interface
constructor.

## Property requirements

An interface may require a public property:

```whim
interface MutableName {
  public string $name;
}

interface FixedName {
  public readonly string $name;
}
```

Property types are invariant. A class that implements `MutableName` must expose
a writable `string` property. A class that implements `FixedName` must expose a
readonly `string` property.

## Constants

An interface constant has a value. Implementing classes inherit it.

```whim
interface Format {
  public const string NAME = 'json';
}

final class JsonFormat implements Format {}

write_line!(JsonFormat::NAME);
```

An implementing class cannot redeclare that constant. Whim also rejects a class
that inherits conflicting constants from two interfaces.

## Extending interfaces

An interface may extend more than one interface:

```whim
interface HasId {
  public readonly int $id;
}

interface HasLabel {
  public function label(): string;
}

interface Entity extends HasId, HasLabel {}
```

The child interface contains all inherited contracts. A class must satisfy them
as one set. Whim rejects inherited method and constant name conflicts.

## `self` in a contract

`self` in an interface method resolves to the implementing class at runtime.

```whim
interface Copyable {
  public function copy(): self;
}

final class Item implements Copyable {
  public function copy(): self {
    return $this;
  }
}
```

## Generic interfaces

Interfaces may have reified type parameters and variance:

```whim
interface Source<out T> {
  public function read(): T;
}

interface Sink<in T> {
  public function write(T $value): void;
}
```

`out T` may appear only in output positions. `in T` may appear only in input
positions. An unmarked type parameter is invariant. The [generics
chapter](generics.md) gives the full rules.

## Sealed interfaces

An interface can list the symbols allowed to sit directly below it:

```whim
interface Outcome for Success, Failure {}
final class Success implements Outcome {}
final class Failure implements Outcome {}
```

Any other direct implementor or child interface fails to link. Listed child
interfaces may define their own lists. See [Inheritance and
Visibility](inheritance.md#sealed-families) for the full rule.
