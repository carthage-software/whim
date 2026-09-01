# Installation

Whim supports macOS on x86-64 and Arm64. It also supports glibc-based Linux on
x86-64, Arm64, and RISC-V 64. Run it through the `whim` command.

## Shell installer

Install the latest release on macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://carthage.software/whim.sh | bash
```

Pass a version to install a specific release:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://carthage.software/whim.sh | bash -s -- --version=0.1.0
```

The installer verifies build attestations when a compatible
[GitHub CLI](https://cli.github.com/) is available.

> Note: Release `0.1.0` has no attestation.

## Manual installation

Download the archive for your system from [GitHub Releases](https://github.com/carthage-software/whim/releases). Put the
`whim` file in a directory on your `PATH`. Then check it:

```console
whim --version
```

## Docker

The image at `ghcr.io/carthage-software/whim` supports amd64, arm64, and
RISC-V 64. Mount a project and pass its entry file:

```sh
docker run --rm -v "$PWD:/app" ghcr.io/carthage-software/whim:latest main.whim
```

Each release publishes `latest`, the full version, and the major-minor version.

## Build from source

Install Rust 1.98 or later. From the repository root, run:

```console
cargo build --locked --release
```

The build produces the `whim` executable at `target/release/whim`.

## Source files

Whim source files use `.whim`. Compiled artifacts use `.whia`.

Continue with [Your First Program](getting-started.md).
