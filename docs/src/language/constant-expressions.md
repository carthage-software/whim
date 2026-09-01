# Constants and Initializers

A namespace constant binds one name to one value:

```whim
const ANSWER = 42;
const LABEL = 'answer';

assert!(ANSWER == 42);
```

Namespace constants have no written type. Their value gives them a type. A
class-like constant may state a type, as the class chapter shows.

## Constant expressions

Constants, attribute arguments, parameter defaults, and property defaults use
constant expressions. Such an expression may use:

- scalar literals and existing constants;
- `embed!` with a literal relative path;
- unary and binary operators;
- tuple, vec, and dict literals, including vec and dict spreads;
- a closure with no `use` list and no `$this` use;
- a named class construction;
- function, static method, and method calls whose inputs are constant
  expressions.

Calls may run code, inspect state, and throw. “Constant expression” names the
source forms allowed at the use site. It does not mean pure or compile-time
work. The expression itself cannot read a local variable or `$this` directly.

```whim
final class Box {
  public function __construct(public int $value) {}

  public static function from(int $value): Box {
    return new Box($value);
  }
}

function add(int $left, int $right): int {
  return $left + $right;
}

const TOTAL = add(20, 22);
const BOX = Box::from(TOTAL);

assert!(BOX->value == 42);
```

## Forms that do not qualify

A constant expression cannot use:

- a variable or `$this`;
- assignment, indexing, or a property read;
- interpolation;
- a short closure or a closure capture;
- `match`, `throw`, a partial call, or a language construct other than
  `embed!`;
- `vec[$value; $size]`;
- a class name held in an expression.

The compiler reports which form broke the rule.

## When Whim evaluates values

Whim evaluates a namespace or class constant when it declares that symbol. It
evaluates a static property default when it declares the class.

Whim evaluates an instance property default for each new object. Two objects
do not share an object made by that default. It evaluates an omitted
parameter default for each call.

An attribute argument runs when Whim creates the attribute value for its
target.

## Self-reference and load order

A constant may refer to a constant declared later in the same compiled unit.
Whim resolves the name before it starts the unit. A constant cannot depend on
itself, whether the path is direct or passes through other constants.

Code may use a loaded constant from another unit. An unknown constant may run
the autoloader. A failed load leaves the constant undefined.
