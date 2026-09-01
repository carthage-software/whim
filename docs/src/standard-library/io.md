# Files and I/O

`Whim\IO` uses small capability interfaces. A function asks only for the work
it needs.

## Handle contracts

- `Handle` marks an I/O value.
- `ReadHandle` reads bytes and waits for readable data.
- `WriteHandle` writes bytes and waits for writable space.
- `BufferedReadHandle` adds byte, line, and delimiter reads.
- `BufferedWriteHandle` adds `flush`.
- `SeekHandle` moves a cursor.
- `CloseHandle` reports and closes owned state.
- `FileDescriptorHandle` exposes an owned `OS\FileDescriptor`.

`CloseHandle::__destruct()` attempts a final close and discards its error. Use
`using` when close time or errors matter.

## Reading

`tryRead($maxBytes)` never waits. It returns bytes now ready and may return an
empty string. `waitUntilReadable($cancellation)` suspends until a read may make
progress. `reachedEndOfDataSource()` tells an empty read from the end.

`read` combines readiness and non-blocking reads. `readAll` reads through the
end with an optional byte limit. `readFixedSize` requires an exact count.

`Reader` adds buffering around any `ReadHandle`. It can read one byte, one line,
through a suffix, or through a suffix with a bound. Its private buffer preserves
bytes read past a delimiter.

## Writing

`tryWrite($bytes)` writes without waiting and returns the count. `write` waits
as needed and may write part of the input. `writeAll` continues until it sends
all bytes.

`IO\copy` moves all bytes from a read handle to a write handle. `copy_chunked`
lets the caller choose a chunk size and limit. `pipe` runs a read-to-write copy
as a task.

## In-memory and adapted handles

`MemoryHandle` is readable, writable, seekable, closeable, and convertible to a
string. Reads and writes share one cursor.

Other adapters include:

- `BoundedReadHandle` fails after a read limit.
- `FixedLengthReadHandle` exposes an exact length.
- `TruncatedReadHandle` stops after a maximum length.
- `ConcatReadHandle` reads several sources in order.
- `JoinedReadWriteHandle` joins separate read and write sides.
- `TeeWriteHandle` copies writes to several targets.
- sink handles discard writes or expose end-of-input reads.
- `SpoolHandle` keeps small data in memory and moves larger data to a temporary
  file while preserving one seekable handle.

Adapters do not claim an operating-system descriptor unless their own contract
implements `FileDescriptorHandle`.

## Standard handles

`IO\input_handle()`, `write_handle()`, and `error_handle()` return the process
standard input, output, and error handles. They are descriptor-backed. The
language write constructs use the same output channels.

## Files

`File\open_read_only`, `open_write_only`, and `open_read_write` return typed
file handles. `WriteMode` selects open-or-create, truncate, append, or
must-create behavior.

```whim,norun
use Whim\File;
use Whim\IO;

using ($source = File\open_read_only('input.txt')) {
  using ($target = File\open_write_only('output.txt')) {
    IO\copy($source, $target);
  }
}
```

`File\read` and `File\write` cover one-shot work. File handles expose their
path and size, support seeking, and can take shared or exclusive locks.
Writable file handles can change the file length with `truncate` and flush
pending data and metadata with `synchronize`. Both operations run on the
blocking worker pool and accept a cancellation token.

## File system

`Whim\Filesystem` creates files, directories, hard links, symbolic links,
named pipes, and temporary entries. It deletes, renames, copies, changes modes
and owners, reads directories, reads links, and resolves canonical paths.

Inspection covers existence, node kind, access bits, metadata, and available or
total disk space. `metadata` follows a symbolic link;
`symbolic_link_metadata` inspects the link itself.

Callers must pass `true` to delete a directory tree. Permission values use
octal literals such as `0o755`.

`Filesystem\exchange_creation_mask()` replaces the process-wide file creation
mask and returns its previous value. Set it during startup. A temporary change
can race with file creation in another task or blocking worker.

`Whim\Path\SEPARATOR` is the POSIX path separator. Path strings use `/`.

## File descriptors

`OS\FileDescriptor` owns one POSIX descriptor. `duplicate($number)` creates a
new owned descriptor from an open number. `toInt()` returns its number.
`isClosed()` and `close()` manage its lifetime.

Duplicating is not the same as borrowing an integer. The new object owns its
descriptor and closes it.
