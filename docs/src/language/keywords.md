# Names and Keywords

Whim names are case-sensitive. `User`, `user`, and `USER` are three names.

## Variables and symbols

A variable starts with `$`:

```whim
$value = 42;
```

Declared symbols do not:

```whim
const LIMIT = 10;

function limit(int $value): int {
  if ($value > LIMIT) {
    return LIMIT;
  }

  return $value;
}
```

Classes, interfaces, enums, functions, constants, aliases, and newtypes share
one symbol table. Two such declarations cannot use the same qualified name.

Properties have their own member names. Methods, class constants, and enum
cases share the class-like member table. They cannot reuse one name in the same
family.

## Qualified names

A backslash separates namespace parts:

```text
App\Model\User
```

A leading backslash starts at the global namespace:

```text
\Whim\Json\encode
```

Without that leading slash, Whim resolves a qualified name through imports and
the current namespace.

## The reserved `_` name

The bare name `_` cannot name any symbol or member. Whim uses it as a wildcard,
a match default, and an unnamed generic slot.

Variables include `$`, so `$_` is valid. It is an ordinary variable, not a
discard:

```whim
$_ = 'kept';
assert!($_ == 'kept');
```

## Keyword levels

Whim has three keyword levels.

Full keywords, such as `if`, `match`, and `return`, cannot name a function or a
constant. Soft keywords, `as` and `is`, may name functions but not constants.
Context keywords, such as `class`, `int`, and `readonly`, may name functions or
constants where the parser can tell what they mean.

Every keyword may name a class-like member because `->`, `?->`, `::`, or a
member declaration makes that use clear:

```whim
final class Tokens {
  public const MATCH = 'match';

  public function match(): string {
    return self::MATCH;
  }
}

assert!(new Tokens()->match() == 'match');
```

The [keyword appendix](../appendices/keywords.md) lists every word and its
level.
