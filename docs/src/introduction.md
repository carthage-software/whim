# The Whim Programming Language

Whim is an experimental programming language inspired by PHP and Hack. As a
research project, it explores language ideas by putting them into practice,
with the hope that useful ones can contribute to PHP’s future.

> Do not use Whim in production.

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
