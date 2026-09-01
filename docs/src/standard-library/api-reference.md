# Namespace Index

This index lists the public standard-library namespaces. Names that end in
`\_Private` are not public and do not appear here.

## Core values and contracts

| Namespace | Purpose |
| --- | --- |
| `Whim` | the `VERSION` constant |
| `Whim\Attribute` | define attribute classes and targets |
| `Whim\Autoload` | register and run symbol autoloaders |
| `Whim\Comparison` | equality, order, and `Ordering` |
| `Whim\Convert` | explicit value conversion contracts |
| `Whim\Default` | the default-value contract |
| `Whim\Enum` | interfaces implemented by all enums |
| `Whim\GC` | explicit cycle collection |
| `Whim\Marker` | built-in attributes and compiler markers |
| `Whim\Option` | `Some`, `None`, and option helpers |
| `Whim\Promise` | the read-only async result contract |
| `Whim\Reference` | weak references and weak maps |
| `Whim\Refine` | common aliases, ranges, and callable types |
| `Whim\Reflection` | read-only access to loaded declarations, types, and values |
| `Whim\Reflection\Attribute` | attribute rules and target kinds |
| `Whim\Reflection\Callable` | functions, methods, closures, captures, and bound arguments |
| `Whim\Reflection\Generic` | type parameters, bindings, and type environments |
| `Whim\Reflection\Member` | methods, properties, constants, and enum cases |
| `Whim\Reflection\Symbol` | classes, interfaces, enums, aliases, newtypes, functions, and constants |
| `Whim\Reflection\Type` | type forms and their parts |
| `Whim\Result` | `Ok`, `Err`, and throwable capture |
| `Whim\Symbol` | symbol lookup and symbol kinds |
| `Whim\Type` | engine-local type identifiers |
| `Whim\Unwind` | errors, exceptions, throwables, and trace frames |

See [Core Types and Functions](core.md), [Reflection](reflection.md), [Option and
Result](../core-library/errors.md), and [Built-in
Attributes](../core-library/attributes.md).

## Strings, numbers, and collections

| Namespace | Purpose |
| --- | --- |
| `Whim\Binary` | fixed-width integer and float encoding |
| `Whim\Dict` | eager keyed collection functions |
| `Whim\Float` | float parsing, bit forms, and checks |
| `Whim\Int` | integer parsing |
| `Whim\Iterate` | iterators and lazy collection functions |
| `Whim\Math` | arithmetic, statistics, bases, and math constants |
| `Whim\Range` | runtime integer range objects |
| `Whim\Str` | byte-string search and changes |
| `Whim\Unicode` | Unicode case folding and code-point checks |
| `Whim\Vec` | eager list functions |

See [Strings, Numbers, and Binary Data](data.md), [Collections and Data
Structures](collections.md), and [Iterators](../core-library/iteration.md).

## Data structures and formats

| Namespace | Purpose |
| --- | --- |
| `Whim\BSON` | BSON values, encoding, decoding, readers, and writers |
| `Whim\CSV` | streaming CSV readers and writers |
| `Whim\Compression` | gzip, deflate, Brotli, and Zstandard streams |
| `Whim\DataStructure` | queue, stack, deque, heap, and priority queue |
| `Whim\Encoding` | shared encoding errors and contracts |
| `Whim\Encoding\Base32` | Base32 text |
| `Whim\Encoding\Base64` | standard and URL-safe Base64 |
| `Whim\Encoding\EncodedWord` | mail header encoded words |
| `Whim\Encoding\Hex` | hexadecimal text |
| `Whim\Encoding\Punycode` | Punycode labels |
| `Whim\Encoding\QuotedPrintable` | quoted-printable text and bytes |
| `Whim\Encoding\URI` | whole-URI percent encoding |
| `Whim\Encoding\UTF8` | UTF-8 checks and lossy repair |
| `Whim\Encoding\Url` | percent and form encoding |
| `Whim\HTML` | WHATWG character references and escaping |
| `Whim\Json` | JSON values, encoding, and decoding |
| `Whim\MIME` | media types, fields, content IDs, and parts |
| `Whim\MIME\MultiPart` | multipart writing and parsing |
| `Whim\MIME\Part` | text, data, and raw MIME parts |
| `Whim\MIME\Sniff` | media-type checks from byte prefixes |
| `Whim\Regex` | byte regular expressions |
| `Whim\UUID` | UUID parsing plus versions 4 and 7 |

See [Encoding and Data Formats](formats.md).

## Time, environment, and the operating system

| Namespace | Purpose |
| --- | --- |
| `Whim\Command` | child-process setup and control |
| `Whim\DateTime` | dates, civil times, zones, and formatting |
| `Whim\Env` | arguments, paths, and environment variables |
| `Whim\OS` | owned file descriptors, accounts, and host metrics |
| `Whim\Path` | POSIX path constants |
| `Whim\Process` | process identity, CPU time, signals, and executable lookup |
| `Whim\Shell` | POSIX shell quoting |
| `Whim\Terminal` | terminal checks, paths, and size |
| `Whim\Time` | durations, monotonic instants, and wall time |

See [Time and Calendars](time.md) and [Environment, Processes, and
Terminals](../core-library/env.md).

## Files and I/O

| Namespace | Purpose |
| --- | --- |
| `Whim\File` | typed file handles and one-shot file work |
| `Whim\Filesystem` | paths, directories, links, metadata, and disk space |
| `Whim\IO` | handle contracts, buffering, adapters, and copying |

See [Files and I/O](io.md).

## Async work

| Namespace | Purpose |
| --- | --- |
| `Whim\Async` | tasks, futures, cancellation, groups, and limits |
| `Whim\Channel` | bounded and unbounded task channels |

See [Tasks and Futures](async.md) and [Channels and Cancellation](channels.md).

## Network and addresses

| Namespace | Purpose |
| --- | --- |
| `Whim\CIDR` | IP network blocks |
| `Whim\IDNA` | international domain-name conversion |
| `Whim\IP` | IPv4 and IPv6 values |
| `Whim\IRI` | international resource identifiers |
| `Whim\Network` | endpoint, stream, listener, and connector contracts |
| `Whim\SOCKS` | SOCKS5 proxy connections |
| `Whim\TCP` | TCP streams, listeners, and connectors |
| `Whim\TLS` | TLS settings, identities, streams, and listeners |
| `Whim\UDP` | datagram sockets |
| `Whim\URI` | generic URI values and reference resolution |
| `Whim\URL` | absolute network URLs and origins |
| `Whim\Unix` | Unix-domain streams, listeners, and pairs |

See [Network, TLS, and Proxies](network.md).

## HTTP

| Namespace | Purpose |
| --- | --- |
| `Whim\HTTP\Client` | HTTP/1.1 and HTTP/2 clients and connectors |
| `Whim\HTTP\Client\Middleware` | client cookie state and request policy |
| `Whim\HTTP\Cookie` | request cookies and `Set-Cookie` values |
| `Whim\HTTP\Message` | methods, fields, requests, responses, and exchanges |
| `Whim\HTTP\Message\Response` | common response factories |
| `Whim\HTTP\Router` | route registration and dispatch |
| `Whim\HTTP\Server` | server setup, settings, and errors |
| `Whim\HTTP\Server\Binding` | stream-listener server bindings |
| `Whim\HTTP\Server\Context` | request context and optional capabilities |
| `Whim\HTTP\Server\Handler` | function, static-file, and handler contracts |
| `Whim\HTTP\Server\Middleware` | CORS, sessions, compression, and timeouts |
| `Whim\HTTP\Server\Responder` | bare and debug error responses |
| `Whim\HTTP\Session` | session values and settings |
| `Whim\HTTP\Session\Storage` | memory and database session stores |
| `Whim\HTTP\WebSocket` | WebSocket messages and connections |
| `Whim\HTTP\WebSocket\Client` | client handshakes and connections |
| `Whim\HTTP\WebSocket\Server` | server upgrades and connection handlers |

See [HTTP Messages and Cookies](http-message.md), [HTTP Client](http-client.md),
and [HTTP Server](http-server.md).

## Databases

| Namespace | Purpose |
| --- | --- |
| `Whim\Database` | shared database, result, transaction, and pool contracts |
| `Whim\Database\PostgreSQL` | event-loop PostgreSQL driver |
| `Whim\Database\SQLite` | blocking-pool SQLite driver |

See [Databases](database.md).

## Mail

| Namespace | Purpose |
| --- | --- |
| `Whim\Message` | Internet mail messages and IDs |
| `Whim\Message\Address` | mailboxes, groups, and address lists |
| `Whim\SMTP` | SMTP values, envelopes, replies, and settings |
| `Whim\SMTP\Client` | SMTP transport and delivery reports |
| `Whim\SMTP\Client\Authentication` | PLAIN, LOGIN, CRAM-MD5, SCRAM, and XOAUTH2 |

See [Mail and MIME Messages](mail.md).

## Security and random data

| Namespace | Purpose |
| --- | --- |
| `Whim\Hash` | digests, HMAC, checksums, and streaming hashers |
| `Whim\Password` | bcrypt and Argon2 password hashes |
| `Whim\PseudoRandom` | the process pseudo-random sequence |
| `Whim\RandomSequence` | repeatable and secure random sequence objects |
| `Whim\SecureRandom` | operating-system random bytes, strings, ints, and floats |

See [Hashes, Passwords, and Random Data](security.md).
