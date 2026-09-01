# Inheritance and Visibility

A class may extend one class and implement any number of interfaces.

```whim
interface Named {
  public function name(): string;
}

class Entity {
  public function __construct(public readonly int $id) {}
}

final class User extends Entity implements Named {
  public function __construct(int $id, private string $label) {
    parent::__construct($id);
  }

  public function name(): string {
    return $this->label;
  }
}
```

Whim rejects class and interface inheritance cycles.

## Dynamic method dispatch

An instance call uses the method on the object's runtime class. This remains
true when a parent method makes the call.

```whim
class Animal {
  public function describe(): string {
    return 'a ' . $this->kind();
  }

  protected function kind(): string {
    return 'animal';
  }
}

final class Dog extends Animal {
  protected function kind(): string {
    return 'dog';
  }
}

write_line!(new Dog()->describe()); // a dog
```

`parent::method()` calls the parent implementation directly.

## Visibility

Any scope may use a `public` member.

The declaring class, its parents, and its children may use a `protected`
member. This access works both ways within one inheritance family. It does not
grant access to an unrelated class.

`private` code belongs only to the class that declares it. A child cannot use
its parent's private member.

Private properties and methods do not take part in normal overriding. A child
may declare its own private member with the same name. Parent code still uses
the parent's member, and child code uses the child's member.

```whim
class ParentValue {
  private string $value = 'parent';

  public function parentValue(): string {
    return $this->value;
  }
}

class ChildValue extends ParentValue {
  private string $value = 'child';

  public function childValue(): string {
    return $this->value;
  }
}
```

## Method overrides

An overriding method must keep a compatible contract:

- It cannot add required parameters.
- It may add optional parameters.
- It may widen visibility, such as `protected` to `public`.
- It may not change an instance method to static or a static method to an
  instance method.
- It may not replace a final method.
- Its parameter and return types must meet the inherited type contract.

Whim checks these rules when it links the classes. An abstract child may leave
an inherited abstract method open. A concrete child may not.

## Property inheritance

Property types are invariant. A child that redeclares an inherited public or
protected property must use the same type. It must also keep the property
readonly or writable as declared.

A child may redeclare a compatible inherited property. The object still has
one slot for that inherited property.

A private parent property is a different slot from a child property with the
same name.

## Constant inheritance

A child inherits visible class and interface constants. A class constant may
replace an inherited class constant with a narrower type:

```whim
class Broad {
  public const int|string VALUE = 1;
}

class Narrow extends Broad {
  public const int VALUE = 2;
}
```

A child cannot replace a final constant.

An inherited method and constant cannot share a name. A class also cannot
replace an interface constant it implements.

## Generic base types

Type arguments form part of the inherited contract:

```whim
interface Source<out T> {
  public function value(): T;
}

final class IntegerSource implements Source<int> {
  public function value(): int {
    return 1;
  }
}
```

The number of base type arguments must match. Each argument must meet its
bound. A class cannot implement incompatible forms of the same generic
interface.

## Sealed families

The `for` clause limits direct children or implementors.

```whim
interface Vehicle for Motorized, Towed {}
interface Motorized extends Vehicle for Car {}
interface Towed extends Vehicle for Trailer {}

final class Car implements Motorized {}
final class Trailer implements Towed {}
```

Only `Motorized` and `Towed` may directly extend or implement `Vehicle`. Only
`Car` may directly extend or implement `Motorized`. Only `Trailer` may directly
extend or implement `Towed`.

Permission continues through a listed child. `Car` is a `Vehicle` because it
implements `Motorized`. The child controls which symbols may sit directly below
it.

A sealed class uses the same form:

```whim
abstract class Event for Login {}
class Login extends Event {}
final class DetailedLogin extends Login {}
```

The linker enforces the family even when its members load from separate files.
