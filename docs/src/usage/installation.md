# Installation

Whim supports macOS on x86-64 and Arm64. It also supports glibc-based Linux on
x86-64, Arm64, and RISC-V 64. Run it through the `whim` command.

## Install a release

Download the archive for your system. Put the `whim` file in a directory on
your `PATH`. Then check it:

```console
whim --version
```

## Build from source

Install Rust 1.98 or later. From the repository root, run:

```console
cargo build --locked --release
```

The build produces the `whim` executable at `target/release/whim`.

## Source files

Whim source files use `.whim`. Compiled artifacts use `.whia`.

Continue with [Your First Program](getting-started.md).
