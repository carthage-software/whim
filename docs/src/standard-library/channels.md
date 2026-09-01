# Channels and Cancellation

A channel sends typed values between tasks. `Channel\bounded()` creates a
fixed-size buffer. `Channel\unbounded()` creates a buffer with no fixed limit.
Both return a sender and a receiver.

```whim,norun
use Whim\Async;
use Whim\Channel;

($sender, $receiver) = Channel\bounded::<string>(4);

$producer = Async\spawn::<null>(fn(): null {
  $sender->send('one');
  $sender->send('two');
  $sender->close();
  return null;
});

assert!($receiver->receive() == 'one');
assert!($receiver->receive() == 'two');
$producer->await();
```

## Sending

`Sender<T>` provides:

- `send($value, $cancellation)` waits for buffer space.
- `trySend($value)` returns at once and throws `FullException` when full.
- `waitUntilSendable($cancellation)` waits for space without sending a value.

Sending through a closed channel throws `ClosedException`.

## Receiving

`Receiver<T>` provides:

- `receive($cancellation): T` waits for a value.
- `tryReceive(): T` returns at once and throws `EmptyException` when empty.
- `waitUntilReceivable($cancellation)` waits for a value without taking it.

A receiver can drain values buffered before the channel closed. Once the closed
channel is empty, receive operations throw `ClosedException`.

The API throws instead of using `null` to mean "no value." A channel may carry
`null` when `T` permits it.

## Channel state

The sender and receiver share these operations:

- `getCapacity(): null|NonNegativeInt` returns `null` for an unbounded channel.
- `count(): NonNegativeInt` returns the buffered item count.
- `isFull()` and `isEmpty()` inspect the buffer.
- `close()` ends further writes.
- `isClosed()` reports whether the channel has closed.

Closing an already closed channel has no further effect.

## Cancellation tokens

Long waits accept `null|CancellationToken`. A wait with `null` does not observe
a token.

A cancellation token provides four operations:

```text
interface CancellationToken {
  public function isCancellationRequested(): bool;
  public function throwIfCancellationRequested(): void;
  public function register(fn(): void $callback): int;
  public function unregister(int $id): void;
}
```

Code that starts a cancellable wait should first call
`throwIfCancellationRequested()`, then register a callback, and unregister it
in `finally`. Most users should pass a token to library operations instead of
managing registrations themselves.

## Signal cancellation

`SignalCancellationToken` starts uncancelled. Calling `cancel()` marks it
cancelled, runs each registered callback, and makes later checks throw
`CancelledException`. A second `cancel()` has no effect. The optional cause is
stored under the cancellation error.

If one callback throws, `cancel()` rethrows it. If several callbacks throw, it
throws `CompositeException`.

```whim,norun
use Whim\Async\SignalCancellationToken;

$token = new SignalCancellationToken();
$token->cancel();
assert!($token->isCancellationRequested());
```

## Linked cancellation

`LinkedCancellationToken` accepts one or more source tokens. It cancels when
any source cancels. Destroying it removes its source registrations.

## Timeouts

`TimeoutCancellationToken` cancels after a `Time\Duration`. It may also take a
parent token. It cancels when either the timer ends or the parent cancels.

The timer arms only while code has registered a callback. Direct calls to
`isCancellationRequested()` and `throwIfCancellationRequested()` still check
the elapsed time.

Timeout cancellation throws `CancelledException` whose cause is a
`TimeoutException`. Parent cancellation has no timeout cause.

Cancellation is cooperative. It wakes operations that accept the token. It
does not stop arbitrary code or undo work that already finished.
