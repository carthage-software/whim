# Appendix D: Grammar Guide

This guide shows the main source forms. It does not replace the parser. The
chapters linked from each section define the checks and runtime rules.

The notation uses:

- `|` for a choice;
- `?` for an optional part;
- `*` for zero or more parts;
- `+` for one or more parts;
- quoted text for source tokens.

```text
attributes      := attribute-list+
attribute-list  := "#[" attribute ("," attribute)* ","? "]"
attribute       := qualified-name call-arguments?

qualified-name  := "\\"? identifier ("\\" identifier)*
variable        := "$" identifier
literal         := "null" | "true" | "false"
                 | integer-literal | float-literal | string-literal
signed-integer-literal
                := "-"? integer-literal
```

The lexical chapter defines identifier and literal bytes. The bare identifier
The grammar reserves `_` even where this guide says `identifier`.

## Source file

```text
source-file     := shebang? source-item*

source-item     := namespace-declaration
                 | use-declaration
                 | function-declaration
                 | class-declaration
                 | interface-declaration
                 | enum-declaration
                 | type-alias
                 | newtype-declaration
                 | constant-declaration
                 | statement
```

A file may mix declarations and statements. A namespace declaration can apply
to the rest of the file or hold a braced source body.

```text
namespace-declaration
                := "namespace" qualified-name ";"
                 | "namespace" qualified-name "{" source-item* "}"

use-declaration := "use" use-items ";"
use-items       := use-item ("," use-item)*
                 | qualified-name "\\" "{" use-item
                   ("," use-item)* ","? "}"
use-item        := qualified-name ("as" identifier)?
```

See [Namespaces and Imports](../language/namespaces.md) and [Loading
Files](../language/loading.md).

## Functions and parameters

```text
function-declaration
                := attributes? "function" function-name type-parameters?
                   parameter-list return-type? block

parameter-list  := "(" (parameter ("," parameter)* ","?)? ")"

parameter       := attributes? parameter-modifier* type? variable
                   ("=" expression)?
parameter-modifier
                := visibility | "readonly"

return-type     := ":" type

type-parameters := "<" type-parameter ("," type-parameter)* ","? ">"

type-parameter  := variance? identifier bounds? default-type?
variance        := "in" | "out"
bounds          := ":" type ("+" type)*
default-type    := "=" type
```

A closure replaces the function name with a parameter list and may add a
`use` capture list. A short closure uses `fn` and captures outer variables
without a capture list. Its body is one expression or a block.

```text
closure         := attributes? "function" type-parameters? parameter-list
                   capture-list? return-type? block

capture-list    := "use" "(" (variable ("," variable)* ","?)? ")"

short-closure   := attributes? "fn" type-parameters? parameter-list
                   return-type? short-closure-body

short-closure-body
                := "=>" expression | block
```

Visibility and `readonly` on a parameter promote it to a property and are
valid only in a class constructor.

See [Functions](../language/functions.md) and [Closures and Short
Closures](../language/callables.md).

## Classes

```text
class-declaration
                := attributes? class-modifier* "class" identifier
                   type-parameters? class-parent? class-interfaces?
                   sealed-family? class-body

class-modifier  := "abstract" | "final" | "readonly"
class-parent    := "extends" named-type
class-interfaces
                := "implements" named-type ("," named-type)*
sealed-family   := "for" named-type ("," named-type)*

class-body      := "{" class-member* "}"
class-member    := property | class-constant | method
```

Properties include `$` in their names. Methods and class constants do not.

```text
property        := attributes? property-modifier+ type?
                   variable ("=" expression)? ";"

property-modifier
                := visibility | "static" | "readonly"

class-constant  := attributes? constant-modifier+ "const" type? identifier
                   "=" expression ";"
constant-modifier
                := visibility | "final"

method          := attributes? method-modifier+ "function"
                   member-name type-parameters? parameter-list
                   return-type? (block | ";")

method-modifier := visibility | "abstract" | "final" | "static"
visibility      := "public" | "protected" | "private"
```

A constructor parameter with a visibility word declares a promoted property.
`readonly` may appear before or after the visibility word. A property,
constant, or method has exactly one visibility word. The other modifiers may
appear in any order, but each may appear only once.

See [Classes and Properties](../language/classes.md) and [Inheritance and
Visibility](../language/inheritance.md).

## Interfaces and enums

```text
interface-declaration
                := attributes? "interface" identifier type-parameters?
                   interface-parents? sealed-family? interface-body

interface-parents
                := "extends" named-type ("," named-type)*

interface-body  := "{" interface-member* "}"
interface-member
                := property | class-constant | method

enum-declaration
                := attributes? "enum" identifier enum-backing?
                   enum-interfaces? "{" enum-member* "}"

enum-backing    := ":" type
enum-interfaces := "implements" named-type ("," named-type)*
enum-member     := enum-case | class-constant | method
enum-case       := attributes? "case" member-name
                   ("=" expression)? ";"
```

See [Interfaces and Sealed Families](../language/interfaces.md) and
[Enums](../language/enums.md).

## Aliases, newtypes, and constants

```text
type-alias      := attributes? "type" identifier type-parameters?
                   "=" type ";"

newtype-declaration
                := attributes? "newtype" identifier type-parameters?
                   "=" type ";"

constant-declaration
                := attributes? "const" identifier "=" expression ";"
```

See [Aliases and Newtypes](../core-library/functions-and-constants.md).

## Statements

```text
statement       := block
                 | ";"
                 | expression ";"
                 | if-statement
                 | while-statement
                 | do-while-statement
                 | for-statement
                 | foreach-statement
                 | try-statement
                 | using-statement
                 | final-local-statement

block           := "{" statement* "}"

if-statement    := "if" "(" expression ")" block
                   ("else" (if-statement | block))?

while-statement := "while" "(" expression ")" block

do-while-statement
                := "do" block "while" "(" expression ")" ";"

for-statement   := "for" "(" expression-list? ";"
                   expression-list? ";" expression-list? ")" block

foreach-statement
                := "foreach" "(" expression "as" foreach-target
                   ("=>" foreach-target)? ")" block

foreach-target  := assignment-target
assignment-target
                := variable | property-access | static-property-access
                 | array-index | array-append
                 | tuple-destructure | dict-destructure

tuple-destructure
                := "(" destructure-item ","
                   (destructure-item ("," destructure-item)*)? ","? ")"
destructure-item
                := assignment-target
                 | assignment-target "=" expression
                 | "..." assignment-target?
dict-destructure
                := "dict" "[" (expression "=>" assignment-target
                   ("," expression "=>" assignment-target)* ","?)? "]"

expression-list := expression ("," expression)*

final-local-statement
                := "final" variable "=" expression ";"
```

See [Statements and Loops](../language/statements.md).

## Error handling and cleanup

```text
try-statement   := "try" block catch-clause* else-clause? finally-clause?

catch-clause    := "catch" "(" type variable? ")" guard? block
guard           := "if" "(" expression ")"
else-clause     := "else" block
finally-clause  := "finally" block

using-statement := "using" "(" using-binding
                   ("," using-binding)* ","? ")" block
using-binding   := bind-target "=" expression
```

A `try` statement needs at least one catch, else, or finally clause.

See [Throwing and Catching](../semantics/error-handling.md) and [Resources and
Cleanup](../core-library/overview.md).

## Expressions

```text
expression      := literal
                 | interpolated-string
                 | variable
                 | constant-name
                 | "(" expression ")"
                 | tuple-literal
                 | vec-literal
                 | vec-fill
                 | dict-literal
                 | closure
                 | short-closure
                 | match-expression
                 | "new" class-expression call-arguments?
                 | "break" integer-literal?
                 | "continue" integer-literal?
                 | "return" expression?
                 | "throw" expression
                 | unary-expression
                 | binary-expression
                 | assignment-expression
                 | call-expression
                 | partial-call
                 | member-expression
                 | index-expression
                 | construct-expression
```

`class-expression` is a named class, `self`, `parent`, `static`, a type
parameter, or an expression that yields a class-name string. The operator
appendix gives the precedence that turns the broad forms above into one tree.

Calls may use positional or named arguments. `?` in an argument place creates
a partial call. `...` in a call place creates a first-class callable or leaves
later partial-call parameters open.

```text
call-arguments  := "(" (call-argument ("," call-argument)* ","?)? ")"
call-argument   := (parameter-name ":")? (expression | "?" | "...")
```

`...` must be the sole or final placeholder. Ordinary calls do not accept
placeholders.

Collection literals use these forms:

```text
tuple-literal   := "(" expression "," ")"
                 | "(" expression "," expression
                   ("," expression)* ","? ")"

vec-literal     := "vec" "[" (vec-item ("," vec-item)* ","?)? "]"
vec-item        := expression | "..." expression
vec-fill        := "vec" "[" expression ";" expression "]"

dict-literal    := "dict" "[" (dict-item ("," dict-item)* ","?)? "]"
dict-item       := expression "=>" expression | "..." expression
```

See [Expressions](../language/expressions.md), [Operators and
Arithmetic](../language/operators.md), and [First-Class and Partial
Calls](../language/partial-calls.md).

## Match patterns

```text
match-expression
                := "match" "(" expression ")" "{"
                   match-arm ("," match-arm)* ","? "}"

match-arm       := pattern "=>" expression

pattern         := union-pattern
union-pattern   := as-pattern ("|" as-pattern)*
as-pattern      := primary-pattern ("@" union-pattern)?
primary-pattern := variable
                 | type
                 | "(" pattern ")"
                 | tuple-pattern
                 | vec-pattern
                 | dict-pattern

tuple-pattern   := "(" pattern ("," pattern)*
                   ("," trailing-pattern)? ","? ")"
vec-pattern     := "vec" "[" (pattern ("," pattern)*)?
                   ("," trailing-pattern)? ","? "]"
dict-pattern    := "dict" "[" (dict-pattern-entry
                   ("," dict-pattern-entry)*)?
                   ("," trailing-pattern)? ","? "]"
dict-pattern-entry
                := (string-literal | signed-integer-literal) "=>" pattern
trailing-pattern
                := "..." pattern?
```

Variables bind. Types and literals check. `@` requires both patterns to match
the same value. Tuple, vec, and dict patterns may nest. Their final item may
use `...` to accept, check, or bind the rest.

See [Match and Destructuring](../language/patterns.md).

## Types

```text
type            := union-type
union-type      := intersection-type ("|" intersection-type)*
intersection-type
                := prefix-type ("&" prefix-type)*
prefix-type     := "!" prefix-type
                 | "=" prefix-type
                 | primary-type

primary-type    := built-in-type
                 | named-type
                 | literal-type
                 | range-type
                 | string-length-type
                 | tuple-type
                 | vec-type
                 | dict-type
                 | array-type
                 | callable-type
                 | classname-type
                 | "(" type ")"

named-type      := qualified-name ("<" type-list ">")?
                   ("::" identifier ("<" type-list ">")?)?
                 | "self" ("::" identifier ("<" type-list ">")?)?
                 | "parent"
                 | "static"
vec-type        := "vec" ("<" type ">")? | "vec" "[" shape-items? "]"
dict-type       := "dict" ("<" type "," type ">")?
                 | "dict" "[" dict-shape-items? "]"
array-type      := "array" ("<" type "," type ">")?
callable-type   := "fn" | "fn" "(" callable-parameters? ")" ":" type
classname-type  := "classname" "<" type ">"

type-list       := type ("," type)* ","?
built-in-type   := "null" | "bool" | "int" | "float" | "string"
                 | "object" | "mixed" | "never" | "void"
literal-type    := literal | "-" (integer-literal | float-literal)
range-type      := signed-integer-literal (".." | "..=")
                   signed-integer-literal?
                 | (".." | "..=") signed-integer-literal
string-length-type
                := "string" "[" (integer-literal | string-length-range) "]"
string-length-range
                := integer-literal (".." | "..=") integer-literal?
                 | (".." | "..=") integer-literal

tuple-type      := "(" type "," ")"
                 | "(" type "," tuple-type-tail ","? ")"
                 | "(" trailing-type ")"
tuple-type-tail := type ("," type)* ("," trailing-type)?
                 | trailing-type
trailing-type   := "..." type?

shape-items     := type ("," type)* ("," trailing-type)? ","?
dict-shape-items
                := dict-shape-entry ("," dict-shape-entry)*
                   ("," dict-shape-rest)? ","?
dict-shape-entry
                := (string-literal | integer-literal) "=>" type
dict-shape-rest := "..." "<" type "," type ">"

callable-parameters
                := callable-parameter ("," callable-parameter)* ","?
callable-parameter
                := "="? type
```

See [Runtime Type Checks](../semantics/type-system.md), [Unions,
Intersections, and Ranges](../language/type-composition.md), and [Collection
and Callable Types](../language/structural-types.md).
