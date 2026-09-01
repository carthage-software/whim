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

Each use creates its own attribute value. Source order stays intact.

## Reading attributes

The `Whim\Attribute` functions read stored attributes:

- `has_attribute($object, $class)` checks the object's class.
- `get_attribute::<T>($object)` gets class attributes of type `T`.
- `get_attributes($object)` gets all attributes on the object's class.
- `get_function_attributes($name)` gets function attributes.
- `get_method_attributes($object, $method)` gets method attributes.
- `get_property_attributes($object, $property)` gets property attributes.
- `get_constant_attributes($object, $constant)` gets class constant
  attributes.
- `get_enum_case_attributes($case)` gets enum case attributes.
- `get_parameter_attributes($object, $method, $parameter)` gets method
  parameter attributes by name or position.

The member functions throw `InvalidArgumentException` when the named member
does not exist. They return an empty vec when the member exists but has no
attributes.

Attributes are values, not comments. The compiler rejects a bad target, a bad
argument, a missing attribute class, or an illegal repeat before the program
runs.
