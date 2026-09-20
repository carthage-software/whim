# Installation

Run Whim through the `whim` command.

## Platform support

Release builds (✅ available, ❌ unavailable):

| Architecture | macOS | Linux (glibc) | Linux (musl) | FreeBSD | Windows (MSVC) |
| ------------ | ----- | ------------- | ------------ | ------- | -------------- |
| x86_64       | ✅ *  | ✅            | ✅ *         | ✅ *    | ✅             |
| x86          | ❌    | ❌            | ❌           | ❌      | ❌             |
| AArch64      | ✅    | ✅ *          | ✅ *         | ✅ *    | ❌             |
| ARM          | ❌    | ❌            | ❌           | ❌      | ❌             |
| RISC-V       | ❌    | ✅ *          | ❌           | ❌      | ❌             |
| LoongArch64  | ❌    | ❌            | ❌           | ❌      | ❌             |
| PowerPC64    | ❌    | ❌            | ❌           | ❌      | ❌             |

`*` marks builds that are not tested in CI. RISC-V builds target 64-bit systems.

CI runs the Rust and Whim test suites on pushes to `main` and pull requests.
FreeBSD and Linux musl builds run only in the release workflow, without running
tests.

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

## Windows

On 64-bit Windows 10 version 1803 or later, download
`whim-<version>-x86_64-pc-windows-msvc.zip` from
[GitHub Releases](https://github.com/carthage-software/whim/releases).
Extract `whim.exe` and add its directory to your user `PATH`.
Open a new terminal and run `whim --version`.

Windows uses the same standard-library API. POSIX-only operations throw
`Whim\Unwind\UnsupportedPlatformException`; see
[Windows platform limits](../standard-library/io.md#windows-platform-limits).

## Manual installation

Download the archive for your system from [GitHub Releases](https://github.com/carthage-software/whim/releases). Put the
`whim` file (`whim.exe` on Windows) in a directory on your `PATH`. Then check it:

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

On Windows, use the `x86_64-pc-windows-msvc` Rust toolchain. Install Visual Studio
Build Tools with the C++ tools and Windows SDK, plus CMake, Perl, and NASM.
The build produces `target\release\whim.exe`. Release builds set
`RUSTFLAGS=-C target-feature=+crt-static` to include the C runtime.
The package manager needs Git for Windows on `PATH`.

On FreeBSD, use the `latest` package repository for Rust 1.98 or later, then
install the build tools and PostgreSQL client library:

```sh
pkg install rust cmake git pkgconf postgresql18-client
```

## Source files

Whim source files use `.whim`. Compiled artifacts use `.whia`.

Continue with [Your First Program](getting-started.md).
