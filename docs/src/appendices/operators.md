# Appendix B: Operators

The table runs from loose binding to tight binding. Parentheses override this
order.

| Level | Operators | Association |
| --- | --- | --- |
| Assignment | `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `**=`, `.=` and compound bit, shift, coalesce, and Boolean assignments | right |
| Coalesce | `??` | right |
| Boolean or | `||` | left |
| Boolean and | `&&` | left |
| Comparison | `==`, `!=`, `<`, `<=`, `>`, `>=`, `<=>` | none |
| Type | `is`, `as`, `?as` | none |
| Pipeline | `|>` | left |
| Concatenation | `.` | left |
| Bitwise or | `|` | left |
| Bitwise xor | `^` | left |
| Bitwise and | `&` | left |
| Shift | `<<`, `>>` | left |
| Addition | `+`, `-` | left |
| Multiplication | `*`, `/`, `%` | left |
| Prefix | `!`, `~`, unary `+` and `-`, prefix `++` and `--` | right |
| Exponentiation | `**` | right |
| Postfix | calls, indexing, `->`, `?->`, `::`, postfix `++` and `--` | left |

Comparisons and type operators do not chain. Write the intended grouping or
combine complete comparisons with `&&`.

## Collection and language operations

Several operations use construct syntax and do not belong in the precedence
table:

- `length!($value)` returns the length of a string or array.
- `contains!($array, $value)` and `contains_key!($array, $key)` search arrays.
- `remove!($array, $key)` removes and returns an entry.
- `swap_remove!($vec, $index)` removes and returns an item without preserving
  vec order.
- `clone!($object, property: $value)` clones an object with changed properties.
- `drop!($resource)` ends ownership immediately and reports a leak.
- `debug!($value)` prints a bounded structural view with its source location.
- `require!` and `require_once!` load source files.

The compiler checks these forms by their own rules.
