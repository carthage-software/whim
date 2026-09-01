# Statements and Loops

Statements run actions and choose control flow.

## Expression statements

Any expression may form a statement when followed by `;`:

```whim
$value = 1;
$value++;
write_line!($value);
```

Discarding a value marked `#[MustUse]` raises an error. Use its result or pass
it to `discard!` to state that you meant to ignore it.

An empty statement is one semicolon. It does nothing.

## Final locals

`final` binds a local once:

```whim
final $rate = 0.1;
final $price = 250.0;

assert!($price * $rate == 25.0);
```

The declaration must be the local's first assignment. Any later write in the
same function or file body is a compile error. A final local still follows
normal value rules: an object may change through the local, but the local
cannot point to another object.

## `if`

An `if` condition must be bool:

```whim
$value = -3;
if ($value < 0) {
  $sign = 'negative';
} else if ($value == 0) {
  $sign = 'zero';
} else {
  $sign = 'positive';
}

assert!($sign == 'negative');
```

## `while` and `do ... while`

`while` tests before each pass. `do ... while` runs its body once before the
first test:

```whim
$count = 0;
while ($count < 3) {
  $count++;
}

do {
  $count--;
} while ($count > 0);

assert!($count == 0);
```

## `for`

A `for` loop keeps its setup, condition, and step in one header:

```whim
$sum = 0;
for ($number = 1; $number <= 5; $number++) {
  $sum += $number;
}

assert!($sum == 15);
```

Each header section may hold a comma-separated expression list. An empty
condition acts as `true`:

```whim
$left = 0;
$right = 3;
for (; $left < $right; $left++, $right--) {
  write_line!($left . ':' . $right);
}
```

## `foreach`

`foreach` accepts an array, an `Iterator<K, V>`, or a `ToIterator<K, V>`:

```whim
$seen = vec[];
foreach (dict['a' => 1, 'b' => 2] as $key => $value) {
  $seen[] = $key . $value;
}

assert!($seen == vec['a1', 'b2']);
```

Omit the key when it is not needed. The value target may be a destructuring
pattern.

Array iteration uses the entries present when the loop begins. Changing the
source does not change that walk.

## Variable scope

Each file body, function, method, closure, and short closure has its own
variable scope.
Control-flow blocks do not create another variable scope. A variable assigned
on only some paths remains undefined when execution took another path.

Functions cannot read file-scope variables. A closure must capture outer
variables, while a short closure captures the outer variables it uses.
