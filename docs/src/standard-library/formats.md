# Encoding and Data Formats

## Text and byte encodings

`Whim\Encoding` groups encodings by format:

- `Base32` supports its standard alphabets and optional padding.
- `Base64` supports standard and URL-safe alphabets and optional padding.
- `Hex` encodes and decodes hexadecimal text.
- `URI` percent-encodes full URI references without changing their syntax.
- `Url` handles component and form encoding.
- `Punycode` converts international labels.
- `QuotedPrintable` handles text and binary modes plus strict or forgiving input.
- `EncodedWord` handles encoded words used in mail headers.
- `UTF8\lossy` replaces invalid UTF-8 with the replacement character.

Decode functions throw `DecodingException` on malformed input. Encode
functions throw `EncodingException` when the format cannot hold a value.

These functions are not interchangeable. `URI\encode` preserves URI
delimiters and valid percent escapes. `URI\decode` leaves escaped delimiters
encoded, so decoding cannot turn data into URI syntax. URL form encoding maps
spaces and plus signs by form rules; component encoding does not. Base64
URL-safe text uses a different alphabet from normal Base64.

## JSON

`Json\Value` is:

```text
null|bool|int|float|string|vec<Value>|dict<string, Value>
```

`Json\encode($value, $pretty)` accepts that union or an object implementing
`ToJson`. `decode($text)` returns `Json\Value`. `decode_as::<T>()` calls
`T::fromJson()` for a type that implements `FromJson`.

```whim
use Whim\Json;

$encoded = Json\encode(dict['ready' => true, 'count' => 3]);
$decoded = Json\decode($encoded);
assert!($decoded is dict<string, Json\Value>);
```

JSON objects have string keys. The encoder rejects non-finite floats. Bad text
throws `DecodingException`; an unsupported value throws `EncodingException`.

## CSV

`CSV\Reader` is an iterator of `vec<string>` records over an `IO\ReadHandle`.
Its constructor sets the delimiter, enclosure, and escape byte. It reads as the
caller asks for rows.

`CSV\Writer` writes records to an `IO\WriteHandle`. `writeAll` accepts an
iterable of records. The `CSV\read` and `CSV\write` functions are short
constructors.

Malformed quoting throws `MalformedCSVException`. CSV has no built-in schema;
all fields are strings.

## BSON

`BSON\encode` writes a `BSON\Document` to one byte string. `BSON\decode`
reads one complete document. `ToBson` and `FromBson` map application values to
and from documents.

The value union includes arrays and documents, 32-bit integers, binary data,
object IDs, wall-clock times, regular expressions, timestamps, Decimal128,
JavaScript, symbols, database pointers, and BSON marker values. `ObjectId` can
parse, generate, compare, and render identifiers. `Binary::fromUUID` and
`toUUID` convert UUID binary values.

`BSON\Reader` reads consecutive documents from an `IO\ReadHandle`.
`BSON\Writer` writes them to an `IO\WriteHandle`. The reader rejects documents
over 16 MiB by default; its constructor can set a lower or higher limit.

Bad bytes throw `DecodingException`. Values that BSON cannot represent throw
`EncodingException`.

## Compression

`Compression\Codec` creates a `Compressor` and `Decompressor`. Whim supplies:

- `Gzip`
- `Deflate`
- `Brotli`
- `Zstandard`

A transformer accepts chunks through `push($bytes)` and returns output now
ready. `finish()` returns the last bytes. A compressor also has `flush()`.
Finish a transformer once and do not push more input after it.

`TransformReadHandle` applies a transformer while reading another handle. This
keeps the codec independent from files, sockets, and HTTP.

`Registry` maps lowercase content-coding names to codecs. It rejects duplicate
names and provides `contains`, `get`, and `codings`. HTTP compression middleware
uses the same registry.

## HTML

`HTML\escape_text` escapes text-node content. `escape_attribute` applies the
stricter attribute rules. `decode` and `decode_attribute` apply WHATWG
character-reference rules for their matching contexts. `entity` expands one
case-sensitive named reference written without `&` or `;`, or returns `null`.

Escaping text does not make it safe as an unquoted attribute, URL, script,
style, or HTTP header. Escape for the output context.

## Regular expressions

`Regex\Pattern::compile($source)` compiles a byte regular expression or throws
`InvalidPatternException`.

The pattern can test `matches`, find one `MatchResult`, replace all literal
matches, or split a string. `Regex\escape($literal)` quotes bytes for a pattern.

`MatchResult` exposes its byte `start`, `end`, full `value`, byte `length`, and
numbered or named captures. An absent or unmatched capture returns `null`.

Patterns work on bytes. Validate or repair UTF-8 first when an application
needs Unicode text rules.

## MIME values

`MIME\MediaType` parses a type, subtype, and parameters. `essence()` omits the
parameters. `Parameters` and `Headers` are readonly ordered collections with
case-insensitive names and original values.

`ContentDisposition` parses `inline`, `attachment`, names, and filenames.
`filename()` returns a safe last path component; `unsafeFilename()` returns the
raw declared value. `ContentId` parses and creates content IDs.

`MIME\Sniff\from_string()` detects a media type from a byte prefix.
`from_handle()` samples a read handle without changing the caller's logical
content stream.

## MIME parts and multipart data

A `Part` exposes `mediaType()`, `headers()`, and a streaming `body()` handle.
`Text`, `Data`, and `RawPart` create common parts. Data transfer encoding runs
as chunks rather than copying a whole attachment.

`MultiPart` owns a boundary and an ordered list of parts. Its body streams each
boundary, header block, and part body. `MultiPart\Parser` reads a multipart body
from a handle and spools large parts from memory to a temporary file.

Set the parser's size, header, part, and spool limits for untrusted HTTP input.
Bad boundaries or fields throw `MultiPartException`.
