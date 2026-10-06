# Server-Sent Events

`Whim\HTTP\SSE` encodes server-sent events and supplies streaming HTTP response
bodies. It ships with stdlib and needs no package or autoloader.

## Events

An `Event` carries UTF-8 text, an optional name, an optional ID, and an optional
reconnect delay:

```whim
use Whim\HTTP\SSE\Event;
use Whim\Time\Duration;

$event = new Event(
  '{"temperature": 21}',
  name: 'reading',
  id: '42',
  retry: Duration::fromSeconds(3),
);
assert!(
  $event->encode()
  == "id: 42\nevent: reading\nretry: 3000\ndata: "
  . '{"temperature": 21}'
  . "\n\n",
);
```

`encode()` writes a `data:` field for each payload line and ends the event with
a blank line. It converts CR and CRLF line endings to LF. Empty data still
writes a field, so the browser delivers the event.

Null omits a name or ID. An empty name selects the default `message` event;
an empty ID clears the browser's resume position. Names and IDs cannot contain
line breaks, and IDs cannot contain NUL. All text must be valid UTF-8. Invalid
input throws `Whim\Unwind\InvalidArgumentException`.

Retry durations must be non-negative. Encoding drops fractions of a
millisecond and handles `Duration::max()` without integer overflow.

`Comment::of($text)` encodes a comment and replaces line breaks with spaces.
Browsers ignore comments; a server can use them as heartbeats.

## Sources and response bodies

Implement `Whim\HTTP\SSE\Source\Source` to supply events:

```whim
use Whim\Async\CancellationToken;
use Whim\HTTP\SSE;
use Whim\HTTP\SSE\Event;
use Whim\HTTP\SSE\Source\Source;

final class Greetings implements Source {
  private bool $finished = false;

  public function poll(null|CancellationToken $cancellation = null): vec<Event> {
    $cancellation?->throwIfCancellationRequested();
    $this->finished = true;
    return vec[new Event('hello', name: 'greeting', id: '1')];
  }

  public function isFinished(): bool {
    return $this->finished;
  }
}

$response = SSE\respond(new Greetings());
assert!($response->status == 200);
assert!($response->body?->readAll() == "id: 1\nevent: greeting\ndata: hello\n\n");
```

`respond($source, $headers = FieldMap::default())` returns status 200 with
`Content-Type: text/event-stream; charset=utf-8`, `Cache-Control: no-store`,
and `X-Accel-Buffering: no`. It keeps other caller headers and removes
`Content-Length` and `Transfer-Encoding`, letting the server choose framing.
The HTTP/1.1 server streams chunks; the HTTP/2 server streams DATA frames.

`Body` implements `Whim\IO\ReadHandle` and `Whim\IO\CloseHandle`. A read polls
only after it drains buffered events. An empty batch produces `Comment::WAITING`
instead of ending the stream. The body ends after the source finishes and its
buffer drains. An already finished source receives no polls.

For a long-lived source, `poll()` must wait for events or a heartbeat interval
and honor its cancellation token. Returning empty batches at once in a loop
would consume CPU. `Source\FunctionSource` wraps a polling callable with the
same signature; it never reports itself finished.

The body checks cancellation before and after each poll. Source errors reach
the reader unchanged. Closing the body discards buffered bytes and closes a
source that implements `CloseHandle`. A suspended poll cannot refill a body
after another task closes it. Use `using` when reading a body directly. If a
handler keeps a body or source, register `Body::close()` through
`Context::onResponseComplete()` to release it when the response ends.

## Reconnection and completion

`SSE\last_event_id($request)` reads `Last-Event-ID`, returning null when the
field is absent or empty. The application decides how to resume from that ID;
the library stores no event history.

`SSE\refuse($event, $headers = FieldMap::default())` sends one event with status
200 and ends the response. This alone does not stop reconnection. The client
must call `EventSource.close()` after an application-defined terminal event:

```javascript
const events = new EventSource('/events');
events.addEventListener('done', () => events.close());
events.addEventListener('failed', () => events.close());
```

A successful stream ending normally makes `EventSource` reconnect. To tell a
browser to stop without sending an event, return an HTTP 204 response. These
rules follow the [SSE specification](https://html.spec.whatwg.org/multipage/server-sent-events.html).

See [HTTP Server](http-server.md) for handlers and response completion,
and [HTTP Messages and Cookies](http-message.md) for response values.
