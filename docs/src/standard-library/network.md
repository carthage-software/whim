# Network, TLS, and Proxies

Network APIs use the event loop and accept cancellation tokens on waits. They
return typed endpoints and capability interfaces rather than raw descriptor
numbers.

## IP addresses and CIDR

`IP\Address` stores either four IPv4 bytes or sixteen IPv6 bytes. `parse`
returns `null`; `from` throws on bad text. `v4` and `v6` require one family.
`fromBytes` accepts an exact 4-byte or 16-byte string.

An address can return canonical, expanded, byte, and reverse-DNS forms. It can
test loopback, private, link-local, multicast, unspecified, documentation, and
global unicast ranges. It can also test and create IPv4-mapped IPv6 addresses.

`CIDR\Block` joins an address and prefix. It reports membership, overlap, and
the first and last address.

## International domain names

`IDNA\to_ascii` converts a Unicode domain name to its ASCII form.
`IDNA\to_unicode` converts an IDNA domain name to Unicode. Both apply UTS #46,
strict host-name rules, and DNS length limits. Invalid input raises an encoding
or decoding exception.

## URI, IRI, and URL

`URI\URI` follows generic URI syntax. It may be relative. It stores optional
scheme and authority plus path, query, and fragment. `URI\resolve` resolves a
reference against a base.

`IRI\IRI` permits international text and converts to and from a URI. Its host
handling applies IDNA through the standard conversion path.

`URL\URL` is the stricter form used by network clients. It requires a scheme
and authority, uses a numeric port, exposes an `Origin`, and reads or builds
query parameters. It converts to URI or IRI.

All three types have nullable `parse`, throwing `from`, readonly public parts,
`with...` copies, normalization, equality, and `toString`.

Use `URI` for a relative reference, `IRI` for an international identifier, and
`URL` for an address a client can connect to.

## Endpoints and streams

`Network\InternetEndpoint` stores an `IP\Address` and port. `UnixEndpoint`
stores a local socket path or `null` for an unnamed endpoint.

`Network\Stream<TEndpoint>` is a read, write, close, and file-descriptor handle.
It reports local and peer endpoints and can shut down reads, writes, or both.
`Listener<TEndpoint>` accepts streams.

An empty read is not enough to prove closure. Use the read-handle end check.

## TCP

`TCP\connect($host, $port, $configuration, $cancellation)` resolves a name or
uses an IP address and returns a `TCP\Stream`.

`TCP\listen()` binds an address and returns a listener. Listen configuration
controls no-delay on accepted streams, address and port reuse, IPv6-only mode,
and backlog. Connect configuration controls no-delay and an optional local bind.

`DefaultConnector` implements the connector interface for reusable clients.
`SecureConnector` wraps another connector and performs TLS. Secure streams
implement both the TCP and TLS stream contracts.

## UDP

`UDP\bind()` returns a datagram socket. `sendTo` sends bytes and metadata to an
endpoint. `receive` and `tryReceive` return a `Datagram` with bytes, sender,
destination details, and congestion data when the host supplies it.

Connecting a UDP socket fixes its peer and returns `ConnectedSocket`; it does
not create a byte stream. Connected sends still preserve datagram boundaries.

Bind configuration controls reuse, broadcast, IPv6-only mode, and socket buffer
sizes.

## Unix sockets

`Unix\connect` and `Unix\listen` use local socket paths. `Unix\pair()` returns
two connected non-blocking streams. All expose file descriptors and work with
the same I/O and cancellation APIs as TCP.

## SOCKS

`SOCKS\Connector` implements `TCP\ProxyConnector`. Its configuration sets the
proxy host and port plus optional username and password. It performs SOCKS5
negotiation, reports authentication and protocol errors, and then returns the
same TCP stream interface.

## TLS identities and settings

`TLS\Certificate` reads DER or PEM certificate data. `Identity` joins a
certificate chain and private key.

Client configuration controls system roots, added roots, an optional client
identity, ALPN values, TLS version bounds, peer verification, name checking,
SNI, and session reuse.

Verification defaults to full checks. `AllowSelfSigned` supports local work;
`Disabled` removes peer checks and should not protect a real remote connection.

Server configuration starts with an identity and can add SNI identities,
client roots, optional or required client authentication, ALPN values, version
bounds, and session reuse.

## TLS over a stream

`TLS\Connector<TEndpoint>` wraps any matching network connector, not only TCP.
`Acceptor<TEndpoint>` wraps an accepted stream. `TLS\listen()` wraps a listener.

A secure stream still exposes normal network reads and writes. Its
`ConnectionState` reports the negotiated version, cipher suite, ALPN value,
handshake kind, peer certificates, and server name where available.

TLS closes and errors remain separate from the transport's own close and error
types. Always close the secure stream, not only the stream under it.
