//! Streaming compression codecs used by the Whim standard library.

use std::cell::RefCell;
use std::io;
use std::io::Write;
use std::mem;

use brotli::CompressorWriter as BrotliEncoder;
use brotli::DecompressorWriter as BrotliDecoder;
use flate2::Compression;
use flate2::Decompress;
use flate2::FlushDecompress;
use flate2::Status;
use flate2::write::GzEncoder;
use flate2::write::MultiGzDecoder;
use flate2::write::ZlibEncoder;
use whim_macros::whim_class;
use whim_macros::whim_methods;
use zstd::stream::raw::DParameter;
use zstd::stream::raw::Decoder as RawZstdDecoder;
use zstd::stream::raw::Operation;
use zstd::stream::write::Encoder as ZstdEncoder;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;

const ENCODER: &str = "Whim\\_Private\\CompressionEncoder";
const DECODER: &str = "Whim\\_Private\\CompressionDecoder";
const BROTLI_BUFFER_SIZE: usize = 4096;
const BROTLI_WINDOW_BITS: u32 = 22;
const ZSTD_WINDOW_BITS: u32 = 23;
const ZSTD_OUTPUT_SIZE: usize = 131_072;
const DEFLATE_OUTPUT_SIZE: usize = 131_072;

enum EncoderState {
    Gzip(GzEncoder<Vec<u8>>),
    Deflate(ZlibEncoder<Vec<u8>>),
    Brotli(Box<BrotliEncoder<Vec<u8>>>),
    Zstandard(ZstdEncoder<'static, Vec<u8>>),
}

impl EncoderState {
    fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Self::Gzip(encoder) => {
                encoder.write_all(bytes)?;
                Ok(mem::take(encoder.get_mut()))
            }
            Self::Deflate(encoder) => {
                encoder.write_all(bytes)?;
                Ok(mem::take(encoder.get_mut()))
            }
            Self::Brotli(encoder) => {
                encoder.write_all(bytes)?;
                Ok(mem::take(encoder.get_mut()))
            }
            Self::Zstandard(encoder) => {
                encoder.write_all(bytes)?;
                Ok(mem::take(encoder.get_mut()))
            }
        }
    }

    fn flush(&mut self) -> io::Result<Vec<u8>> {
        match self {
            Self::Gzip(encoder) => {
                encoder.flush()?;
                Ok(mem::take(encoder.get_mut()))
            }
            Self::Deflate(encoder) => {
                encoder.flush()?;
                Ok(mem::take(encoder.get_mut()))
            }
            Self::Brotli(encoder) => {
                encoder.flush()?;
                Ok(mem::take(encoder.get_mut()))
            }
            Self::Zstandard(encoder) => {
                encoder.flush()?;
                Ok(mem::take(encoder.get_mut()))
            }
        }
    }

    fn finish(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Gzip(encoder) => encoder.finish(),
            Self::Deflate(encoder) => encoder.finish(),
            Self::Brotli(mut encoder) => {
                encoder.flush()?;
                Ok((*encoder).into_inner())
            }
            Self::Zstandard(encoder) => encoder.finish(),
        }
    }
}

struct ZstandardDecoder {
    decoder: RawZstdDecoder<'static>,
    finished_frame: bool,
}

impl ZstandardDecoder {
    fn new() -> io::Result<Self> {
        let mut decoder = RawZstdDecoder::new()?;
        decoder.set_parameter(DParameter::WindowLogMax(ZSTD_WINDOW_BITS))?;
        Ok(Self {
            decoder,
            finished_frame: false,
        })
    }

    fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }

        let mut output = Vec::new();
        let mut chunk = vec![0; ZSTD_OUTPUT_SIZE];
        let mut offset = 0;
        let mut drain = false;
        while offset < bytes.len() || drain {
            let status = self.decoder.run_on_buffers(&bytes[offset..], &mut chunk)?;
            output.extend_from_slice(&chunk[..status.bytes_written]);
            offset += status.bytes_read;
            self.finished_frame = status.remaining == 0;
            drain = status.bytes_written == chunk.len();

            if status.bytes_read == 0 && status.bytes_written == 0 {
                if offset < bytes.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the zstd decoder made no progress",
                    ));
                }

                break;
            }
        }

        Ok(output)
    }

    fn finish(self) -> io::Result<Vec<u8>> {
        if self.finished_frame {
            Ok(Vec::new())
        } else {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete zstd frame",
            ))
        }
    }
}

struct DeflateDecoder {
    decoder: Decompress,
    finished: bool,
}

impl DeflateDecoder {
    fn new() -> Self {
        Self {
            decoder: Decompress::new(true),
            finished: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        if self.finished {
            return if bytes.is_empty() {
                Ok(Vec::new())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "data follows the deflate stream",
                ))
            };
        }

        let mut output = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            output.reserve(DEFLATE_OUTPUT_SIZE);
            let input_before = self.decoder.total_in();
            let output_before = self.decoder.total_out();
            let status = self
                .decoder
                .decompress_vec(&bytes[offset..], &mut output, FlushDecompress::None)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let consumed = usize::try_from(self.decoder.total_in() - input_before)
                .map_err(io::Error::other)?;
            let produced = usize::try_from(self.decoder.total_out() - output_before)
                .map_err(io::Error::other)?;
            offset += consumed;

            if status == Status::StreamEnd {
                self.finished = true;
                if offset != bytes.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "data follows the deflate stream",
                    ));
                }

                break;
            }

            if consumed == 0 && produced == 0 {
                break;
            }
        }

        Ok(output)
    }

    fn finish(self) -> io::Result<Vec<u8>> {
        if self.finished {
            Ok(Vec::new())
        } else {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete deflate stream",
            ))
        }
    }
}

enum DecoderState {
    Gzip(MultiGzDecoder<Vec<u8>>),
    Deflate(DeflateDecoder),
    Brotli(Box<BrotliDecoder<Vec<u8>>>),
    Zstandard(ZstandardDecoder),
}

impl DecoderState {
    fn push(&mut self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Self::Gzip(decoder) => {
                decoder.write_all(bytes)?;
                Ok(mem::take(decoder.get_mut()))
            }
            Self::Deflate(decoder) => decoder.push(bytes),
            Self::Brotli(decoder) => {
                decoder.write_all(bytes)?;
                Ok(mem::take(decoder.get_mut()))
            }
            Self::Zstandard(decoder) => decoder.push(bytes),
        }
    }

    fn finish(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Gzip(decoder) => decoder.finish(),
            Self::Deflate(decoder) => decoder.finish(),
            Self::Brotli(mut decoder) => {
                decoder.close()?;
                (*decoder).into_inner().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "incomplete brotli stream")
                })
            }
            Self::Zstandard(decoder) => decoder.finish(),
        }
    }
}

#[whim_class("Whim\\_Private\\CompressionEncoder", final)]
#[derive(Default)]
pub(crate) struct CompressionEncoder {
    state: RefCell<Option<EncoderState>>,
}

default_built_in_state!(CompressionEncoder);

#[whim_methods]
impl CompressionEncoder {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "gzip(0..=9 $level): null|Whim\\_Private\\CompressionEncoder",
        static,
        must_use
    )]
    fn gzip<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        // SAFETY: the surrounding invariant proves this result is successful.
        let level = unsafe {
            unwrap_result_invariant(
                u32::try_from(arguments.int(0)),
                "a validated gzip level fits u32",
            )
        };
        create_encoder(
            context,
            EncoderState::Gzip(GzEncoder::new(Vec::new(), Compression::new(level))),
        )
    }

    #[whim_method(
        "deflate(0..=9 $level): null|Whim\\_Private\\CompressionEncoder",
        static,
        must_use
    )]
    fn deflate<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        // SAFETY: the surrounding invariant proves this result is successful.
        let level = unsafe {
            unwrap_result_invariant(
                u32::try_from(arguments.int(0)),
                "a validated deflate level fits u32",
            )
        };
        create_encoder(
            context,
            EncoderState::Deflate(ZlibEncoder::new(Vec::new(), Compression::new(level))),
        )
    }

    #[whim_method(
        "brotli(0..=11 $quality): null|Whim\\_Private\\CompressionEncoder",
        static,
        must_use
    )]
    fn brotli<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        // SAFETY: the surrounding invariant proves this result is successful.
        let quality = unsafe {
            unwrap_result_invariant(
                u32::try_from(arguments.int(0)),
                "a validated brotli quality fits u32",
            )
        };
        create_encoder(
            context,
            EncoderState::Brotli(Box::new(BrotliEncoder::new(
                Vec::new(),
                BROTLI_BUFFER_SIZE,
                quality,
                BROTLI_WINDOW_BITS,
            ))),
        )
    }

    #[whim_method(
        "zstandard(-131072..=22 $level): null|Whim\\_Private\\CompressionEncoder",
        static,
        must_use
    )]
    fn zstandard<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        // SAFETY: the surrounding invariant proves this result is successful.
        let level = unsafe {
            unwrap_result_invariant(
                i32::try_from(arguments.int(0)),
                "a validated zstandard level fits i32",
            )
        };
        let Ok(mut encoder) = ZstdEncoder::new(Vec::new(), level) else {
            return Ok(Value::null());
        };
        if encoder.window_log(ZSTD_WINDOW_BITS).is_err() {
            return Ok(Value::null());
        }

        create_encoder(context, EncoderState::Zstandard(encoder))
    }

    #[whim_method("push(string $bytes): null|string", must_use)]
    fn push(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
        let bytes = arguments.bytes(0);
        with_encoder(context, |state| state.push(bytes))
    }

    #[whim_method("flush(): null|string", must_use)]
    fn flush(context: &Context<'_, '_, '_>) -> Value {
        with_encoder(context, EncoderState::flush)
    }

    #[whim_method("finish(): null|string", must_use)]
    fn finish(context: &Context<'_, '_, '_>) -> Value {
        let receiver = context.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let built_in = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a compression encoder has built-in state",
            )
        };
        finish_state(context, &built_in.state, EncoderState::finish)
    }
}

#[whim_class("Whim\\_Private\\CompressionDecoder", final)]
#[derive(Default)]
pub(crate) struct CompressionDecoder {
    state: RefCell<Option<DecoderState>>,
}

default_built_in_state!(CompressionDecoder);

#[whim_methods]
impl CompressionDecoder {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method("gzip(): null|Whim\\_Private\\CompressionDecoder", static, must_use)]
    fn gzip(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        create_decoder(context, DecoderState::Gzip(MultiGzDecoder::new(Vec::new())))
    }

    #[whim_method("deflate(): null|Whim\\_Private\\CompressionDecoder", static, must_use)]
    fn deflate(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        create_decoder(context, DecoderState::Deflate(DeflateDecoder::new()))
    }

    #[whim_method("brotli(): null|Whim\\_Private\\CompressionDecoder", static, must_use)]
    fn brotli(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        create_decoder(
            context,
            DecoderState::Brotli(Box::new(BrotliDecoder::new(Vec::new(), BROTLI_BUFFER_SIZE))),
        )
    }

    #[whim_method(
        "zstandard(): null|Whim\\_Private\\CompressionDecoder",
        static,
        must_use
    )]
    fn zstandard(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let Ok(decoder) = ZstandardDecoder::new() else {
            return Ok(Value::null());
        };
        create_decoder(context, DecoderState::Zstandard(decoder))
    }

    #[whim_method("push(string $bytes): null|string", must_use)]
    fn push(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
        let bytes = arguments.bytes(0);
        with_decoder(context, |state| state.push(bytes))
    }

    #[whim_method("finish(): null|string", must_use)]
    fn finish(context: &Context<'_, '_, '_>) -> Value {
        let receiver = context.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let built_in = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a compression decoder has built-in state",
            )
        };
        finish_state(context, &built_in.state, DecoderState::finish)
    }
}

fn create_encoder(context: &mut Context<'_, '_, '_>, state: EncoderState) -> Result<Value, Throw> {
    let object = context.new_built_in_instance(ENCODER)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<CompressionEncoder>(&object),
            "a compression encoder has built-in state",
        )
    };
    built_in.state.replace(Some(state));
    Ok(object)
}

fn create_decoder(context: &mut Context<'_, '_, '_>, state: DecoderState) -> Result<Value, Throw> {
    let object = context.new_built_in_instance(DECODER)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<CompressionDecoder>(&object),
            "a compression decoder has built-in state",
        )
    };
    built_in.state.replace(Some(state));
    Ok(object)
}

fn with_encoder(
    context: &Context<'_, '_, '_>,
    operation: impl FnOnce(&mut EncoderState) -> io::Result<Vec<u8>>,
) -> Value {
    let receiver = context.receiver();
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<CompressionEncoder>(&receiver),
            "a compression encoder has built-in state",
        )
    };
    operate_state(context, &built_in.state, operation)
}

fn with_decoder(
    context: &Context<'_, '_, '_>,
    operation: impl FnOnce(&mut DecoderState) -> io::Result<Vec<u8>>,
) -> Value {
    let receiver = context.receiver();
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<CompressionDecoder>(&receiver),
            "a compression decoder has built-in state",
        )
    };
    operate_state(context, &built_in.state, operation)
}

fn operate_state<T>(
    context: &Context<'_, '_, '_>,
    slot: &RefCell<Option<T>>,
    operation: impl FnOnce(&mut T) -> io::Result<Vec<u8>>,
) -> Value {
    let result = {
        let mut state = slot.borrow_mut();
        let Some(active) = state.as_mut() else {
            return Value::null();
        };
        let result = operation(active);
        if result.is_err() {
            state.take();
        }
        result
    };
    stream_result(context, result)
}

fn finish_state<T>(
    context: &Context<'_, '_, '_>,
    slot: &RefCell<Option<T>>,
    finish: impl FnOnce(T) -> io::Result<Vec<u8>>,
) -> Value {
    let Some(state) = slot.borrow_mut().take() else {
        return Value::null();
    };
    stream_result(context, finish(state))
}

fn stream_result(context: &Context<'_, '_, '_>, result: io::Result<Vec<u8>>) -> Value {
    result.map_or_else(|_| Value::null(), |bytes| context.owned_string(bytes))
}
