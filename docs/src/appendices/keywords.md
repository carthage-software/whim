# Appendix A: Keywords

Whim gives keywords three reservation levels. The level controls where the
word may still act as a name.

| Level | Function name | Constant name | Member name |
| --- | :---: | :---: | :---: |
| full | no | no | yes |
| soft | yes | no | yes |
| contextual | yes | yes | yes |

A member name follows `->`, `?->`, or `::`, or appears in a class-like member
declaration. Every keyword may appear there.

## Full keywords

These words may appear only as keywords or member names:

```text
break       catch       continue    do          else
false       finally     fn          for         foreach
function    if          match       new         null
parent      return      self        static      throw
true        try         using       while
```

## Soft keywords

These words may also name functions:

```text
as          is
```

They cannot name constants because a bare use could conflict with the
operator.

## Contextual keywords

These words may name functions, constants, and members when their position
makes the meaning clear:

```text
abstract    array       bool        case        class
classname   const       default     dict        enum
extends     final       float       implements  in
int         interface   mixed       namespace   never
newtype     object      out         private     protected
public      readonly    string      type        use
vec         void
```

## The `_` identifier

Whim reserves `_` even though it is not a keyword token. It cannot name a
namespace symbol, class-like member, parameter, type parameter, or import.

`$_` is valid because variable names include the `$` prefix. Whim treats it as
an ordinary variable.

## Literal words

`true`, `false`, and `null` are full keywords and literal values. They may also
appear in type positions as literal types.

## Case

Keywords use the lowercase spellings above. Names are case-sensitive, so a
different case is a different identifier:

```whim
function Match(): int {
  return 42;
}

assert!(Match() == 42);
```

Use such names with care. The formatter does not change identifier case.
