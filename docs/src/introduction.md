# The Whim Programming Language

> Whim is a toy programming language. Do not use it in production.

Whim is a small language with strict runtime types. Its syntax will feel
familiar if you know PHP or Hack, but Whim follows its own rules.

```whim
function greet(string $name): string {
  return 'Hello, ' . $name . '!';
}

write_line!(greet('Ada'));
```

Whim has reified generics, value-based arrays, classes, interfaces, enums,
pattern matching, async tasks, and a large standard library. The `whim`
command runs and formats source files, prints bytecode, and manages Git
dependencies.

This book explains the language as it works now. Whim has no promise of
backward compatibility. A later release may change or remove any rule in this
book.

Try Whim in the [Whim playground](https://play.whim.sh/) without installing it.

Start with [Installation](usage/installation.md), then write
[Your First Program](usage/getting-started.md).
