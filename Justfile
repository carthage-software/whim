# List every recipe.
default:
    @just --list

# Build the release interpreter used by scripts.
build:
    cargo build --locked --release --bin whim

# Run the Rust test suite; shares build artifacts with `just build`.
test:
    cargo nextest run --locked --workspace --cargo-profile release

# Run the Rust test suite on the dev profile, keeping the engine's
# debug-checked invariants active.
test-debug:
    cargo nextest run --locked --workspace

# Run the Whim-native test suite; optional filter and parallelism.
suite filter='' parallelism='10': build
    {{justfile_directory()}}/target/release/whim \
        {{justfile_directory()}}/tests/run.whim '{{filter}}' {{parallelism}}

# Run the Whim-native test suite on the debug binary: slower, but the
# engine's debug-checked invariants stay active.
suite-debug filter='' parallelism='10':
    cargo build --locked --bin whim
    WHIM_LOG=info {{justfile_directory()}}/target/debug/whim \
        {{justfile_directory()}}/tests/run.whim '{{filter}}' {{parallelism}}

# Disassemble a program without executing it.
dump file:
    {{justfile_directory()}}/target/release/whim disassemble {{file}}
