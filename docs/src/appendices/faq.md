# Appendix F: Frequently Asked Questions

## Is Whim related to PHP?

Whim uses syntax from the PHP family. It is not a PHP implementation. It has
its own types, arrays, generics, async work, errors, and standard library.

## Can I run PHP code as Whim code?

No. Some source may look alike, but the languages have different syntax and
runtime semantics.

## Why does Whim check types at runtime?

Whim can load code while it runs. Runtime checks keep each declared contract
in force after that load. This includes generic arguments, ranges, callable
signatures, and collection members.

## Why must conditions return bool?

Whim does not guess whether null, a number, a string, or an array means true.
Write the test you mean.

## Why do arrays use value semantics?

A local array change should not change another variable by accident. The
runtime shares storage until one copy changes, so assignment need not copy all
items at once.

## When should I use null, Option, or Result?

Use `null|T` when `null` cannot also be a valid `T`. Use `Option<T>` when
`Some(null)` must differ from no value. Use `Result<T, E>` when failure is data
that the caller should inspect. Most library failures throw.

## Does Whim use threads?

Normal Whim code runs on one event loop. Tasks can overlap waits, but they do
not run Whim code on several CPU cores at once. Separate bounded worker pools
run blocking SQLite, file, and operating-system work.

## Does Whim support Windows?

No. Whim supports macOS on x86-64 and Arm64, and glibc-based Linux on x86-64,
Arm64, and RISC-V 64.

## Where is the package registry?

There is none. A Git repository is a package identity. SemVer Git tags are its
releases. Whim installs each graph under the current project's `vendor/`.

## Does `whim run` load packages on its own?

No. Run reads settings from the manifest, but it does not inspect the lock or
vendor directory. Require `vendor/autoload.whim` from the application.

## Will Whim keep old code working?

No. Any release may add, change, or remove language rules and library APIs.

## Should I use Whim in production?

No. Whim is a toy for experiments.

## What would make Whim a production project?

<iframe src="https://giphy.com/embed/13B1WmJg7HwjGU" width="100%" height="auto" />
