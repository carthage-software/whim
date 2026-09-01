# Your First Program

Create `hello.whim`:

```whim
write_line!('Hello, Whim!');
```

Run it:

```console
whim hello.whim
```

Whim runs statements at file scope. You do not need a `main` function.

## Start a project

A single file is enough for a script. For a project, create an empty directory
and initialize it:

```console
mkdir hello
cd hello
whim init
```

`whim init` creates `whim.toml`, `src/main.whim`, and `tests/main.whim`. It also
starts a Git repository when needed and writes the Git rules used by Whim
projects. Pass `--no-git` to skip Git setup.

Run the generated program:

```console
whim src/main.whim
```

Use `whim init` when you start an application or library. The manifest lets
Whim format the whole project and manage its Git dependencies.

## Variables

A variable starts with `$`. Assignment creates it:

```whim
$language = 'Whim';
$year = 2026;

write_line!($language . ' ' . $year);
```

Whim infers the variable's type from its value. A later assignment may change
that type:

```whim
$value = 10;
$value = 'ten';

assert!($value is string);
```

Use `==` and `!=` for equality. Conditions must produce `bool`:

```whim
$name = 'Ada';
if ($name != '') {
  write_line!('Hello, ' . $name);
}
```

`if ($name)` is an error. Whim does not turn strings, numbers, arrays, or null
into booleans.

## Functions

A function declares each parameter and its return type:

```whim
function square(int $number): int {
  return $number * $number;
}

assert!(square(9) == 81);
```

Whim checks the argument before the call and checks the result before it
returns to the caller.

## Imports

Use a fully qualified name or import a symbol:

```whim
use Whim\Str;

$parts = Str\split('one,two,three', ',');
assert!(length!($parts) == 3);
```

One `use` form imports classes, interfaces, enums, functions, constants,
aliases, and newtypes.

## Format the file

From a project with a `whim.toml`, format every project source:

```console
whim fmt
```

You may instead name one file or directory:

```console
whim fmt hello.whim
whim fmt src
```

Use `--check` when you want a nonzero exit status instead of an edit:

```console
whim fmt --check
```

The next chapter builds a program with files, loops, and a dict.
