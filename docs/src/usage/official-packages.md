# Official Packages

The Whim project maintains
[Trifle packages on Codeberg](https://codeberg.org/trifle). Each package has
its own repository.

These packages use names under `Trifle\`. Their Git tags define their
versions. Install them through Whim's Git package manager.

| Package                                                      | Description                                                  | Install                                                    |
| ------------------------------------------------------------ | ------------------------------------------------------------ | ---------------------------------------------------------- |
| [`ansi`](https://codeberg.org/trifle/ansi)                   | ANSI colours, styles, cursor movement, and screen controls.  | `whim add git+ssh://git@codeberg.org/trifle/ansi`          |
| [`args`](https://codeberg.org/trifle/args)                   | Typed command-line parsing and generated usage.              | `whim add git+ssh://git@codeberg.org/trifle/args`          |
| [`cache`](https://codeberg.org/trifle/cache)                 | Stampede-safe local LRU and database caches.                 | `whim add git+ssh://git@codeberg.org/trifle/cache`         |
| [`config`](https://codeberg.org/trifle/config)               | Typed configuration from values, files, and the environment. | `whim add git+ssh://git@codeberg.org/trifle/config`        |
| [`console`](https://codeberg.org/trifle/console)             | Terminal output, tables, progress, and prompts.              | `whim add git+ssh://git@codeberg.org/trifle/console`       |
| [`cqrs`](https://codeberg.org/trifle/cqrs)                   | Command and query buses with events, logs, and traces.       | `whim add git+ssh://git@codeberg.org/trifle/cqrs`          |
| [`diff`](https://codeberg.org/trifle/diff)                   | Myers diffs for values and text, with unified output.        | `whim add git+ssh://git@codeberg.org/trifle/diff`          |
| [`dotenv`](https://codeberg.org/trifle/dotenv)               | `.env` parsing and environment layering.                     | `whim add git+ssh://git@codeberg.org/trifle/dotenv`        |
| [`event`](https://codeberg.org/trifle/event)                 | Typed events with hierarchy-aware listeners.                 | `whim add git+ssh://git@codeberg.org/trifle/event`         |
| [`expect`](https://codeberg.org/trifle/expect)               | Fluent expectations for tests.                               | `whim add git+ssh://git@codeberg.org/trifle/expect`        |
| [`fake`](https://codeberg.org/trifle/fake)                   | Deterministic fake data generated from a seed.               | `whim add git+ssh://git@codeberg.org/trifle/fake`          |
| [`feed`](https://codeberg.org/trifle/feed)                   | Atom and RSS feed generation.                                | `whim add git+ssh://git@codeberg.org/trifle/feed`          |
| [`humanize`](https://codeberg.org/trifle/humanize)           | Human-readable durations, sizes, counts, and numbers.        | `whim add git+ssh://git@codeberg.org/trifle/humanize`      |
| [`lock`](https://codeberg.org/trifle/lock)                   | Leased mutual exclusion over pluggable stores.               | `whim add git+ssh://git@codeberg.org/trifle/lock`          |
| [`log`](https://codeberg.org/trifle/log)                     | Structured logging with composable loggers.                  | `whim add git+ssh://git@codeberg.org/trifle/log`           |
| [`markdown`](https://codeberg.org/trifle/markdown)           | CommonMark and GitHub-flavoured Markdown parsing.            | `whim add git+ssh://git@codeberg.org/trifle/markdown`      |
| [`markdown-ansi`](https://codeberg.org/trifle/markdown-ansi) | Markdown rendering for styled terminal text.                 | `whim add git+ssh://git@codeberg.org/trifle/markdown-ansi` |
| [`markdown-html`](https://codeberg.org/trifle/markdown-html) | Markdown rendering for HTML.                                 | `whim add git+ssh://git@codeberg.org/trifle/markdown-html` |
| [`money`](https://codeberg.org/trifle/money)                 | Integer money, ISO 4217 currencies, and lossless allocation. | `whim add git+ssh://git@codeberg.org/trifle/money`         |
| [`otp`](https://codeberg.org/trifle/otp)                     | HOTP, TOTP, and authenticator provisioning.                  | `whim add git+ssh://git@codeberg.org/trifle/otp`           |
| [`qr`](https://codeberg.org/trifle/qr)                       | QR code generation with matrix and SVG output.               | `whim add git+ssh://git@codeberg.org/trifle/qr`            |
| [`queue`](https://codeberg.org/trifle/queue)                 | Message queues with retries, workers, and pluggable stores.  | `whim add git+ssh://git@codeberg.org/trifle/queue`         |
| [`seal`](https://codeberg.org/trifle/seal)                   | Sealed payloads and signed JWTs with key rotation.           | `whim add git+ssh://git@codeberg.org/trifle/seal`          |
| [`semver`](https://codeberg.org/trifle/semver)               | Semantic versions and Cargo-style requirements.              | `whim add git+ssh://git@codeberg.org/trifle/semver`        |
| [`template`](https://codeberg.org/trifle/template)           | Logic-less Mustache templates rendered from JSON values.     | `whim add git+ssh://git@codeberg.org/trifle/template`      |
| [`theme`](https://codeberg.org/trifle/theme)                 | Paints and themes for terminal text.                         | `whim add git+ssh://git@codeberg.org/trifle/theme`         |
| [`trace`](https://codeberg.org/trifle/trace)                 | Timed spans recorded through structured logs.                | `whim add git+ssh://git@codeberg.org/trifle/trace`         |

Codeberg hosts all [Trifle package repositories](https://codeberg.org/trifle).

After you add `Trifle\Diff` and load `vendor/autoload.whim`, you can compare two
sequences:

```whim,norun
use Trifle\Diff;
use Trifle\Diff\Operation;

$edits = Diff\diff::<string>(vec['a', 'b', 'c'], vec['a', 'c', 'd']);

foreach ($edits as $edit) {
  $marker = match ($edit->operation) {
    Operation::Keep => ' ',
    Operation::Delete => '-',
    Operation::Insert => '+',
  };

  write_line!($marker . ' ' . $edit->value);
}
```

See [Git Dependencies](dependencies.md) for manifests, locks, updates, and
autoloading.
