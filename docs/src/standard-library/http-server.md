# HTTP Server

The server accepts HTTP/1.1 and HTTP/2 streams, parses requests, calls one
handler, and writes responses. Its public layer stays in Whim and works with
the network and I/O interfaces.

## Handler

Every request reaches:

```text
interface Handler {
  public function handle(
    Context $context,
    Request $request,
    CancellationToken $cancellation,
  ): Response;
}
```

`FunctionHandler` accepts a callable with any useful subset of context,
request, and cancellation parameters, or no parameters. It adapts that callable
once in its constructor.

```whim,norun
use Whim\HTTP\Server\Handler\FunctionHandler;

$handler = new FunctionHandler(
  fn(): Whim\HTTP\Message\Response =>
    Whim\HTTP\Message\Response\text('hello'),
);
```

## Context

Every context has protocol, local endpoint, peer endpoint, and a mutable dict of
string parameters. It can hold one session and register callbacks that run when
the response completes.

Some contexts add a capability:

- `Informational` can send a 1xx response.
- `Push` can start an HTTP/2 server push.
- `Secure` exposes TLS security state.
- `Upgrade` can hand the stream to another protocol.

Check the interface with `is` before using an optional capability.

## Bindings and server life

`Binding\Stream` wraps a network listener and an `enableH2c` choice. A TLS
listener negotiates HTTP/2 through ALPN; a plain listener can opt into cleartext
HTTP/2.

`Server` takes one or more bindings and a configuration. `serve($handler,
$cancellation)` runs until cancellation, closed bindings, or a fatal accept or
connection error. One server object can serve only once.

Shutdown stops accepts, lets active requests drain up to the shutdown timeout,
then closes remaining connections.

Configuration bounds idle time, header and body time, response writes,
connections, connections per peer, concurrent requests, server pushes, header
and body sizes, requests per connection, HTTP/2 settings, middleware, and the
error responder.

## Middleware

Server middleware receives context, request, next handler, and cancellation. It
may change the request, call the next handler, change the response, or return
without calling next.

`Middleware\wrap($handler, $layers)` creates a handler chain. The server's
configuration can hold the same middleware list.

Built-in middleware includes:

- `CORS` for origin, method, field, credential, and cache rules.
- `HandlerTimeout` for an optional per-handler deadline.
- `RequestDecompression` for registered content codings and size bounds.
- `ResponseCompression` for `Accept-Encoding` negotiation.
- `Session` for cookie-backed stored sessions.
- `FunctionMiddleware` for a callable adapter.

Handler timeouts are not part of the base server. Add the middleware when the
application wants that policy.

## Errors and responders

`HTTPException` carries a 4xx or 5xx status for expected request failure. Other
throwables become server errors. The configured `Responder` turns a status and
optional cause into a response.

`Bare` returns an empty error response. `Debug` includes the throwable and full
trace text and is suitable only for local work. A responder failure or an
exception from an upgrade callback stops the server and remains visible.

## Router

`HTTP\Router\Router` implements both `Handler` and `RouteCollection`. `add`
registers one method, path pattern, and handler. Helpers cover GET, HEAD, POST,
PUT, PATCH, DELETE, and OPTIONS.

Patterns support:

- literal text;
- `{name}` for one non-empty path segment;
- `{name:expression}` for a segment checked by a byte regular expression;
- `[text]` for an optional sequence;
- a final `*` for the rest of the path, stored under parameter name `*`;
- `\*` for a literal asterisk.

A match percent-decodes each captured value and appends it to
`Context::$parameters` before it calls the route handler.
`Router\parameter($context, $name)` returns one value or `null`.

`prefix($path, $group)` adds a prefix to every route registered in the group.
`through($middleware, $group)` wraps only that group. The router implements
`ToIterator` over `(method, pattern, handler)` registrations.

No path match throws `NotFoundException`. A path with no matching method throws
`MethodNotAllowedException` and includes the allowed methods.

## Static files

`Handler\StaticFiles` serves a root directory under a URL prefix. It supports
an index file, fixed response fields, cache control, media sniffing, ranges,
conditional requests, and safe path resolution. It rejects traversal outside
the root.

The file body stays an I/O handle. The handler does not read the full file into
memory.

## WebSocket server

`HTTP\WebSocket\Server\Handler` upgrades an HTTP/1.1 route and gives a
`Connection` to a `ConnectionHandler`. `FunctionConnectionHandler` adapts
callables with useful subsets of its arguments.

A connection can send text, binary, ping, and close frames. `receive()` accepts
a per-call cancellation token. `tryReceive()` returns `null` when no complete
message is ready; `waitUntilReceivable()` waits without taking one.

Received messages are `TextMessage`, `BinaryMessage`, or `CloseMessage`. The
connection reports its subprotocol and close code and reason. Configuration
bounds frames and complete messages and lists accepted subprotocols.

Destroying a live connection attempts a safe close without letting a transport
error escape the destructor.
