# Appendix E: Glossary

### Artifact

A compiled Whim unit stored in a `.whia` file.

### Array

The common type of tuples, vecs, and dicts.

### Attribute

A typed value attached to a declaration, member, or parameter.

### Autoloader

Code that tries to define a symbol when Whim first needs it.

### Backed enum

An enum whose cases each have an int or string value.

### Bound

A type rule that limits a generic type argument.

### Callable

A closure, short closure, first-class function, bound method, or partial call.

### Cancellation token

A value that tells a waiting operation to stop waiting.

### Class family

A class and its parent and child classes.

### Closure

An unnamed function with a block body and an explicit capture list.

### Constant expression

A source form allowed in constants and defaults. It cannot read a local
variable or `$this` directly, but a call inside it may run code, inspect state,
or throw.

### Coroutine

A call stack that may pause and later continue on the event loop.

### Dict

A mutable ordered array with bool, int, or string keys.

### Future

A read-only view of a value or throwable that will arrive later.

### Identity

The rule by which two values refer to the same object or callable.

### Interface

A set of methods, properties, constants, and constructor rules that an object
must meet.

### Literal type

A type that contains one scalar, constant, or enum-case value.

### Newtype

A runtime tag placed on a value that must fit a backing type.

### Nullable type

A union of `null` and another type, written `null|T`.

### Partial call

A callable made from a call expression whose `?` arguments remain open.

### Reified generic

A generic whose type arguments stay available while the program runs.

### Resource

An object whose lifetime code checks with `using` or `drop!`.

### Sealed family

A class or interface that lists the symbols allowed directly below it.

### Strong reference

A reference that keeps an object alive.

### Symbol

A named class, interface, enum, function, constant, alias, or newtype.

### Task

A cooperatively scheduled async call.

### Tuple

An immutable fixed-size array whose positions may have different types.

### Type alias

A transparent name for another type.

### Type identifier

An engine-local integer used to compare or index runtime types.

### Unit enum

An enum whose cases have names but no backing values.

### Value semantics

Assignment and argument passing give arrays independent values. The runtime may
share their storage until one value changes.

### Vec

A mutable dense array with integer keys from zero.

### Weak reference

A reference that observes an object without keeping it alive.

### Wildcard type

`_` inside another type, meaning that a nested type exists but need not match
one fixed type.
