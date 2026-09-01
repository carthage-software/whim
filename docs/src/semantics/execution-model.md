# Compilation and Execution

Whim compiles source to register bytecode, links its declarations, then runs
its file body. The `whim` command performs these steps in one process.

## Program start

For `whim app.whim`, the command:

1. creates the language core;
2. loads the bundled standard-library artifact;
3. reads and compiles `app.whim`;
4. links its declarations;
5. runs its file-scope statements; and
6. runs all remaining destructors.

The core contains the symbols that the language itself needs. The standard
library is a precompiled `.whia` artifact made from the source under
`lib/src/`. The entry file sees both sets of symbols.

The command reads runtime settings from the nearest `whim.toml`. It does not
read `whim.lock` or `vendor/`. A project that uses Git packages must require its
generated loader.

## One source unit

The compiler treats one source file as one unit. It parses the whole unit before
it runs any of it. Declarations in that unit may refer to declarations written
later in the same file.

Whim reports failures by stage:

- `ParserError` for invalid source tokens or syntax;
- `CompilerError` for a rule that one unit breaks;
- `LinkerError` for a conflict between declared symbols or class contracts.

The linker publishes a unit only after its declarations pass. A failed unit
does not replace a symbol that already exists.

## Initializers and the file body

After linking, Whim evaluates namespace constants, class constants, and static
property defaults. It also checks the values of static properties. An
initializer may call code or throw, so a unit can fail before its first
file-scope statement.

Instance property defaults run when code creates an object. Parameter defaults
run when a call omits that argument. Attribute arguments run when code asks
Whim to create the attribute object.

File-scope statements then run from top to bottom. Their locals belong only to
that file body. The body returns `null` when it ends.

## Optimization

The compiler optimizes each unit before it runs. It may remove a type check that
it proves will pass, choose a bytecode instruction for a known value kind, fold
a fixed expression, or inline an eligible call.

Optimization must not change program results. Use `WHIM_OPTIMIZATIONS=off` to
compare a problem with plain bytecode. Use `whim disassemble` to inspect either
form.

## Loaded files

`require!`, `require_once!`, and autoloaders compile more units in the same
process. Each successful unit adds its symbols to the current program. Its
file-scope statements run under the same event loop and use the same heap,
standard handles, environment, and process state.

A loaded unit cannot redeclare an existing symbol. `require_once!` identifies a
file by its resolved path and stops a second run of that path.

## Calls, tasks, and shutdown

Normal calls use one stack of call frames. Async tasks use separate stacks and
cooperate on one event loop. A task runs until it returns, throws, or reaches a
wait that suspends it.

The entry body does not drain the event loop on its own. Call `Async\drain()` or
await a future to run scheduled work. Referenced tasks keep `Async\drain()`
running; unreferenced tasks do not.

After the entry body ends, Whim runs remaining destructors. An uncaught
throwable ends the program with status 255. `exit!` ends it with the requested
status and does not run pending `finally` blocks. Shutdown destructors still
run after `exit!`. `panic!` prints a message and trace, uses status 255, skips
pending `finally` blocks, and runs shutdown destructors.
