# The Whim Programming Language

The book is an [mdBook](https://rust-lang.github.io/mdBook/) rooted at
[`src/SUMMARY.md`](src/SUMMARY.md).

Build it with:

```console
mdbook build docs
```

Serve it while editing with:

```console
mdbook serve docs --open
```

`tests/docs/samples.whim` runs each `whim` code fence. Use `whim,norun` for an
example that should compile without running, `whim,compile_fail` for a planned
error, and `whim,ignore` only for an incomplete fragment.
