# The `whim` Command

The `whim` program runs source, formats it, prints bytecode, and manages Git
dependencies.

Use `whim --help` or `whim COMMAND --help` for the installed version's exact
options.

## Run

The short and explicit forms are equal:

```console
whim app.whim one two
whim run app.whim one two
```

The first argument after the file has index zero in
`Whim\Env\get_arguments()`.

Use `-` as the file to read source from standard input:

```console
printf "write_line!('hello');\n" | whim -
```

Source read this way cannot use `embed!` because it has no directory.

Global options must come before the source file:

| Option | Effect |
| --- | --- |
| `--colors auto|always|never` | choose colored output |
| `--config PATH` | load settings from another `whim.toml` |

Use `--` before a source path that starts with `-`.

`whim run` reads the nearest `whim.toml`, loads the bundled standard-library
artifact, then compiles the entry file and files that the program loads. It
does not inspect `whim.lock` or `vendor/`.

Runtime settings belong in `[runtime]`:

```toml
[runtime]
optimizations = "on"
call-depth = 10000
cycle-threshold = 10001
full-trace = false
```

Each field is optional. For one run, use `WHIM_OPTIMIZATIONS`,
`WHIM_CALL_DEPTH`, `WHIM_CYCLE_THRESHOLD`, or `WHIM_FULL_TRACE`. Environment
values override the file:

```console
WHIM_OPTIMIZATIONS=off whim app.whim
WHIM_FULL_TRACE=true whim app.whim
```

## Format

```console
whim fmt
whim fmt src/ tests/
whim fmt --check
```

With no paths, the command finds the nearest `whim.toml` and formats the project
from that file's directory. It skips `vendor/` and `.git/`. A command with no
paths fails when it cannot find a manifest.

The command also accepts files and directories. An explicit directory ignores
`format.include` but honors exclusions. An explicit file bypasses both.
`--check` reports files that would change and writes nothing.

Project selection belongs in `[format]`:

```toml
[format]
include = ["**/*.whim"]
exclude = ["src/generated/**"]
```

Patterns use `/` and are relative to the manifest directory. Exclusion wins.
The default inclusion is `**/*.whim`; `vendor/` and `.git/` are always excluded
from directory walks.

Layout settings can come from the nearest `whim.toml`, a file passed through the
global `--config` option, or command options:

- `--print-width N`
- `--tab-width N` or `--tab-size N`
- `--use-tabs true|false`
- `--end-of-line lf|crlf`

Command options override file settings.

## Disassemble

```console
whim disassemble app.whim
WHIM_OPTIMIZATIONS=off whim disassemble app.whim
printf "write_line!('hello');\n" | whim disassemble -
```

This command compiles the program and prints its register bytecode. It does not
run the entry file. `WHIM_OPTIMIZATIONS=off` prints the form before
optimization.

## Language server

```console
whim language-server
```

The language server speaks LSP over standard input and output. It provides
keyword completion, snippets, formatting, keyword colors, folding, selection
ranges, and occurrence highlights. It does not index the project or provide
symbol navigation.

## Project commands

`init`, `add`, `remove`, `install`, and `update` change dependency state.
`show`, `why`, `suggestions`, and `fund` inspect it. `why-not` tests a new
requirement against the graph. It creates and locks the project cache and may
fetch Git tags, so it needs a writable project and network access.

These commands search the current directory and its parents for `whim.toml`,
except `init`, which creates a project in the current directory. `whim init
--no-git` skips Git setup.

Package command failures leave the old manifest, lock, vendor tree, and loader
together. See [Git Dependencies](dependencies.md).

## Log output

`WHIM_LOG` sets the CLI log filter. `WHIM_LOG=off` disables all log output,
including error-level records. A CLI command can therefore fail with no error
text while still returning a nonzero status. This is intended.

Log errors are not Whim exceptions. Disabling log output does not catch or
change a thrown value, an uncaught throwable, or a `panic!` trace.

## Exit status

The CLI returns zero on success. `exit!($status)` selects another status.
`panic!` and an uncaught throwable use 255. CLI errors return a nonzero status.
Error text goes to standard error; program output goes to the handle used by
its write calls.
