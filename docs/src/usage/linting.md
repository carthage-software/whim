# Linting

`whim lint` checks source for common mistakes, unclear code, and security risks.
It does not run the code. All rules start enabled, and linting never changes a
file.

```console
whim lint
whim lint src/ tests/
```

The first form requires a `whim.toml` and checks its project directory. Both
forms skip `vendor/` and `.git/` during directory walks.

The command logs a run summary to standard error. `WHIM_LOG=whim=debug` shows
configuration, discovery, and full counts; `WHIM_LOG=whim=trace` adds file and phase
timings. `WHIM_LOG=error` hides progress logs while keeping lint diagnostics.
See [Log output](cli.md#log-output) for details.

## JSON output

Use `--json` to write one JSON array to standard output:

```console
whim lint --json
whim lint --json src/ > lint.json
```

Each entry contains:

| Field      | Value                                                                              |
| ---------- | ---------------------------------------------------------------------------------- |
| `path`     | Source path, using the same spelling as text output                                |
| `code`     | Rule code, or `syntax` or `read` for a file error                                  |
| `level`    | `error`, `warning`, `info`, `note`, or `help`                                      |
| `message`  | Diagnostic message                                                                 |
| `span`     | `start` and `end` objects, each with a byte `offset`; `null` for read errors       |
| `rendered` | Full plain-text diagnostic, including annotations and help; `null` for read errors |

Offsets start at zero, and the end offset is exclusive. They count UTF-8 bytes,
not characters. The array follows file discovery order and is empty (`[]`) when
there are no findings. Whim writes it in batches as files finish, so it does not
hold the whole report in memory.

JSON output has no ANSI color codes, even with `--colors always`. Logs and setup
errors, such as an invalid config or a missing target path, stay on standard
error. `--json` does not change file selection, rule settings, or exit status.

## Rules

| Rule                      | Default level | Checks                                                          |
| ------------------------- | ------------- | --------------------------------------------------------------- |
| `tagged-todo`             | warning       | TODO comments without an owner or issue tag                     |
| `tagged-fixme`            | warning       | FIXME comments without an owner or issue tag                    |
| `loop-does-not-iterate`   | warning       | Loops with an unconditional break or return                     |
| `yoda-conditions`         | help          | Comparisons with a variable before a literal or constant        |
| `use-compound-assignment` | help          | Assignments such as `$x = $x + 1`                               |
| `no-assign-in-argument`   | warning       | Assignments used directly as call arguments                     |
| `no-assign-in-condition`  | warning       | Assignments used directly as if or while conditions             |
| `no-dead-store`           | warning       | Local assignments overwritten before a read                     |
| `excessive-nesting`       | warning       | Block depth above the set limit                                 |
| `no-redundant-static`     | help          | `static` references where a final class makes `self` sufficient |
| `no-redundant-final`      | help          | Final methods in final classes or enums                         |
| `no-redundant-else`       | help          | Else branches after a branch that always exits                  |
| `no-literal-password`     | error         | Literal passwords, tokens, secrets, and API keys                |
| `no-insecure-comparison`  | error         | Direct comparisons of password or token values                  |
| `no-redundant-continue`   | help          | A continue at the end of a loop body                            |

Use a tag such as `TODO(#123)`, `TODO(@owner)`, or `FIXME(owner)`.

`no-dead-store` checks each function, method, or closure on its own. It tracks
branches separately and counts closure captures as reads. It skips parameters,
scoped bindings, variables used in catch or finally blocks, and names that start
with an underscore. It reports an overwritten assignment, not every unused
variable.

The security rules infer sensitive values from names ending in `password`,
`token`, `secret`, `apikey`, or `api_key`, without regard to case. They provide
local checks, not a proof that a program is secure. Use `Whim\Hash\equals()` for
secret string comparisons and `Whim\Password\verify()` for password hashes.

## Settings

File patterns use `/` and are relative to the manifest directory. Exclusions win.
Explicit directories ignore `include`; explicit files bypass both file filters.
Per-rule exclusions apply to every file.

```toml
[lint]
include = ["**/*.whim"]
exclude = ["src/generated/**"]
minimum_fail_level = "error"

[lint.rules.tagged-todo]
level = "warning"
exclude = ["tests/**"]

[lint.rules.yoda-conditions]
mode = "deny"

[lint.rules.no-assign-in-condition]
ignore-while-statements = true

[lint.rules.excessive-nesting]
threshold = 7
function-like-threshold = 4

[lint.rules.no-literal-password]
enabled = false
```

Each rule accepts `enabled`, `level`, and `exclude`. Rule levels and
`minimum_fail_level` accept `error`, `warning`, `info`, `note`, or `help`.
`minimum_fail_level` controls the exit status without hiding lower-level findings.

`yoda-conditions.mode` defaults to `require`; choose `deny` to keep variables on
the left. `no-assign-in-condition.ignore-while-statements` defaults to `false`
and covers both `while` and `do`/`while`. `excessive-nesting.threshold` defaults
to 7; its optional `function-like-threshold` counts each function body from 1.

The language server uses each workspace's `whim.toml`, including file and rule
exclusions. These filters also apply to open files. Editors can request a full
workspace check when a manifest exists; without one, Whim reports the missing
config and checks only open files. Open buffers take precedence over disk
contents. Restart the server after creating or changing the config.
