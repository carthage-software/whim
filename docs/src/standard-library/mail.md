# Mail and MIME Messages

`Whim\Message` models Internet mail. `Whim\MIME` supplies its content parts and
headers. `Whim\SMTP` sends a message and envelope.

## Addresses

`Message\Address\Mailbox` stores local part, domain, and optional display name.
`Group` stores a named address group. `AddressList` holds mailboxes and groups,
supports iteration, and can flatten its mailboxes.

Each type has nullable `parse`, throwing `from`, and `toString`. `Mailbox` also
has checked `fromParts`. Output constructors enforce valid dot-atom and domain
shape instead of accepting header injection.

## Message IDs

`MessageId` parses, creates, and formats a message identifier. `generate()`
uses secure random data plus an optional domain. `MIME\ContentId` supplies the
same shape for body parts and can form a `cid:` URI.

## Message

`Message\Message::create()` starts an immutable message. `withFrom`,
`withSender`, `withTo`, `withCc`, `withBcc`, `withReplyTo`, `withDate`,
`withMessageId`, `withSubject`, `withReferences`, `withInReplyTo`, and
`withContent` return changed messages and keep matching headers in sync.

`withHeader` and `withoutHeader` handle custom fields. `fromHeaders` parses the
known structured fields from an existing MIME header map.

`Message\parse($handle)` reads a message and MIME body. `serialize($message)`
returns a streaming read handle. Large attachments stay streaming through
transfer encoding and multipart output.

## Envelope

An SMTP envelope is separate from visible message headers. `Envelope::fromParts`
takes an optional sender and at least one recipient. `fromMessage` derives the
sender from `Sender` or the first `From` mailbox and recipients from To, Cc, and
Bcc.

Bcc addresses belong in the envelope but should not appear in serialized
visible headers.

## SMTP values

`SMTP\Command` and `Reply` parse and format protocol lines.
`EnhancedStatusCode` stores its class, subject, and detail. Enums name
capabilities, security modes, priority, delivery status requests, return modes,
and delivery deadlines.

## Transport

`SMTP\Client\DefaultTransport` connects, negotiates EHLO features, applies
optional authentication, sends a message, and pools idle connections.

Transport configuration sets host, optional port, plaintext, STARTTLS, or
implicit TLS, local hostname, pipelining, chunking, chunk size, partial-success
policy, idle pool limits, connector, and TLS settings.

When the caller omits a port, STARTTLS uses 587, implicit TLS uses 465, and
plaintext uses 25.

Idle connections have a timeout. A checked-out idle connection must answer
`NOOP`; otherwise the transport closes it and opens a new one.

## Authentication

Authenticators implement one contract over an SMTP connection. The library
includes PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, and XOAUTH2. Credentials carry
`SensitiveParameter` markers.

The server must advertise the chosen method. A missing or rejected method throws
`AuthenticationException`, not a missing dict-key error.

## Delivery options and report

Per-send settings cover delivery status notifications, envelope ID, required
TLS, priority, deliver-by, and future release when the server advertises those
extensions.

With partial success disabled, a rejected recipient fails the send. With it
enabled, `DeliveryReport` lists every rejected mailbox and reply, including the
case where the server rejects all recipients.

Connection, authentication, protocol, extension, and transmission failures use
separate exception types.
