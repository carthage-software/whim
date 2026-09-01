# Environment, Processes, and Terminals

## Environment

`Whim\Env` reads the current process state:

- `get_arguments()` returns arguments after the entry file.
- `get_variable()` returns `null` for a missing environment value.
- `get_variables()` returns all current environment values.
- `set_variable()` and `remove_variable()` update the process environment.
- `current_directory()` and `set_current_directory()` handle the working path.
- `home_directory()` and `temporary_directory()` return common paths.
- `current_binary()` returns the Whim executable path.
- `current_script()` returns the entry source path when one exists.

```whim
use Whim\Env;

$arguments = Env\get_arguments();
$home = Env\home_directory();
if ($home != null) {
  write_line!($home);
}
```

Environment state belongs to the whole process. A library should prefer an
explicit parameter when a value can vary by call.

## Child commands

`Command\Command` is a readonly child-process definition. Start with
`Command::create($program)`, then add arguments, environment values, a working
directory, stream choices, or a separate process group through `with...`
methods.

A stream may use a pipe, inherit the parent's stream, discard data, use a file
descriptor, or use a terminal. The command API passes arguments directly; it
does not join them into shell text.

`start()` returns a `Child`. `run()` waits and returns `Output`. `output()`
returns standard output and throws `FailedException` on a bad exit. `succeeds()`
returns a boolean.

A child exposes its process ID, running state, available pipe handles, signals,
and waits. `join()` captures output. `terminate()` sends a graceful signal and
then kills after its optional grace period. `kill()` stops at once.

`ExitStatus` is `Exited|Signalled`. `Output` stores status, standard output, and
standard error.

## Current process

`Process\get_id()` and `get_parent_id()` return process IDs. The identity
functions read or change real, effective, and supplementary user and group
IDs. Session and process-group functions inspect or change POSIX process
relationships. Priority functions read or change a process's scheduling
priority. Resource-limit functions read or change the current process's soft
and hard limits; `null` means unlimited. A missing process argument means the
current process.

Changing identities, groups, sessions, priorities, or resource limits may need
operating-system permission. A failed operation throws `RuntimeException` with
the system error as its cause.

`Process\replace()` replaces Whim with another program and returns `never`.
Its argument list follows the program name. A `null` environment inherits the
current one; a dict replaces it. Failure throws `RuntimeException`.

`exists($pid)` checks a process. `signal` and `signal_group` send a supported
POSIX `Signal`.

`cpu_times()` returns the user and system CPU time consumed by the current
process and its waited-for children. Each value is a `Duration`.

`watch_signal()` calls a function each time a catchable signal arrives. Keep
the returned `SignalWatcher` alive while it is needed, then call `close()`.
Its destructor also stops the watch.

`find_executable($name)` searches the current executable path and returns an
absolute non-empty path or `null`.

Not every listed signal exists on every POSIX system. `isSupported()` checks
the host; `isCatchable()` rejects signals such as forced kill and stop.

## Host information

`OS\information()` identifies the running system and machine. `uptime()`
returns a `Duration`. `load_averages()` reports the one-, five-, and
fifteen-minute load averages. `memory()` reports total and available physical
memory in bytes.

## Users and groups

`OS\find_user()` and `OS\find_group()` query the operating-system account
directory by name or numeric identifier. They return immutable `User` and
`Group` records, or `null` when no record exists.

`OS\groups_for_user()` returns the available primary and supplementary group
records for a user. A group's `memberNames` contains only names explicitly
listed in that group record; users whose primary group matches may be absent.

Account-directory lookups run on the blocking worker pool. They accept an
optional cancellation token and do not block other Whim tasks.

## Shell text

Use `Shell\escape_argument()` to quote one value for the POSIX shell and
`Shell\join()` to quote and join a list. Use them only when the task needs shell
syntax. Prefer `Command` for a normal program call.

## Terminal

`Terminal\attached($descriptor)` checks whether a descriptor is a terminal.
`size($descriptor)` returns columns and rows or `null`. `path($descriptor)`
returns the terminal path or `null`.
