//! MIME content detection.

use file_format::FileFormat;
use sonic_rs::Value as JsonValue;
use sonic_rs::from_slice;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

#[whim_function(
    "Whim\\_Private\\mime_sniff(string $bytes): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn sniff(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    if bytes.is_empty() {
        return context.string(b"application/octet-stream");
    }

    if looks_like_json(bytes) {
        return context.string(b"application/json");
    }

    let format = FileFormat::from_bytes(bytes);
    if format == FileFormat::Mpeg4Part14 {
        return context.string(b"video/mp4");
    }

    context.string(format.media_type().as_bytes())
}

fn looks_like_json(bytes: &[u8]) -> bool {
    let bytes = bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(bytes)
        .trim_ascii_start();
    matches!(bytes.first(), Some(b'{' | b'[')) && from_slice::<JsonValue>(bytes).is_ok()
}
