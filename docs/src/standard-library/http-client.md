# HTTP Client

The HTTP client sends `HTTP\Message\Request` values and returns a `Transaction`.
It supports HTTP/1.1 and HTTP/2, connection reuse, TLS, proxies, middleware,
redirects, retries, cookies, and cancellation.

## Basic use

```whim,norun
use Whim\HTTP\Client\DefaultClient;
use Whim\HTTP\Message\Request;
use Whim\URL\URL;

$client = new DefaultClient();
$request = Request::fromURL('GET', URL::from('https://example.com/'));
$transaction = $client->send($request);
$body = $transaction->response->body?->readAll();
```

`Client::send()` takes a request, per-send settings, and optional cancellation.
The request needs an absolute URL or a base URL in the client settings.

The returned body is a read handle. Read or close it before expecting a pooled
HTTP/1.1 connection to return to the pool.

## DefaultClient

`DefaultClient` accepts a connector, a `Configuration`, and an iterable of
middleware. The default connector pools direct TCP and TLS connections.

Configuration sets header and body size limits, informational response limits,
a base URL, TLS settings, HTTP/2 settings, enabled protocol versions, and an
optional proxy.

`SendConfiguration` overrides one call. It can also set callbacks for
informational responses and connection metadata, plus a connection timeout.

The client rejects trailers without a body, `CONNECT` without a tunnel API,
and a `TRACE` request with a body.

## Connections and connectors

A `Connector` receives an origin, request, settings, and cancellation token. It
returns a `Connection`. A connection reports protocol and endpoint metadata,
whether the pool may reuse it, and an `exchange` operation.

`DirectConnector` opens one network connection. `PooledConnector` reuses safe
connections by origin. `UnixConnector` sends HTTP over a Unix socket.

`ProxyConfiguration` supports an HTTP proxy URL, optional authorization, TLS to
the proxy, a server-name override, and host bypass rules.

## Middleware

Client middleware runs after the connector acquires a connection and before
the protocol exchange. It receives the connection, request, effective
settings, next handler, and cancellation token.

`CookieJar` stores accepted response cookies and adds matching cookies to later
requests. It follows domain, path, secure, expiry, and same-site data available
to the client. `clear()` removes stored cookies and `count()` reports them.

`DeniedDestinationsMiddleware` rejects configured IP blocks after resolution.
`publicOnly()` blocks private, loopback, link-local, and other non-public
destinations. Use it when a caller controls the target URL.

## Redirects and retries

`RedirectClient` wraps any client. It follows 301, 302, 303, 307, and 308 with a
fixed limit. It strips credentials on cross-origin moves, follows safe referrer
rules, and rewinds a seekable request body when a redirect must replay it. A
body that cannot rewind stops the redirect.

`RetryClient` retries idempotent methods after connection or transport errors.
It uses bounded attempts and increasing delays. A request body must be seekable
to replay.

Redirect and retry are client decorators, not connection middleware, because
they may need another connection.

## WebSocket client

`HTTP\WebSocket\Client\connect()` performs a WebSocket handshake for a URL and
returns a client connection. Configuration sets frame and message limits,
headers, origin, subprotocols, response header size, and TLS settings.

The connection adds URL, response fields, endpoints, and TLS state to the common
WebSocket connection API described in the server chapter.
