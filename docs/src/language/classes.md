# Classes and Properties

A class defines objects with identity. Two variables can point to the same
object, and a change through either variable changes that object.

```whim
class Counter {
  public int $value = 0;

  public function increment(): void {
    $this->value++;
  }
}

$first = new Counter();
$second = $first;
$second->increment();
write_line!($first->value); // 1
```

## Declaring a class

A class body may contain properties, constants, and methods. Every member must
state its visibility: `public`, `protected`, or `private`.

```whim
class User {
  public const string KIND = 'user';
  private static int $created = 0;

  public function __construct(public readonly int $id, private string $name) {
    self::$created++;
  }

  public function rename(string $name): void {
    $this->name = $name;
  }

  public function label(): string {
    return $this->id . ': ' . $this->name;
  }

  public static function created(): int {
    return self::$created;
  }
}
```

Properties use `$` in their names. Methods and constants do not. A property may
share a name with a method or constant. Methods and constants may not share a
name with each other. An enum case also uses that member name set.

## Construction

`new` creates an object, then calls `__construct` when the class has one. The
call site must have access to that constructor. Code outside the class can call
only a public constructor.

```whim
final class Point {
  public function __construct(public int $x, public int $y) {}
}

$point = new Point(3, 4);
```

A visibility modifier on a constructor parameter promotes that parameter to a
property. The property keeps the parameter's type and `readonly` modifier.

Without promotion, the constructor assigns properties through `$this`:

```whim
final class Name {
  private string $value;

  public function __construct(string $value) {
    $this->value = $value;
  }

  public function value(): string {
    return $this->value;
  }
}
```

Named arguments work with constructors:

```whim
final class Entry {
  public function __construct(public int $number, public string $text) {}
}

$entry = new Entry(
  text: 'answer',
  number: 42,
);
```

A child constructor does not call its parent on its own. Call
`parent::__construct(...)` when the parent needs setup.

A private constructor can force callers through a factory:

```whim
final class Token {
  private function __construct(public string $value) {}

  public static function from(string $value): Token {
    return new Token($value);
  }
}

$token = Token::from('ready');
```

## Property state

A property may have a default value:

```whim
class Job {
  public string $state = 'waiting';
}
```

A property without a default starts uninitialized. Reading it throws
`Whim\Unwind\UninitializedPropertyError`.

```whim,norun
class Job {
  public string $state;
}

$job = new Job();
$state = $job->state; // throws
```

An initializer may call functions or methods and may create objects. Whim
evaluates a non-static property initializer for each new object. Objects do not
share that value.

```whim
final class Box {
  public function __construct(public int $value) {}
}

final class Holder {
  public Box $box = new Box(1);
}

$first = new Holder();
$second = new Holder();
$first->box->value = 9;
write_line!($second->box->value); // 1
```

Every write checks the property's type. A compound assignment also leaves the
old value in place when its operation or type check fails.

## Readonly properties

A `readonly` property accepts one write. The write must occur in code that can
access the property. Later writes throw `Whim\Unwind\ReadonlyError`.

```whim
class Account {
  public readonly int $id;

  public function __construct(int $id) {
    $this->id = $id;
  }
}
```

A child constructor may initialize an inherited public or protected readonly
property. It may not initialize a private parent property.

`readonly class` makes each instance property readonly, including promoted
properties that omit the word `readonly`.

```whim
readonly class Pair {
  public function __construct(public int $left, public int $right) {}
}
```

A readonly class cannot have static properties. A readonly class may extend
only another readonly class, and a child of a readonly class must also be
readonly.

## Static properties and class constants

Static properties belong to the class family, not to one object. An inherited
static property uses the same stored value.

```whim
class Registry {
  public static int $count = 0;
}

class ChildRegistry extends Registry {}

ChildRegistry::$count = 3;
write_line!(Registry::$count); // 3
```

A class constant has a type and a value:

```whim
final class Limits {
  public const int MAXIMUM = 100;
}

write_line!(Limits::MAXIMUM);
```

`final` prevents a child from replacing a class constant.

Constant values may call functions and methods or create objects. Whim checks
them when it declares the class.

## `$this`, `self`, `parent`, and `static`

`$this` is the current object in an instance method.

`self` names the class that declares the current method. `parent` names its
direct parent. `static` names the class on which the call began, so it supports
late static dispatch.

```whim
class Model {
  public static function create(): static {
    return new static();
  }

  public function kind(): string {
    return 'model';
  }
}

final class Post extends Model {
  public function kind(): string {
    return 'post';
  }
}

write_line!(Post::create()->kind()); // post
```

Use `self`, not `static`, as a parameter type. A return type may use either.

## Abstract and final classes

An abstract class may hold abstract methods. An abstract method has no body. A
concrete child must implement every abstract method before code can create it.

```whim
abstract class Shape {
  abstract public function area(): float;
}

final class Square extends Shape {
  public function __construct(public float $side) {}

  public function area(): float {
    return $this->side * $this->side;
  }
}
```

Code cannot create an abstract class. Code cannot extend a final class or
replace a final method.

`final abstract class` has a narrow use: it groups static code. It may contain
only constants, static properties, and concrete static methods.

```whim
final abstract class Numbers {
  public const int ONE = 1;

  public static function one(): int {
    return self::ONE;
  }
}
```

## Cloning

`clone!($object)` creates a new object with copies of the source properties.
The two objects then have separate property slots.

```whim
final class Point {
  public function __construct(public int $x, public int $y) {}
}

$source = new Point(1, 2);
$copy = clone!($source, y: 9);
$source->x = 7;

write_line!($copy->x); // 1
write_line!($copy->y); // 9
```

Named fields after the object replace properties on the clone. Normal
visibility and readonly rules apply at the call site. A wrong field name or a
non-object source throws `TypeError`. Some built-in classes reject cloning.
Whim has no clone hook.

## Destruction

A class may declare `__destruct(): void`. Whim calls it when no strong
reference can reach the object, including when cycle collection finds an
unreachable cycle.

```whim,norun
final class Lease {
  public function __destruct(): void {
    write_line!('released');
  }
}

$lease = new Lease();
$lease = null;
```

A child inherits its parent's destructor unless it declares one. A child
destructor replaces the parent destructor; call `parent::__destruct()` to run
both.

A destructor may throw. During normal execution, the throw starts at the
statement that removed the last reference. During shutdown, a destructor
failure becomes the program's failure. Keep destructors short and make cleanup
safe to call more than once.
