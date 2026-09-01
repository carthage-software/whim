# Standard Library Rules

The standard library uses the `Whim\` namespace. The CLI loads its compiled
artifact before it compiles the entry file. Programs do not need to require it.

Whim also maintains [official packages](../usage/official-packages.md) under
the `Trifle\` namespace. They do not ship in the standard-library artifact.

## Public and private code

Public declarations live under domain namespaces such as `Whim\Str`,
`Whim\HTTP`, and `Whim\Database`.

The language core supplies names under `Whim\_Private`. A domain `_Private`
namespace holds Whim code used by that domain. Both forms are implementation
details. Application code must not call them.

Most library code uses Whim. The core supplies operations that need the
operating system, event loop, heap, or a tested low-level library.

## Input and output types

Library functions take the broadest safe input and return the narrowest useful
output. A read-only collection function normally accepts
`Refine\Iterable<K, V>`, not only a vec or dict. A function that needs random
access or a known size accepts an array.

Refined types state checked bounds in the signature:

```whim
use Whim\Refine\NonEmptyString;
use Whim\Refine\PositiveInt;

function repeat_name(NonEmptyString $name, PositiveInt $count): vec<string> {
  return Whim\Vec\fill($count, $name);
}
```

The engine checks those bounds at the call.

## Missing values

The library uses `null|T` when `T` cannot itself contain `null`. It uses
`Option<T>` when `T` is generic and may contain `null`, since `Some(null)` and
`None` must stay different.

This rule keeps concrete APIs small without losing a value in generic code.
Function parameters follow the same rule.

## Errors

Library operations throw on failure. `Result` is available for APIs where an
error is ordinary data, but it is not the default error path.

The `Whim\Unwind` hierarchy covers general argument, state, range, and runtime
errors. A domain adds its own exception when callers need to distinguish that
failure. Cancellation uses `CancelledException`.

## Value style

Configuration and message objects are often readonly. Methods named `with...`
return a changed clone:

```whim,norun
$configuration = Whim\TCP\ListenConfiguration::default()
  ->withReuseAddress(true)
  ->withBacklog(256);
```

Mutable resources, pools, buffers, handles, and task controls use stateful
methods. Any object that owns a closeable operating-system resource also closes
it from its destructor. Use `using` for prompt cleanup.

## Naming

Functions use `snake_case`; methods use `camelCase`. Types use `PascalCase` and
constants use `SCREAMING_SNAKE_CASE`.

An operation driven by a callback often ends in `_by`. Its key-aware form often
ends in `_with_key` or `_by_key`. A `try...` method returns at once and reports
that it cannot proceed; a blocking form may suspend the current task.

## Stability

Whim has no compatibility promise. The standard library may add, remove,
rename, or redesign an API in any release. Read the docs that ship with the
binary you run.
