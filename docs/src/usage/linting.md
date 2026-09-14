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

| Field      | Value                                                                                             |
| ---------- | ------------------------------------------------------------------------------------------------- |
| `path`     | Source path, using the same spelling as text output                                               |
| `code`     | Rule code, `lint-attribute` for an invalid lint attribute, or `syntax` or `read` for a file error |
| `level`    | `error`, `warning`, `info`, `note`, or `help`                                                     |
| `message`  | Diagnostic message                                                                                |
| `span`     | `start` and `end` objects, each with a byte `offset`; `null` for read errors                      |
| `rendered` | Full plain-text diagnostic, including annotations and help; `null` for read errors                |

Offsets start at zero, and the end offset is exclusive. They count UTF-8 bytes,
not characters. The array follows file discovery order and is empty (`[]`) when
there are no findings. Whim writes it in batches as files finish, so it does not
hold the whole report in memory.

JSON output has no ANSI color codes, even with `--colors always`. Logs and setup
errors, such as an invalid config or a missing target path, stay on standard
error. `--json` does not change file selection, rule settings, or exit status.

## Rules

| Rule                         | Default level | Checks                                                          |
| ---------------------------- | ------------- | --------------------------------------------------------------- |
| `tagged-todo`                | warning       | TODO comments without an owner or issue tag                     |
| `tagged-fixme`               | warning       | FIXME comments without an owner or issue tag                    |
| `loop-does-not-iterate`      | warning       | Loops with an unconditional break or return                     |
| `yoda-conditions`            | help          | Comparisons with a variable before a literal or constant        |
| `use-compound-assignment`    | help          | Assignments such as `$x = $x + 1`                               |
| `no-parameter-shadowing`     | warning       | Catch and foreach targets that reuse a parameter name           |
| `prefer-while-loop`          | note          | For loops with no initializer or increment                      |
| `prefer-early-return`        | help          | One if statement wrapping a function body                       |
| `prefer-early-continue`      | help          | One if statement wrapping a loop body                           |
| `no-multi-assignments`       | warning       | Chained assignments in one expression                           |
| `readable-literal`           | warning       | Long numeric literals without underscore separators             |
| `no-assign-in-argument`      | warning       | Assignments used directly as call arguments                     |
| `no-assign-in-condition`     | warning       | Assignments used directly as if or while conditions             |
| `no-dead-store`              | warning       | Local assignments overwritten before a read                     |
| `no-redundant-variable`      | warning       | Final local values that are never read                          |
| `no-empty-catch-clause`      | warning       | Catch clauses that do not handle their error                    |
| `excessive-nesting`          | warning       | Block depth above the set limit                                 |
| `cyclomatic-complexity`      | error         | Callable or class-like branch counts above the set limit        |
| `no-redundant-static`        | help          | `static` references where a final class makes `self` sufficient |
| `no-redundant-final`         | help          | Final methods in final classes or enums                         |
| `no-redundant-else`          | help          | Else branches after a branch that always exits                  |
| `no-redundant-continue`      | help          | A continue at the end of a loop body                            |
| `no-self-assignment`         | warning       | Variables or properties assigned to themselves                  |
| `no-redundant-use`           | warning       | Unused or same-namespace imports                                |
| `no-redundant-readonly`      | help          | Readonly properties repeated in a readonly class                |
| `no-redundant-nullsafe`      | help          | Nullsafe property links made redundant by `??`                  |
| `constant-condition`         | help          | If conditions with a result fixed by the source                 |
| `inline-variable-return`     | help          | A local assigned only for the next return                       |
| `no-redundant-string-concat` | help          | Adjacent string literals joined with `.`                        |
| `no-empty-comment`           | note          | Comments that contain no text                                   |
| `no-literal-password`        | error         | Literal passwords, tokens, secrets, and API keys                |
| `no-insecure-comparison`     | error         | Direct comparisons of password or token values                  |
| `sensitive-parameter`        | error         | Sensitive parameters missing the marker attribute               |
| `no-debug-symbols`           | warning       | `debug!` calls in application code                              |
| `disallowed-symbols`         | warning       | Exact symbols named in the rule settings                        |

Use a tag such as `TODO(#123)`, `TODO(@owner)`, or `FIXME(owner)`.

`no-dead-store` checks each function, method, or closure on its own. It tracks
branches separately and counts closure captures as reads. It skips parameters,
scoped bindings, variables used in catch or finally blocks, and names that start
with an underscore. It reports an overwritten assignment, not every unused
variable.

`no-redundant-variable` reports the last value written to a local when no later
code reads it. It keeps loop-carried reads, branch joins, closure captures, and
finally blocks in scope. Its help keeps expressions that have side effects.

The security rules infer sensitive values from names ending in `password`,
`token`, `secret`, `apikey`, or `api_key`, without regard to case. They provide
local checks, not a proof that a program is secure. Use `Whim\Hash\equals()` for
secret string comparisons and `Whim\Password\verify()` for password hashes.
`sensitive-parameter` accepts the exact `Whim\Marker\SensitiveParameter`
attribute, including an imported alias, and skips Boolean-only parameters.

## Scoped lint attributes

Use `Whim\Lint\Allow`, `Warn`, `Deny`, and `Forbid` to set a rule's level
on a declaration:

```whim
use Whim\Lint\{Allow, Deny};

#[Deny('no-debug-symbols')]
class HeaderParser {
  #[Allow(
    rule: 'no-insecure-comparison',
    reason: 'The token is not security-sensitive',
  )]
  public function hasToken(
    string $value,
    #[Allow(
      rule: 'sensitive-parameter',
      reason: 'This is an HTTP grammar token',
    )]
    string $token,
  ): bool {
    return $value == $token;
  }
}
```

`Allow` suppresses the rule. `Warn` reports a warning. `Deny` reports an error.
`Forbid` reports an error and prevents any nested scope from lowering its level.
Trying to lower a forbidden rule produces an error on the conflicting attribute,
even when the code has no finding for that rule. A nested `Deny` keeps an
enclosing `Forbid` in force.

The setting covers the declaration, its signature, and its contents. It ends
at the declaration's boundary. Nested declarations can override an outer
`Allow`, `Warn`, or `Deny`. Attributes follow source order; the last setting for
a rule wins unless an earlier `Forbid` prevents it. The setting follows source
scope, not calls or inheritance.

All four attributes are repeatable. They support classes, interfaces, enums,
functions, methods, closures, parameters, properties, constants, enum cases,
type aliases, newtypes, and files. Each takes a required `string $rule` and an
optional `null|string $reason = null`, both exposed as public properties.
Use one attribute per rule.

Use `#![...]` to set a rule's level for the whole file:

```whim
namespace App;

use Whim\Lint\Allow;

#![Allow(
  rule: 'no-debug-symbols',
  reason: 'This script inspects runtime values',
)]

debug!(vec[1, 2, 3]);
```

File settings cover all code and comments in the file, including code before
the attribute and code in other namespaces. File attributes follow source
order, and each name uses the namespace and imports at its location.
Declaration attributes override file `Allow`, `Warn`, or `Deny` settings,
even when the file attribute appears later. A file `Forbid` prevents any
declaration or later file attribute from lowering the level.

Arguments may use positions or names. Named arguments can appear in either
order, and a positional rule can be followed by `reason: ...`. The reason
explains the setting; the linter only evaluates the rule argument.

Rule names must use exact lint codes. The linter reads string literals,
parentheses, and string concatenation, so these are equivalent:

```whim
use Whim\Lint\Allow;

#[Allow('sensitive-parameter')]
#[Allow(rule: 'sensitive' . '-' . 'parameter')]
function token_value(string $token): string {
  return $token;
}
```

The linter does not resolve constants or run code to read a rule name. An
unknown rule, missing rule argument, duplicate or extra argument, non-string
rule name, or rule expression that it cannot read produces a `lint-attribute`
error. These errors cannot be suppressed or lowered by another attribute or
by rule settings.

Project settings supply the starting policy. `Warn`, `Deny`, or `Forbid` can
enable a disabled rule within a declaration or file. File and per-rule
exclusions still apply. The CLI and language server use the same attribute
rules, and `minimum_fail_level` still determines whether a warning fails the
command.

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

[lint.rules.readable-literal]
min-digits = 5

[lint.rules.prefer-early-return]
max-allowed-statements = 0

[lint.rules.no-empty-comment]
preserve-single-line-comments = true

[lint.rules.cyclomatic-complexity]
threshold = 15
method-threshold = 8

[lint.rules.disallowed-symbols]
symbols = [
    'Legacy\run',
    { name = 'Legacy\Client', help = 'Use Modern\Client.', level = 'error' },
]

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
The two early-exit rules allow no wrapped statements by default.
`readable-literal.min-digits` defaults to 5. `no-empty-comment` checks line and
block comments unless `preserve-single-line-comments` is true.
`cyclomatic-complexity.threshold` defaults to 15. Its optional
`method-threshold` also checks each method.

`disallowed-symbols` uses exact, case-sensitive symbol names. It checks
declarations, imports and aliases, attributes, types, patterns, direct calls,
construction, constants, and static members. A leading `\` in a setting is
optional. Each entry may set its own help text and level. An empty list reports
nothing. Dynamic callable or class values and late-bound `static` references do
not have one source-resolved name, so the rule skips them.

The language server uses each workspace's `whim.toml`, including file and rule
exclusions. These filters also apply to open files. Editors can request a full
workspace check when a manifest exists; without one, Whim reports the missing
config and checks only open files. Open buffers take precedence over disk
contents. Restart the server after creating or changing the config.
