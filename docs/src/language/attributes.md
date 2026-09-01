# Attributes

An attribute adds typed data to a declaration or parameter. Whim checks the
attribute class, its target, its arguments, and whether it may repeat.

## Defining an attribute

Mark a class with `Whim\Attribute\Attribute`:

```whim
use Whim\Attribute\Attribute;

#[Attribute(Attribute::TARGET_CLASS)]
final readonly class Table {
  public function __construct(public string $name) {}
}

#[Table('users')]
final class User {}
```

Applying `#[Table('users')]` creates a `Table` value. The arguments follow the
same count and type rules as a normal constructor call. Attribute arguments may
not read local variables.

An attribute class may have no constructor. Apply it without parentheses:

```whim
use Whim\Attribute\Attribute;

#[Attribute(Attribute::TARGET_FUNCTION)]
final class Endpoint {}

#[Endpoint]
function home(): string {
  return '/';
}
```

## Targets

Pass one or more target flags to `#[Attribute]`:

| Flag | Target |
| --- | --- |
| `TARGET_CLASS` | class, interface, or enum |
| `TARGET_FUNCTION` | named function or closure |
| `TARGET_METHOD` | method |
| `TARGET_PROPERTY` | property |
| `TARGET_CLASS_CONSTANT` | class-like constant or enum case |
| `TARGET_PARAMETER` | function, method, or closure parameter |
| `TARGET_TYPE_ALIAS` | type alias |
| `TARGET_NEWTYPE` | newtype |
| `TARGET_CONSTANT` | namespace constant |
| `TARGET_SYMBOL` | any named symbol |
| `TARGET_ALL` | every supported target |

Join flags with `|`:

```whim
use Whim\Attribute\Attribute;

#[Attribute(Attribute::TARGET_FUNCTION | Attribute::TARGET_METHOD)]
final class Timed {}
```

With no flag, an attribute accepts every target.

## Repeated attributes

An attribute may appear once on one target unless its flags include
`IS_REPEATABLE`.

```whim
use Whim\Attribute\Attribute;

#[Attribute(Attribute::TARGET_CLASS | Attribute::IS_REPEATABLE)]
final readonly class Tag {
  public function __construct(public string $name) {}
}

#[Tag('http')]
#[Tag('public')]
final class Handler {}
```

Each use stores its own arguments. Source order stays intact.

## Reading attributes

Use `Whim\Reflection` to inspect attributes:

```whim
use Whim\Attribute\Attribute;
use Whim\Reflection;

#[Attribute(Attribute::TARGET_CLASS | Attribute::IS_REPEATABLE)]
final readonly class Tag {
  public function __construct(public string $name) {}
}

#[Tag('http')]
#[Tag('public')]
final class Handler {}

$handler = Reflection\reflect_class('Handler');
assert!($handler != null);

foreach ($handler->getAttributes::<Tag>() as $attribute) {
  $tag = $attribute->newInstance();
  write_line!($tag->name);
}
```

Every declaration reflection provides `getAttributes::<T>()` and
`getAttributesByName()`. Both return attribute reflections without running an
attribute constructor. `AttributeReflection::newInstance()` constructs a fresh
attribute value.

Use `reflect_function()` for a function. A class-like reflection provides its
methods, properties, and constants. A method reflection provides its
parameters. `reflect_object($case)->getEnumCase()` finds an enum case. Missing
declarations and members return `null`.

See [Reflection](../standard-library/reflection.md) for the complete API.

Attributes are values, not comments. The compiler rejects a bad target, a bad
argument, a missing attribute class, or an illegal repeat before the program
runs.
