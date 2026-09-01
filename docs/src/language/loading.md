# Loading Files

Whim can compile and run another source file while a program runs.

## `require!`

`require!($path)` resolves the path, compiles the file, links its symbols, and
runs its file-scope statements:

```whim,norun
require!(directory!() . '/support.whim');
```

`directory!()` returns the directory of the current source file. Use it for a
path that should not depend on the process working directory.

A source file returns `null`. A parse, compile, link, or file error throws. A
symbol declared by a successful load becomes available to later code.

`require!` runs the file each time. A file that declares symbols cannot usually
run twice because the second load would redeclare them.

## `require_once!`

`require_once!($path)` loads a resolved path at most once:

```whim,norun
require_once!(directory!() . '/vendor/autoload.whim');
```

A later call for the same resolved path returns `null` without running the file
again. Circular `require_once!` calls stop when they reach a file whose load
has started but not ended.

If `require!` loaded a path first, `require_once!` also treats it as loaded.

## Compile and link order

Whim compiles a loaded file as one unit. Declarations in that unit can refer to
each other even when the source declares them later.

The linker rejects duplicate names before it publishes the new unit. A failed
load does not replace an existing symbol.

## Autoloading

When Whim needs an unknown symbol, it may ask registered autoloaders to define
it. Type checks, generic arguments, property types, constants, and ordinary
calls may all trigger autoloading.

An autoloader must return `true` only after it has defined the requested symbol.
See [Autoloading](../core-library/autoload.md) for the API and generated Git
dependency loader.
