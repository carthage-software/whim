# Aliases and Newtypes

Type aliases and newtypes both name a backing type. An alias is transparent. A
newtype adds a runtime tag.

## Type aliases

Declare an alias with `type`:

```whim
type UserName = string&!'';
type Coordinate = (float, float);

function greet(UserName $name): string {
  return 'Hello, ' . $name;
}
```

`UserName` and `string&!''` are the same type. A value gains no tag when it
passes through the alias.

Aliases may refer to aliases declared later. They may also be recursive when
the recursion passes through a collection or other value structure.

```whim
type Json = null|bool|int|float|string|vec<Json>|dict<string, Json>;
type Tree<T> = T|(Tree<T>, Tree<T>);
```

Whim checks recursive values without running forever on cycles.

## Generic aliases

An alias may have parameters, bounds, defaults, and variance.

```whim
type Pair<A, B> = (A, B);
type Source<out T> = fn(): T;
type NonNull<T: !null = int> = T;
```

The alias expands with its supplied arguments. Runtime diagnostics may keep the
alias name when that name makes the failed contract clearer.

An alias cannot name bare `void`.

## Newtypes

Declare a newtype with `newtype`:

```whim
newtype UserId = int;
newtype PostId = int;
```

Construct it by calling its name:

```whim
newtype UserId = int;
newtype PostId = int;

$user = UserId(7);
$post = PostId(7);
```

The constructor checks the backing type and adds the tag. `UserId(7)` fits
`UserId` and `int`. Plain `7` fits `int` but not `UserId`. `PostId(7)` does not
fit `UserId`.

```whim
newtype UserId = int;

function load(UserId $id): void {}

load(UserId(7));
```

Newtypes use their backing value directly for normal operations:

```whim
newtype Count = int;

$total = Count(2) + Count(3);
assert!($total == 5);
```

Member access, indexing, calls, iteration, arithmetic, comparison, and string
joining act on the backing value when that operation supports it.

## Casting newtypes

`$value as Newtype` checks the backing type and applies the newtype tag.
`?as` returns the tagged value or `null`.

```whim
newtype UserId = int;

$id = 7 as UserId;
$missing = 'seven' ?as UserId;

assert!($id is UserId);
assert!($missing == null);
```

Casting a newtype to its backing type succeeds. Casting between two newtypes
checks the new target's backing type and applies the target tag.

## Generic newtypes

Newtypes may have parameters, bounds, defaults, and variance.

```whim
newtype Identifier<T: int|string = int> = T;
newtype Producer<out T> = fn(): T;

$number = Identifier(42);
$name = Identifier::<string>('user');
```

The constructor uses `::<...>` when you supply type arguments.

## Layered newtypes

A newtype may back another newtype:

```whim
newtype Inner = int;
newtype Outer = Inner;

$value = Outer(Inner(5));
assert!($value is Outer);
assert!($value is Inner);
assert!($value is int);
```

Each tag remains part of the value's type.

## Mutable backing values

A newtype check always includes the current backing value. Mutation can make a
tagged collection stop fitting its declared newtype.

```whim
newtype Numbers = vec<int>;

$numbers = Numbers(vec[1]);
$numbers[] = 'changed';

assert!(!($numbers is Numbers));
```

Whim does not freeze the backing vec. It checks the type again when code crosses
a typed boundary.

Dict keys are different. A dict stores normalized scalar keys, so a newtype tag
on an integer, string, or boolean key does not remain in the dict. Do not use a
newtype as a dict key type when the returned dict must still satisfy that tag.

## Attributes

Type aliases and newtypes may carry attributes that target their symbol kinds.
`Whim\Attribute\Attribute` defines separate target bits for aliases and
newtypes.

## Choosing one

Use an alias when two spellings should accept the same values. Use a newtype
when code must not mix two values only because their backing types match.

```whim
type Port = 1..=65535;
newtype UserId = int;
```

`Port` refines integers without adding identity. `UserId` marks which domain an
integer belongs to.
