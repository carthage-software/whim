# Installation

Whim supports macOS and FreeBSD on x86-64 and Arm64, Linux with glibc on
x86-64, Arm64, and RISC-V 64, and Linux with musl on x86-64 and Arm64.
Run it through the `whim` command.

## Shell installer

Install the latest release on macOS, Linux, or FreeBSD:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://whim.sh/install.sh | bash
```

On Alpine Linux, install the tools needed by the installer first:

```sh
apk add bash curl ca-certificates
```

On FreeBSD 14.4 or later, install the tools and PostgreSQL client library first:

```sh
pkg install bash curl postgresql18-client
```

FreeBSD builds use the system PostgreSQL client library, including when the
script does not use PostgreSQL.

Pass a version to install a specific release:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://whim.sh/install.sh | bash -s -- --version=0.2.0
```

The installer verifies build attestations when a compatible
[GitHub CLI](https://cli.github.com/) is available.

On Linux, it detects glibc or musl and downloads the matching build.

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

On FreeBSD, use the `latest` package repository for Rust 1.98 or later, then
install the build tools and PostgreSQL client library:

```sh
pkg install rust cmake git pkgconf postgresql18-client
```

## Source files

Whim source files use `.whim`. Compiled artifacts use `.whia`.

Continue with [Your First Program](getting-started.md).
