# Reflection

`Whim\Reflection` inspects loaded files, declarations, types, and values.

## Files

`get_loaded_files()` returns loaded `FileReflection` values, sorted by path
with anonymous inputs first. `reflect_file($path)` finds a loaded file or
returns `null`. Both functions read loaded metadata without loading or
executing a file.

A file reflection provides:

- `getPath()`: the recorded source path, or `null` for an anonymous input such
  as standard input.
- `getOrigin()`: the file's `DeclarationOrigin`, matching its symbols' origin.
- `getLocation()`: the whole source span for a user file with a path, or `null`
  otherwise.
- `getAttributes::<T>()` and `getAttributesByName($class)`: file attributes in
  source order. Each attribute's `getTarget()` returns its `FileReflection`.
- `getDocumentation()`: `null`; files have no declaration documentation.
- `getSymbols($kind = null)`: the file's named symbols, sorted by fully
  qualified name. Pass a `Whim\Symbol\SymbolKind` to select one kind.
- `hasTopLevelCode()`: whether the source contains executable statements
  outside function and method bodies, including statements inside namespaces.

```whim
use Whim\Reflection;
use Whim\Symbol\SymbolKind;

foreach (Reflection\get_loaded_files() as $file) {
  write_line!($file->getPath() ?? '<anonymous>');
  foreach ($file->getSymbols(SymbolKind::Function) as $function) {
    write_line!('  ' . $function->getName());
  }
}
```

Lookup matches the recorded path exactly, including its bytes. Use a path from
`getPath()` or `SourceLocation::getFile()`. Artifacts keep their individual
source files; the artifact container itself is not an extra source file. If a
path has been loaded more than once, lookup and listing use its latest load.
Anonymous inputs remain separate entries even when they share a diagnostic
label.

Every symbol reflection has `getFile()`, which returns its defining
`FileReflection` or `null` for a symbol defined in Rust. This works for classes,
interfaces, enums, functions, constants, type aliases, and newtypes, including
symbols from anonymous inputs. A symbol keeps its defining file even when the
same path is loaded again later.

Symbols include declarations across all namespaces in the file. Methods,
closures, imports, and stub declarations backed by another definition are not
separate symbols in this list. Files without named symbols still appear in
`get_loaded_files()`.

The compiler records `hasTopLevelCode()` before optimization. Unreachable
statements still count, even if optimization removes them. Imports,
declarations, and file attributes do not count. Initializers
within declarations do not count either, so this flag does not guarantee that
loading the file has no side effects.

`FileReflection` implements `DeclarationReflection`. The
[attribute syntax](../language/attributes.md#file-attributes) for files is
`#![...]`; the target flag is `Attribute::TARGET_FILE`.

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
attribute.

## Symbols and members

The `Whim\Reflection` namespace has one reflection class for each named
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

## Invoke callables and create instances

`FunctionReflection::invoke($typeArguments = vec[], $arguments = vec[])` calls
the reflected function. `CallableReflection::invoke()` takes the same
arguments and calls the `fn` value retained by `reflect_callable()`. Both
implement `Whim\Reflection\InvokableReflection`.

Type arguments are a `vec<TypeReflection>` in type parameter order. Value
arguments are a `vec<mixed>` in parameter order. Omitted arguments use the
callable's defaults. Whim checks argument counts, types, bounds, where
constraints, and return types as it does for a direct call. It does not infer
type arguments from values.

```whim
use Whim\Reflection;

function identity<T>(T $value): T {
  return $value;
}

$function = Reflection\reflect_function('identity');
$int = Reflection\reflect_type::<int>();
assert!($function->invoke(vec[$int], vec[42]) == 42);

$prefix = 'Hello, ';
$greet = Reflection\reflect_callable(fn(string $name): string => $prefix . $name);
assert!($greet->invoke(vec[], vec['world']) == 'Hello, world');
```

A callable keeps its captures, bound receiver, class scope, type
bindings, and partial arguments. Pass only the remaining arguments to a
partial application. A specialized `fn` uses its stored type arguments; passing
more type arguments throws `TypeError`. `ClosureReflection`
describes a closure declaration. To invoke a closure with its captures,
reflect the `fn` value with `reflect_callable()`.

`MethodReflection::invoke($target, $typeArguments = vec[], $arguments = vec[])`
requires an object for an instance method and a `classname<object>` for a
static method. The target's class must be the declaring class or one of its
subtypes. An unrelated class fails even if it has a method with the same name.
The call uses the target's override when one exists. For a static call, the
target class binds `static`, including `new static()` and a `static` return
type. The method's declaring class still determines `self` and `parent`.

Visibility uses the scope that calls `invoke()`. Holding a reflection of a
private or protected method grants no extra access. Instance methods get
class type arguments from the object; the supplied type arguments bind only
the method's own type parameters.

`ClassReflection::instantiate($typeArguments = vec[], $arguments = vec[])`
creates an object and runs its constructor. Type arguments bind the class;
value arguments go to the constructor. Normal constructor visibility and
instantiation rules apply. Constructors and destructors cannot be called
through `MethodReflection::invoke()`.

Every supplied `TypeReflection` must be resolved. Use `resolve()` with a type
environment to replace unbound parameters before passing them. Invocation
returns the callable's result, or `null` for a `void` callable. Exceptions
propagate to the caller, and calls may suspend as usual. Discarding an invoked
callable's result still enforces its `#[MustUse]` attribute.

## Generics

A generic declaration lists its type parameters in source order. Each
`TypeParameterReflection` gives its owner, position, variance, bounds, and
default.

`FunctionLikeReflection::getWhereConstraints()` returns
`Whim\Reflection\Generic\WhereConstraintReflection` entries in source order,
including repeated constraints. Functions, methods, and closures without a clause
return an empty vector.
Each constraint provides:

- `getParameter()`: the exact `TypeParameterReflection` it constrains.
- `getBound()`: its upper bound as a `TypeReflection`, with generic parameters left unresolved.
- `getDeclaringFunctionLike()`: the function, method, or closure that declares the constraint.
- `getPosition()`: its zero-based position in the clause.
- `getLocation()`: its source span for user code, or `null` for an artifact.

Inherited methods retain their original declaring method and type parameters.
Resolve a bound with an object or callable's `TypeEnvironmentReflection` to
substitute its type arguments. Where constraints remain separate from
`TypeParameterReflection::getBounds()`, which describes the parameter declaration.
Artifacts preserve these constraints.

A `TypeEnvironmentReflection` maps each type parameter to its type argument.
The declaration forms part of a type parameter's identity, so two parameters
named `T` use separate keys. Object and callable reflections include bindings
from parent classes and interfaces.

`ParameterReflection::getDefaultValue($environment = null)` evaluates a
parameter's default with the supplied type bindings. Pass the callable's type
environment for a generic function or method, or the object's or specialized
class type's environment for a constructor parameter. Each call evaluates the
default again, including any object or closure it creates.

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

The primitive `uint` has kind `TypeKind::Uint`. Literal, integer-range, enum
backing-value, and dict-shape-key reflection preserve unsigned values and their
full 64-bit magnitude. Their value and bound results can therefore be `uint`.

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

`reflect_callable()` returns a `CallableReflection` for a `fn` value. It reports
the declaration, function type, type bindings, bound object, called class,
captured values, and bound arguments. Its `getDeclaration()` returns a
`FunctionReflection`, `MethodReflection`, or `ClosureReflection`. These share
the `FunctionLikeReflection` interface for declaration metadata.

Three predicates distinguish callable values:

| Value | `isClosure()` | `isFirstClassCallable()` | `isPartialFunctionApplication()` |
| --- | --- | --- | --- |
| `fn(int $x): int => $x` | `true` | `false` | `false` |
| `foo(...)` or `$object->method(...)` | `false` | `true` | `false` |
| `foo::<int>(...)` | `false` | `true` | `false` |
| `foo(?, 'fixed')` or `foo(?, ?)` | `false` | `false` | `true` |

Exactly one predicate returns `true`. Partial function application reports
`isPartialFunctionApplication()` even when it binds no values or starts from a
closure. `getDeclaration()` still reports the original declaration. Binding
type arguments or using `(...)` on an existing callable preserves its kind.

`reflect_newtype_value()` returns the outer newtype reflection, or `null` for a
value that has no newtype. `getBackingValue()` returns the value inside that
newtype. Pass the result to `reflect_newtype_value()` to inspect a nested
newtype.

## Object shape types

`reflect_type::<#{ value: int, ... }>()` returns an
`ObjectShapeTypeReflection` with kind `TypeKind::ObjectShape`. `isOpen()` reports
whether extra public instance properties are allowed. `getProperties()` returns
`ObjectShapePropertyReflection` values in source order; each has `getName()` and
`getType()` methods. The usual `accepts()`, `isSubtypeOf()`, `resolve()`, and type
identity operations apply to object shapes. `accepts()` checks current property
values on every call.
