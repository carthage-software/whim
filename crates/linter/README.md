# whim-linter

Whim's source linter follows Mago's rule registry, typed settings, node dispatch,
scope tracking, and rule layout. Each rule checks the syntax nodes it needs and
reports `annotate_snippets::Group` values. A diagnostic callback lets editor
clients consume the same spans, rule codes, levels, and messages. The crate does
not parse files, render reports, or apply fixes.

The rules derive from Mago's linter.
Whim-specific checks cover closures, pattern bindings, enums, and language constructs.

See [Linting](../../docs/src/usage/linting.md) for rules and settings.
