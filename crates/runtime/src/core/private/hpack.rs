//! Stateful HPACK encoding and decoding.

use std::cell::OnceCell;
use std::cell::RefCell;
use std::fmt::Display;

use httlib_hpack::Decoder;
use httlib_hpack::Encoder;
use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unreachable_invariant;
use crate::unwrap_option_invariant;
use crate::value::Value;

const HPACK_ENCODER: &str = "Whim\\_Private\\HpackEncoder";
const HPACK_DECODER: &str = "Whim\\_Private\\HpackDecoder";

struct EncoderState {
    encoder: Encoder<'static>,
    pending_minimum_capacity: Option<u32>,
    pending_capacity: Option<u32>,
}

#[whim_class("Whim\\_Private\\HpackEncoder", final)]
#[derive(Default)]
pub(crate) struct HpackEncoder {
    state: OnceCell<RefCell<EncoderState>>,
}

default_built_in_state!(HpackEncoder);

#[whim_methods]
impl HpackEncoder {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "create(int $maximumTableCapacity): Whim\\_Private\\HpackEncoder",
        static
    )]
    fn create<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let capacity = capacity(context, arguments.int(0))?;
        let object = context.new_built_in_instance(HPACK_ENCODER)?;
        let Some(built_in) = state_ref::<Self>(&object) else {
            return Err(context.type_error("the HPACK encoder has no built-in state"));
        };

        if built_in
            .state
            .set(RefCell::new(EncoderState {
                encoder: Encoder::with_dynamic_size(capacity),
                pending_minimum_capacity: None,
                pending_capacity: None,
            }))
            .is_err()
        {
            return Err(context.type_error("the HPACK encoder is already initialized"));
        }

        Ok(object)
    }

    #[whim_method("resize(int $capacity): void")]
    fn resize<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let capacity = capacity(context, arguments.int(0))?;
        let receiver = context.receiver();
        let Some(built_in) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the HPACK encoder has no built-in state"));
        };
        let Some(state) = built_in.state.get() else {
            return Err(context.type_error("the HPACK encoder is not initialized"));
        };
        let mut state = state.borrow_mut();

        state.pending_minimum_capacity = Some(
            state
                .pending_minimum_capacity
                .map_or(capacity, |minimum| minimum.min(capacity)),
        );

        state.pending_capacity = Some(capacity);
        Ok(Value::null())
    }

    #[whim_method("encode(vec<(string, string, bool)> $headers): string")]
    fn encode<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let headers = arguments.vec(0);
        let receiver = context.receiver();
        let Some(built_in) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the HPACK encoder has no built-in state"));
        };
        let Some(state) = built_in.state.get() else {
            return Err(context.type_error("the HPACK encoder is not initialized"));
        };
        let encoded = encode_headers(&mut state.borrow_mut(), headers.iter())
            .map_err(|error| hpack_error(context, error))?;

        Ok(context.owned_string(encoded))
    }
}

fn encode_headers<'value>(
    state: &mut EncoderState,
    headers: impl IntoIterator<Item = &'value Value>,
) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::new();
    if let Some(capacity) = state.pending_capacity.take() {
        if let Some(minimum) = state.pending_minimum_capacity.take()
            && minimum != capacity
        {
            state
                .encoder
                .update_max_dynamic_size(minimum, &mut encoded)
                .map_err(|error| error.to_string())?;
        }

        state
            .encoder
            .update_max_dynamic_size(capacity, &mut encoded)
            .map_err(|error| error.to_string())?;
    }

    for header in headers {
        let (name, value, sensitive) = header_parts(header);
        let flags = if sensitive {
            Encoder::NEVER_INDEXED | Encoder::HUFFMAN_NAME | Encoder::HUFFMAN_VALUE
        } else {
            Encoder::WITH_INDEXING
                | Encoder::BEST_FORMAT
                | Encoder::HUFFMAN_NAME
                | Encoder::HUFFMAN_VALUE
        };

        state
            .encoder
            .encode((name.to_vec(), value.to_vec(), flags), &mut encoded)
            .map_err(|error| error.to_string())?;
    }

    Ok(encoded)
}

fn header_parts(header: &Value) -> (&[u8], &[u8], bool) {
    // SAFETY: the surrounding invariant proves this option contains a value.
    let header = unsafe {
        unwrap_option_invariant(header.as_tuple(), "a validated HPACK header is a tuple")
    };
    let [name, value, sensitive] = header.as_slice() else {
        // SAFETY: the surrounding invariant makes this path unreachable.
        unsafe { unreachable_invariant("a validated HPACK header has exactly three fields") }
    };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let name = unsafe {
        unwrap_option_invariant(
            name.as_string_bytes(),
            "a validated HPACK header name is a string",
        )
    };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let value = unsafe {
        unwrap_option_invariant(
            value.as_string_bytes(),
            "a validated HPACK header value is a string",
        )
    };
    // SAFETY: the surrounding invariant proves this option contains a value.
    let sensitive = unsafe {
        unwrap_option_invariant(
            sensitive.as_bool(),
            "a validated HPACK header policy is a boolean",
        )
    };
    (name, value, sensitive)
}

#[whim_class("Whim\\_Private\\HpackDecoder", final)]
#[derive(Default)]
pub(crate) struct HpackDecoder {
    decoder: OnceCell<RefCell<Decoder<'static>>>,
}

default_built_in_state!(HpackDecoder);

#[whim_methods]
impl HpackDecoder {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "create(int $maximumTableCapacity): Whim\\_Private\\HpackDecoder",
        static
    )]
    fn create<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let capacity = capacity(context, arguments.int(0))?;
        let object = context.new_built_in_instance(HPACK_DECODER)?;
        let Some(built_in) = state_ref::<Self>(&object) else {
            return Err(context.type_error("the HPACK decoder has no built-in state"));
        };

        if built_in
            .decoder
            .set(RefCell::new(Decoder::with_dynamic_size(capacity)))
            .is_err()
        {
            return Err(context.type_error("the HPACK decoder is already initialized"));
        }
        Ok(object)
    }

    #[whim_method("resize(int $capacity): void")]
    fn resize<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let capacity = capacity(context, arguments.int(0))?;
        let receiver = context.receiver();
        let Some(built_in) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the HPACK decoder has no built-in state"));
        };
        let Some(decoder) = built_in.decoder.get() else {
            return Err(context.type_error("the HPACK decoder is not initialized"));
        };

        decoder.borrow_mut().set_max_dynamic_size(capacity);
        Ok(Value::null())
    }

    #[whim_method("decode(string $bytes): vec<(string, string, bool)>")]
    fn decode<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let mut bytes = arguments.bytes(0).to_vec();
        let receiver = context.receiver();
        let Some(built_in) = state_ref::<Self>(&receiver) else {
            return Err(context.type_error("the HPACK decoder has no built-in state"));
        };
        let Some(decoder) = built_in.decoder.get() else {
            return Err(context.type_error("the HPACK decoder is not initialized"));
        };
        let result = {
            let mut headers = Vec::new();
            decoder
                .borrow_mut()
                .decode(&mut bytes, &mut headers)
                .map(|_| headers)
                .map_err(|error| error.to_string())
        };
        let headers = result.map_err(|error| hpack_error(context, error))?;

        let headers = headers
            .into_iter()
            .map(|(name, value, flags)| {
                context.tuple([
                    context.owned_string(name),
                    context.owned_string(value),
                    Value::bool(flags & Decoder::NEVER_INDEXED != 0),
                ])
            })
            .collect::<Vec<_>>();
        Ok(context.vec(headers))
    }
}

fn capacity(context: &mut Context<'_, '_, '_>, value: i64) -> Result<u32, Throw> {
    u32::try_from(value).map_err(|_| context.type_error("the HPACK table capacity is too large"))
}

fn hpack_error(context: &mut Context<'_, '_, '_>, error: impl Display) -> Throw {
    let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
    context.vm.throw(class, &error.to_string(), 0)
}
