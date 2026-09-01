# The Whim standard library

This crate owns the official `Whim` source tree and exposes its compiled
artifact to the Whim CLI.

Every declaration lives under `src/`. Portable implementations are ordinary
Whim code. Symbols supplied by the runtime are declared with `#[Stub]` in
the same namespace and file layout, so editors and documentation tools see the
complete library without making the runtime depend on this crate.

## Built-in or Whim

Write an API in Whim whenever the language and existing `Whim` APIs can
express it. Use a Rust-backed built-in only for facilities Whim cannot reach:

- operating-system operations;
- scheduler and event-loop primitives;
- heap features such as weak references;
- roots the runtime needs before any Whim artifact is loaded.
- Performance-critical operations that cannot be made fast enough in Whim.

The runtime must build and operate without this crate. Built-in code therefore
cannot depend on classes implemented only here. With one failure mode, a
built-in returns null and its Whim wrapper throws. With several failure
modes, it throws its own built-in error, such as
`Whim\_Private\SystemError`.

## Principles

- Type as strictly as possible. Use `Whim\Refine` types such as
  `NonEmptyString` and `Uint32` instead of taking a wide type and checking it
  by hand. A validation helper that only narrows a type is a bug.
- Take general inputs and return specific outputs. Iteration functions accept
  `Iterable`, not a concrete container.
- Errors are exceptions. We chose exceptions over `Result` for error handling
  because they allocate nothing on the happy path. This may be revisited, but
  it is the rule today.
- Platform and engine numbers are linked constants, not functions returning
  tables. Declare each value in Rust, add its `#[Stub]` declaration under
  `src/_Private`, and use it directly. Enums that mirror those numbers are
  int-backed by the constants, as `Filesystem\NodeKind` and `Process\Signal`
  are.
- Comparators use `Whim\Comparison\Ordering`, never bare integers. Types with a
  natural order implement `Whim\Comparison\Order`.
- The library is internally consistent. When two namespaces solve the same
  problem in two ways, one of them is wrong.

## Optional values

`null|T` has a hole when `T` is a type parameter that a caller binds to a type
including null: an absent value and the value null become the same thing.
`Option` exists to keep them apart, and that is the only reason to use it.
Fewer objects also matter, so `Option` is not the default.

- A return type is `Option<T>` only when the value may be absent and `T` might
  itself include null. In practice, this means `T` is a type parameter without
  a null-excluding bound, as in `Vec\first`. A value that may be absent but
  cannot be null stays `null|X`, as in `null|NonEmptyString`.
- A parameter is `Option<T> $value = new None` under the same two conditions:
  it is optional, and `T` might include null. A concrete optional parameter
  stays `null|X $value = null`. A required parameter is just `T $value`.
- Where generic code meets a concrete nullable API, convert explicitly with
  `some()` or `none()`. Do not add a general nullable-to-Option helper. For a
  null-including `T`, it cannot tell the two cases apart, which is the exact
  hole `Option` closes.

## Consistency rules

Argument order:

- Iteration functions take the iterable first, as in `Vec\map` and
  `Dict\filter`.
- `$haystack`, `$needle`, and `$pattern` keep that order everywhere they appear
  together.

Naming:

- Use long descriptive names, never abbreviations. `Impl` is the one allowed
  suffix, on the private class behind a public interface.
- Functions are `snake_case`. Methods are `camelCase`. Classes, interfaces,
  enums, type aliases, newtypes, and type parameters are `PascalCase`.
  Constants are `SCREAMING_SNAKE_CASE`. Variables and parameters are
  `$camelCase`.
- An operation on values is the default name. Its key version takes a `_key`
  suffix, as in `sort` and `sort_by_key`. A version driven by a caller-supplied
  function takes a `_by` suffix, as in `sort_by` and `unique_by`.

Errors:

- Reuse the `Whim\Unwind` hierarchy when a class there fits. A domain-specific
  exception lives in its domain namespace.
- A wrapper that classifies a built-in failure passes the errno as the
  exception code, as `Filesystem\_Private\failure` does.
- Permission bits are octal literals: `0o755`, never `493`.

Markers and docs:

- Every callable with a body carries `#[TrackCaller]`, or exactly one
  infallible marker such as `#[AlwaysInline]`. Abstract interface methods carry
  no execution marker; each implementation decides for itself.
- `#[MustUse]` goes on anything whose result is a bug to discard.
- `#[TraceBoundary]` belongs to `_Private`, and to
  `Whim\OS\FileDescriptor` as the one engine class a trace may cross.
- Every public declaration gets a doc comment, usually one line. Document a
  private declaration only when its contract is not clear from the code. Avoid
  inline comments. Put longer explanations in a commit message or issue.

## Source rules

Directories match namespace components exactly. A class, interface, or enum
lives in a file named after it. Functions live in `functions.whim`, or in a
snake-case file for a coherent group. Constants live in `constants.whim` and
types in `types.whim`. Files contain one namespace and no executable top-level
code unless initialization is intentionally part of the artifact.

`_Private` declarations are implementation details with no compatibility
guarantee. `Whim\_Private` is built-in; domain `_Private` namespaces are
implemented in Whim.

## Artifact build

The build script discovers every `.whim` file under `src/`, compiles the whole
source set with trusted standard-library return types, optimizes and verifies
the result, and serializes one versioned artifact. It then loads the artifact
into a fresh runtime as a build-time validation step.

The artifact retains source text and per-file ranges, so diagnostics and stack
traces use paths such as `<std>/Math/clamp.whim`. The CLI loads the artifact
before it runs application code. Runtime startup never parses or compiles the
standard-library sources.

## Adding to it

Follow these rules and add new behavior under
`tests/conformance/standard-library/`. Public APIs must also be reflected in
the standard-library reference under `docs/`.
