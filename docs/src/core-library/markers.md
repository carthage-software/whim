# Compiler Markers

These marker attributes tell the compiler how it may treat a declaration. Most
application code does not need them.

## Inline control

- `#[AlwaysInline]` asks the optimizer to inline a resolved function or method.
- `#[NeverInline]` forbids inlining.
- `#[Cold]` marks a rare function or method and keeps it out of line.

These markers do not change a program's result. They may change its bytecode,
speed, code size, and stack trace. Measure before adding them.

## Frameless

`#[Frameless]` marks an eligible function or method that can run without a call
frame. The callable must take no parameters and return a literal value. The
compiler rejects other shapes.

Frameless calls cannot hold normal call state. Use the marker only for tiny
constant accessors supplied by the standard library.

## Inheritance checks

`#[ConsistentConstructor]` makes child constructors keep a signature that is
compatible with the parent constructor.

`#[ConsistentGenerics]` makes inherited generic bounds stay equal through the
class family.

Both markers belong on classes. They let code use a class family without
finding a different construction or generic contract on one child.

## Stub

`#[Stub]` gives source tools the signature of a symbol whose implementation
already exists in the language core. The compiler checks that the real symbol
exists and matches the stub, then skips the source body.

The optional `reason` says why the symbol is a stub. The optional `issue`
records the tracked speed work for a temporary built-in implementation.

```whim,norun
namespace Whim\_Private;

use Whim\Marker\Stub;

#[Stub]
function getmypid(): (1..) {}
```

Only the standard library should declare stubs. A normal package must provide
the code it declares.
