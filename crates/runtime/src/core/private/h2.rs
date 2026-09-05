//! HTTP/2 frame encoding and incremental decoding.

use std::cell::OnceCell;
use std::cell::RefCell;

use whim_macros::whim_class;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_result_invariant;
use crate::value::Value;

const DECODER: &str = "Whim\\_Private\\H2FrameDecoder";
const MAXIMUM_FRAME_SIZE: u32 = 0x00ff_ffff;
const MAXIMUM_STREAM_ID: u32 = 0x7fff_ffff;
const PROTOCOL_ERROR: i64 = 0x1;
const FRAME_SIZE_ERROR: i64 = 0x6;

struct RawFrame {
    kind: u8,
    flags: u8,
    stream: u32,
    payload: Vec<u8>,
}

struct DecoderState {
    buffer: Vec<u8>,
    maximum_frame_size: u32,
}

struct DecodeError {
    message: &'static str,
    code: i64,
}

#[whim_class("Whim\\_Private\\H2FrameDecoder", final)]
#[derive(Default)]
pub(crate) struct H2FrameDecoder {
    state: OnceCell<RefCell<DecoderState>>,
}

default_built_in_state!(H2FrameDecoder);

#[whim_methods]
impl H2FrameDecoder {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "create(int $maximumFrameSize): Whim\\_Private\\H2FrameDecoder",
        static
    )]
    fn create<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let maximum_frame_size = frame_size(context, arguments.int(0))?;
        let object = context.new_built_in_instance(DECODER)?;
        let Some(decoder) = state_ref::<Self>(&object) else {
            return Err(context.type_error("the HTTP/2 frame decoder has no built-in state"));
        };

        if decoder
            .state
            .set(RefCell::new(DecoderState {
                buffer: Vec::new(),
                maximum_frame_size,
            }))
            .is_err()
        {
            return Err(context.type_error("the HTTP/2 frame decoder is already initialized"));
        }
        Ok(object)
    }

    #[whim_method("push(string $bytes): vec<(int, int, int, string)>")]
    fn push<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.bytes(0);
        let receiver = context.receiver();
        let Some(decoder) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the HTTP/2 frame decoder has no built-in state"));
        };
        let Some(state) = decoder.state.get() else {
            return Err(context.type_error("the HTTP/2 frame decoder is not initialized"));
        };
        let frames = decode_frames(&mut state.borrow_mut(), bytes)
            .map_err(|error| decoding_error(context, error.message, error.code))?;
        let frames = frames
            .into_iter()
            .map(|frame| {
                context.tuple([
                    Value::int(i64::from(frame.kind)),
                    Value::int(i64::from(frame.flags)),
                    Value::int(i64::from(frame.stream)),
                    context.owned_string(frame.payload),
                ])
            })
            .collect::<Vec<_>>();

        Ok(context.vec(frames))
    }

    #[whim_method("finish(): void")]
    fn finish(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let receiver = context.receiver();
        let Some(decoder) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the HTTP/2 frame decoder has no built-in state"));
        };
        let Some(state) = decoder.state.get() else {
            return Err(context.type_error("the HTTP/2 frame decoder is not initialized"));
        };
        if !state.borrow().buffer.is_empty() {
            return Err(decoding_error(
                context,
                "the HTTP/2 frame is truncated",
                PROTOCOL_ERROR,
            ));
        }

        Ok(Value::null())
    }
}

#[whim_function(
    "Whim\\_Private\\h2_encode_frame(int $type, int $flags, int $stream, string $payload): string"
)]
pub(crate) fn encode_frame(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let kind = u8::try_from(arguments.int(0))
        .map_err(|_| context.type_error("the HTTP/2 frame type must fit in one byte"))?;
    let flags = u8::try_from(arguments.int(1))
        .map_err(|_| context.type_error("the HTTP/2 frame flags must fit in one byte"))?;
    let stream = u32::try_from(arguments.int(2))
        .ok()
        .filter(|stream| *stream <= MAXIMUM_STREAM_ID)
        .ok_or_else(|| context.type_error("the HTTP/2 stream identifier is out of range"))?;
    let payload = arguments.bytes(3);
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|length| *length <= MAXIMUM_FRAME_SIZE)
        .ok_or_else(|| context.type_error("the HTTP/2 frame payload is too large"))?;

    let mut encoded = Vec::with_capacity(9 + payload.len());
    encoded.extend_from_slice(&length.to_be_bytes()[1..]);
    encoded.push(kind);
    encoded.push(flags);
    encoded.extend_from_slice(&stream.to_be_bytes());
    encoded.extend_from_slice(payload);
    Ok(context.owned_string(encoded))
}

fn frame_size(context: &mut Context<'_, '_, '_>, value: i64) -> Result<u32, Throw> {
    u32::try_from(value)
        .ok()
        .filter(|size| *size <= MAXIMUM_FRAME_SIZE)
        .ok_or_else(|| context.type_error("the HTTP/2 maximum frame size is out of range"))
}

fn decode_frames(state: &mut DecoderState, bytes: &[u8]) -> Result<Vec<RawFrame>, DecodeError> {
    state.buffer.extend_from_slice(bytes);
    let mut frames = Vec::new();
    let mut offset = 0;
    while state.buffer.len() - offset >= 9 {
        let length = u32::from_be_bytes([
            0,
            state.buffer[offset],
            state.buffer[offset + 1],
            state.buffer[offset + 2],
        ]);
        if length > state.maximum_frame_size {
            return Err(DecodeError {
                message: "the HTTP/2 frame exceeds the configured maximum size",
                code: FRAME_SIZE_ERROR,
            });
        }
        // SAFETY: the surrounding invariant proves this result is successful.
        let length = unsafe {
            unwrap_result_invariant(
                usize::try_from(length),
                "an HTTP/2 frame length fits in the host address space",
            )
        };
        let frame_length = 9 + length;
        if state.buffer.len() - offset < frame_length {
            break;
        }

        let stream = u32::from_be_bytes([
            state.buffer[offset + 5],
            state.buffer[offset + 6],
            state.buffer[offset + 7],
            state.buffer[offset + 8],
        ]) & 0x7fff_ffff;
        frames.push(RawFrame {
            kind: state.buffer[offset + 3],
            flags: state.buffer[offset + 4],
            stream,
            payload: state.buffer[offset + 9..offset + frame_length].to_vec(),
        });
        offset += frame_length;
    }

    if offset > 0 {
        let remaining = state.buffer.len() - offset;
        state.buffer.copy_within(offset.., 0);
        state.buffer.truncate(remaining);
    }
    Ok(frames)
}

fn decoding_error(context: &mut Context<'_, '_, '_>, message: &str, code: i64) -> Throw {
    let class = context
        .vm
        .intern(b"Whim\\HTTP\\_Private\\FrameDecodingException");
    context.vm.throw(class, message, code)
}
