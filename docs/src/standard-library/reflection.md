# Reflection

`Whim\Reflection` gives read-only access to loaded declarations, types, and
values.

## Find declarations

Each symbol kind has its own lookup function:

- `reflect_class`, `reflect_interface`, and `reflect_enum` find class-like
  symbols.
- `reflect_type_alias` and `reflect_newtype` find named types.
- `reflect_function` and `reflect_constant` find functions and constants.
- `reflect_symbol` finds any named symbol.
- `reflect_class_like` accepts a class name or an object.

Each function returns `null` for a missing name or the wrong symbol kind. The
second argument to `reflect_symbol` controls autoloading. Name-based lookup
functions may invoke the registered autoloader.

```whim
namespace Example;

use Whim\Reflection;

final class User {
  public function __construct(public int $id) {}
}

$class = Reflection\reflect_class('Example\\User');
assert!($class != null);
assert!($class->getName() == 'Example\\User');
assert!($class->getProperty('id')?->isPromoted());
```

`get_loaded_symbols()` returns symbols loaded in the current engine. Its
optional `Whim\Symbol\SymbolKind` argument filters the result. This function
does not invoke an autoloader.

## Origin and source

Each declaration has one `DeclarationOrigin`:

- `Core`: Rust code in the engine.
- `Extension`: code from a Whim artifact, including the standard library.
- `User`: source that the program loads.

Only user declarations have a `SourceLocation` or a docblock. Core and
extension declarations return `null` for both. A source location gives the
file, byte offsets, lines, and columns. `getDocumentation()` returns the whole
docblock, including `/**` and `*/`.

Declarations list their attributes. `getAttributes::<T>()` returns
attributes whose class fits `T`. `getAttributesByName()` matches an exact class
name. `AttributeReflection::newInstance()` creates an instance of the
attribute. No other reflection call constructs a user value.

## Symbols and members

The `Whim\Reflection\Symbol` namespace has one reflection class for each named
symbol kind.

On a class-like reflection, `getMethods()`, `getProperties()`, and
`getConstants()` include inherited members. `getDeclaredMethods()`,
`getDeclaredProperties()`, and `getDeclaredConstants()` return direct
declarations only. A class reflection gives its parent, interfaces, constructor,
destructor, flags, and attribute definition. An enum reflection gives its cases
and backing type.

The `Whim\Reflection\Member` namespace covers methods, properties, class
constants, and enum cases. Member lookup functions return `null` for a missing
name. A method lists the parent or interface methods it implements. A property
gives its type, default value, and promoted, readonly, and static flags.

## Generics

A generic declaration lists its type parameters in source order. Each
`TypeParameterReflection` gives its owner, position, variance, bounds, and
default.

A `TypeEnvironmentReflection` maps each type parameter to its type argument.
The declaration forms part of a type parameter's identity, so two parameters
named `T` use separate keys. Object and callable reflections include bindings
from parent classes and interfaces.

`getSpecialization()` returns the type arguments that a class or object passes
to a parent class or interface. `TypeReflection::resolve()` replaces type
parameters with arguments from a type environment. Its second argument
supplies the called class type for `static`.

## Types

Three functions return type reflections:

- `reflect_type::<T>()` reflects the reified type `T`.
- `reflect_type_of($value)` reflects a value's runtime type.
- `reflect_type_id($id)` finds the type for an engine-local
  `Whim\Type\TypeId`.

`TypeReflection` reports the type kind, text, resolved state, and type ID. For a
resolved type, `accepts()` tests a value, `equals()` compares types, and
`isSubtypeOf()` tests the subtype relation.

`Whim\Reflection\Type` has a reflection class for each type form: primitive
values, literals, integer ranges, string lengths, named types, unions,
intersections, negation, functions, collections, shapes, class names, tuples,
wildcards, type parameters, and `static`.

`StringLengthTypeReflection` reports the least byte length and the optional
greatest byte length.

`toString()` returns text for logs and error messages. The text is neither a
type ID nor valid source code. `getId()` and `equals()` compare types.

## Live values

`ObjectReflection` keeps a strong reference to its object and reports:

- its class and reified class type;
- its full type environment;
- every instance property, including private properties from parent classes;
- each property's declared type, current value type, and whether it is
  initialized.

`PropertyValueReflection::getValue()` throws `UninitializedPropertyError` when
the property is uninitialized.

`reflect_callable()` reports a callable's declaration, function type, type
bindings, bound object, called class, captured values, and bound arguments.

`reflect_newtype_value()` returns the outer newtype reflection, or `null` for a
value that has no newtype. `getBackingValue()` returns the value inside that
newtype. Pass the result to `reflect_newtype_value()` to inspect a nested
newtype.
