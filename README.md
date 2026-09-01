# Whim

Whim is an experimental programming language built for exploration.

> [!WARNING]
> Whim is a toy. Do not use it in production. Every release may add, remove,
> or redesign any part of the language. Whim has no compatibility promise,
> release schedule, or production support.

Whim checks types and keeps generic type arguments at run time. It has
value-based collections, pattern matching, classes, interfaces, enums, and
cooperative tasks.

```whim
final readonly class Box<T> {
  public function __construct(public T $value) {}
}

function describe(mixed $value): string {
  return match ($value) {
    $box @ Box<int> => 'integer: ' . $box->value,
    $box @ Box<string> => 'text: ' . $box->value,
    $_ => 'other',
  };
}

write_line!(describe(new Box::<int>(42)));
```

## What Whim includes

The `whim` executable runs source, formats code, prints bytecode, provides a
language server, and manages Git dependencies. The standard library is written
mainly in Whim. It covers async I/O, files, processes, networking, TLS, HTTP,
WebSockets, SQLite, PostgreSQL, dates, encodings, and common data formats.

We also maintain packages in the [Trifle group on Codeberg](https://codeberg.org/trifle).

## Install

Install the latest release on macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://carthage.software/whim.sh | bash
```

You can also download an archive from [GitHub Releases]. After installation, run:

```console
whim --version
```

The container image is available at `ghcr.io/carthage-software/whim`.

Whim supports macOS on x86-64 and Arm64, and glibc-based Linux on x86-64,
Arm64, and RISC-V 64.

To build Whim from source, install Rust 1.98 or later and run:

```console
cargo build --locked --release
./target/release/whim --version
```

[GitHub Releases]: https://github.com/carthage-software/whim/releases

## Learn Whim

The [Whim book] documents the language, command-line tools, package manager,
and standard library. Start with [Installation], then write
[Your First Program].

[Whim book]: https://whim.carthage.software/
[Installation]: https://whim.carthage.software/usage/installation.html
[Your First Program]: https://whim.carthage.software/usage/getting-started.html

## Development

Run the Rust and Whim test suites with:

```console
cargo test --locked --workspace --all-features
cargo build --locked --release --bin whim
./target/release/whim tests/run.whim
```

The [Justfile](Justfile) provides shorter development commands.

## Reports

Whim accepts issue reports but not pull requests. Read
[CONTRIBUTING.md](CONTRIBUTING.md) before opening an issue. Follow
[SECURITY.md](SECURITY.md) to report a security problem.

## License

Use Whim under either the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT).
