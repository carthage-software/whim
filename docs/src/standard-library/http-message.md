# HTTP Messages and Cookies

`Whim\HTTP\Message` holds transport-free HTTP values. Bodies use I/O handles,
so a message need not hold all bytes in memory.

## Fields

`FieldMap` is a readonly ordered list of `(name, value)` fields plus an index for
case-insensitive lookup. `from()` validates field names and values.

- `get($name)` returns the first value or `null`.
- `getAll($name)` returns every value in order.
- `has($name)` checks a name.
- `with` replaces all values under a name.
- `withAdded` appends one value.
- `without` removes a name.
- `toVec`, `count`, `isEmpty`, and `toIterator` inspect the map.

`fromPartsUnsafe` is for already parsed and indexed internal data. Application
code should use `from`.

## Request and response

`Request` stores method, request target, optional absolute URL, protocol
version, fields, optional body, and optional future trailers. Build one with
`fromParts` or `fromURL`. `fromPartsUnsafe` skips method and target checks.

`Response` stores status, protocol version, fields, optional body, and optional
future trailers. Its constructor checks the refined status type at the call.

Both are readonly. `withMethod`, `withStatus`, `withHeader`, `withBody`, and
other `with...` methods return changed copies.

`ProtocolVersion` names HTTP/1.0, HTTP/1.1, HTTP/2, and HTTP/3 message values.
The included client and server support HTTP/1.1 and HTTP/2; the HTTP/3 enum case
does not add an HTTP/3 transport.

`Transaction` joins informational responses with the final response. `Exchange`
joins one request and one response. `reason_phrase($status)` returns a standard
phrase when one exists.

## Response helpers

`Whim\HTTP\Message\Response` provides:

- `json($value)` with `application/json`
- `text($value)` with UTF-8 plain text
- `html($value)` with UTF-8 HTML
- `empty($status)` with no body
- `redirect`, `see_other`, `temporary_redirect`, and `permanent_redirect`

Text and HTML accept a string or `Convert\ToString`. JSON accepts `Json\Value`
or `Json\ToJson`. Redirects accept a string, URI, or URL.

## Request cookies

`Cookie\Collection` parses `Cookie` request fields into an ordered list of
name-value pairs. Duplicate names stay available through `getAll`; `get`
returns the first.

The collection implements iteration and `toString`. `fromHeaders` reads all
cookie fields from a `FieldMap`.

## Set-Cookie

`Cookie\SetCookie` stores name, value, expiry, max age, domain, path, secure,
HTTP-only, and same-site settings. `fromParts` validates each part; `parse`
returns `null` for a bad field. `with...` methods return changed copies.

`Cookie\add($response, $cookie)` appends one `Set-Cookie` field to a response.
It does not replace other cookies.

`SameSite` has `Strict`, `Lax`, and `None`. A `SameSite::None` cookie should also
be secure for current browsers.

## Sessions

`HTTP\Session\Session` stores `Json\Value` entries under non-empty names. It
supports `contains`, throwing `get`, `set`, `remove`, iteration, `toDict`, and
`clear`. It can request a new identifier or destruction.

`Session\Configuration` sets the cookie template, idle timeout, optional cookie
lifetime, and rolling expiry. The default cookie is `__session`, secure,
HTTP-only, path `/`, and same-site lax. Pass a cookie with `secure: false` for
plain HTTP local work.

`MemoryStore` keeps session records in one process. `DatabaseStore` uses the
database contracts. Store writes use a revision check; a conflict throws
`ConflictException` rather than dropping one concurrent update.

The server session middleware loads a record, attaches the session to the
request context, and writes or deletes it after the handler. The context gives
the handler `getSession()`.
