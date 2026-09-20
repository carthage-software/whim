# whim-sys

Files, sockets, processes, and OS services used by Whim.

This crate owns OS resources and exposes Rust values and errors. It has no dependency on the Whim runtime, heap, or bytecode. Unix and Windows code lives in private modules behind the same API.

`Descriptor` owns a file, pipe, socket, or signal subscription. `Readiness` tells the caller whether to register a descriptor with `whim-loop` or poll after a delay. The caller decides when to suspend or resume a task.

File and account calls are synchronous. The runtime submits them to its worker pool and uses `Operation` to receive completion or cancellation. Cancelling an operation discards its result; it does not stop an OS call that has already started.

`Processes` owns child-process handles and tracks child CPU time where the OS requires it. Unsupported operations return `Error::Unsupported` before performing the operation. The runtime maps this to `Whim\Unwind\UnsupportedPlatformException`.

Run the crate's tests with `cargo test -p whim-sys`. They exercise OS resources without creating a Whim engine.
