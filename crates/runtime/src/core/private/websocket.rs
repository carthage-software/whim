//! WebSocket framing and handshake primitives.

use std::cell::RefCell;
use std::io;
use std::io::Read;
use std::io::Write;
use std::mem;
use std::str;

use base64ct::Encoding as _;
use bytes::Buf;
use bytes::Bytes;
use bytes::BytesMut;
use sha1::Digest as _;
use tungstenite::Error as WebSocketError;
use tungstenite::Message;
use tungstenite::Utf8Bytes;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::Role;
use tungstenite::protocol::WebSocket;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::protocol::frame::coding::CloseCode;

use whim_macros::whim_class;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;

const CODEC: &str = "Whim\\_Private\\WebSocketCodec";
const EXCEPTION: &[u8] = b"Whim\\HTTP\\WebSocket\\Exception";
const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const EVENT_TEXT: i64 = 1;
const EVENT_BINARY: i64 = 2;
const EVENT_CLOSE: i64 = 8;

#[derive(Debug, Default)]
struct BufferTransport {
    input: BytesMut,
    output: Vec<u8>,
    input_finished: bool,
}

impl BufferTransport {
    fn push(&mut self, bytes: &[u8]) -> bool {
        if self.input_finished {
            return false;
        }

        self.input.extend_from_slice(bytes);
        true
    }

    const fn finish(&mut self) {
        self.input_finished = true;
    }

    fn take_output(&mut self) -> Vec<u8> {
        mem::take(&mut self.output)
    }
}

impl Read for BufferTransport {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.input.is_empty() {
            if self.input_finished {
                return Ok(0);
            }

            return Err(io::ErrorKind::WouldBlock.into());
        }

        let length = output.len().min(self.input.len());
        self.input.copy_to_slice(&mut output[..length]);
        Ok(length)
    }
}

impl Write for BufferTransport {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct CodecState {
    socket: WebSocket<BufferTransport>,
    closed: bool,
}

impl CodecState {
    fn new(role: Role, maximum_message_size: usize, maximum_frame_size: usize) -> Self {
        let configuration = WebSocketConfig::default()
            .read_buffer_size(16 * 1024)
            .write_buffer_size(0)
            .max_message_size(Some(maximum_message_size))
            .max_frame_size(Some(maximum_frame_size));

        Self {
            socket: WebSocket::from_raw_socket(
                BufferTransport::default(),
                role,
                Some(configuration),
            ),
            closed: false,
        }
    }
}

struct Event {
    kind: i64,
    payload: Vec<u8>,
    close_code: i64,
}

#[whim_class("Whim\\_Private\\WebSocketCodec", final)]
#[derive(Default)]
pub(crate) struct WebSocketCodec {
    state: RefCell<Option<CodecState>>,
}

default_built_in_state!(WebSocketCodec);

#[whim_methods]
impl WebSocketCodec {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "server(1.. $maximumMessageSize, 1.. $maximumFrameSize): Whim\\_Private\\WebSocketCodec",
        static,
        must_use
    )]
    fn server<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        create_codec(context, arguments, Role::Server)
    }

    #[whim_method(
        "client(1.. $maximumMessageSize, 1.. $maximumFrameSize): Whim\\_Private\\WebSocketCodec",
        static,
        must_use
    )]
    fn client<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        create_codec(context, arguments, Role::Client)
    }

    #[whim_method("push(string $bytes): void")]
    fn push<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.bytes(0);
        with_state(context, |state| {
            if !state.socket.get_mut().push(bytes) {
                return Err(StateError::Message(
                    "cannot push bytes after finishing WebSocket input",
                ));
            }

            Ok(())
        })?;

        Ok(Value::null())
    }

    #[whim_method("finishInput(): void")]
    fn finish_input(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        with_state(context, |state| {
            state.socket.get_mut().finish();
            Ok(())
        })?;

        Ok(Value::null())
    }

    #[whim_method("nextEvent(): null|(int, string, int)", must_use)]
    fn next_event(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let event = with_state(context, receive_event)?;
        let Some(event) = event else {
            return Ok(Value::null());
        };

        Ok(context.tuple([
            Value::int(event.kind),
            Value::from_string_vec(context.vm.heap(), event.payload),
            Value::int(event.close_code),
        ]))
    }

    #[whim_method("takeOutput(): string", must_use)]
    fn take_output(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let output = with_state(context, |state| {
            flush(state)?;
            Ok(state.socket.get_mut().take_output())
        })?;

        Ok(Value::from_string_vec(context.vm.heap(), output))
    }

    #[whim_method("sendText(string $text): void")]
    fn send_text<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.bytes(0);
        let text = str::from_utf8(bytes)
            .map_err(|_| {
                websocket_exception(
                    context,
                    "text in a WebSocket frame must be valid UTF-8",
                    1007,
                )
            })?
            .to_owned();
        send(context, Message::Text(Utf8Bytes::from(text)))?;
        Ok(Value::null())
    }

    #[whim_method("sendBinary(string $bytes): void")]
    fn send_binary<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let bytes = arguments.bytes(0);
        send(context, Message::Binary(Bytes::copy_from_slice(bytes)))?;
        Ok(Value::null())
    }

    #[whim_method("ping(string $payload = ''): void")]
    fn ping<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let payload: &[u8] = if arguments.is_absent(0) {
            &[]
        } else {
            arguments.bytes(0)
        };
        if payload.len() > 125 {
            return Err(websocket_exception(
                context,
                "a WebSocket ping payload cannot exceed 125 bytes",
                1002,
            ));
        }

        send(context, Message::Ping(Bytes::copy_from_slice(payload)))?;
        Ok(Value::null())
    }

    #[whim_method("close(null|int $code = null, string $reason = ''): void")]
    fn close<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let code = arguments.optional_int(0);
        let reason: &[u8] = if arguments.is_absent(1) {
            &[]
        } else {
            arguments.bytes(1)
        };
        let frame = close_frame(context, code, reason)?;
        with_state(context, |state| match state.socket.close(frame) {
            Ok(()) => Ok(()),
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                state.closed = true;
                Ok(())
            }
            Err(error) => Err(StateError::WebSocket(error)),
        })?;

        Ok(Value::null())
    }

    #[whim_method("isClosed(): bool", must_use)]
    fn is_closed(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
        let closed = with_state(context, |state| Ok(state.closed))?;
        Ok(Value::bool(closed))
    }
}

fn create_codec<'call>(
    context: &mut Context<'call, '_, '_>,
    arguments: Arguments<'call>,
    role: Role,
) -> Result<Value, Throw> {
    let maximum_message_size = size(
        context,
        arguments.int(0),
        "the maximum WebSocket message size is out of range",
    )?;
    let maximum_frame_size = size(
        context,
        arguments.int(1),
        "the maximum WebSocket frame size is out of range",
    )?;
    let object = context.new_built_in_instance(CODEC)?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<WebSocketCodec>(&object),
            "a WebSocket codec has built-in state",
        )
    };
    *built_in.state.borrow_mut() = Some(CodecState::new(
        role,
        maximum_message_size,
        maximum_frame_size,
    ));

    Ok(object)
}

#[whim_function(
    "Whim\\_Private\\websocket_accept_key(string $key): null|string[28]",
    must_use
)]
pub(crate) fn accept_key(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let key = arguments.bytes(0);
    let Ok(key_text) = str::from_utf8(key) else {
        return Value::null();
    };
    let mut decoded = [0_u8; 16];
    let Ok(decoded) = base64ct::Base64::decode(key_text, &mut decoded) else {
        return Value::null();
    };
    if decoded.len() != 16 {
        return Value::null();
    }

    let mut digest = sha1::Sha1::new();
    digest.update(key);
    digest.update(GUID);
    let digest = digest.finalize();
    let mut encoded = [0_u8; 28];
    // SAFETY: the surrounding invariant proves this result is successful.
    let accept = unsafe {
        unwrap_result_invariant(
            base64ct::Base64::encode(&digest, &mut encoded),
            "a 28-byte buffer holds the Base64 encoding of a SHA-1 digest",
        )
    };
    context.string(accept.as_bytes())
}

enum StateError {
    Message(&'static str),
    WebSocket(WebSocketError),
}

fn with_state<T>(
    context: &mut Context<'_, '_, '_>,
    operation: impl FnOnce(&mut CodecState) -> Result<T, StateError>,
) -> Result<T, Throw> {
    let receiver = context.receiver();
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<WebSocketCodec>(&receiver),
            "a WebSocket codec has built-in state",
        )
    };
    let result = {
        let mut state = built_in.state.borrow_mut();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let state = unsafe {
            unwrap_option_invariant(
                state.as_mut(),
                "a WebSocket codec is initialized by its factory",
            )
        };
        operation(state)
    };

    match result {
        Ok(value) => Ok(value),
        Err(StateError::Message(message)) => Err(websocket_exception(context, message, 0)),
        Err(StateError::WebSocket(error)) => {
            let code = i64::from(close_code_for_error(&error));
            Err(websocket_exception(context, &error.to_string(), code))
        }
    }
}

fn receive_event(state: &mut CodecState) -> Result<Option<Event>, StateError> {
    loop {
        match state.socket.read() {
            Ok(Message::Text(text)) => {
                return Ok(Some(Event {
                    kind: EVENT_TEXT,
                    payload: text.as_bytes().to_vec(),
                    close_code: 0,
                }));
            }
            Ok(Message::Binary(bytes)) => {
                return Ok(Some(Event {
                    kind: EVENT_BINARY,
                    payload: bytes.to_vec(),
                    close_code: 0,
                }));
            }
            Ok(Message::Close(frame)) => {
                state.closed = true;
                let (payload, close_code) = match frame {
                    Some(frame) => (
                        frame.reason.as_bytes().to_vec(),
                        i64::from(u16::from(frame.code)),
                    ),
                    None => (Vec::new(), 0),
                };
                return Ok(Some(Event {
                    kind: EVENT_CLOSE,
                    payload,
                    close_code,
                }));
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Ok(Message::Frame(_)) => {
                return Err(StateError::Message(
                    "the WebSocket codec returned an unexpected raw frame",
                ));
            }
            Err(WebSocketError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(None);
            }
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
                state.closed = true;
                return Ok(None);
            }
            Err(error) => {
                queue_error_close(state, &error);
                return Err(StateError::WebSocket(error));
            }
        }
    }
}

fn send(context: &mut Context<'_, '_, '_>, message: Message) -> Result<(), Throw> {
    with_state(context, |state| match state.socket.send(message) {
        Ok(()) => Ok(()),
        Err(error) => {
            queue_error_close(state, &error);
            Err(StateError::WebSocket(error))
        }
    })
}

fn flush(state: &mut CodecState) -> Result<(), StateError> {
    match state.socket.flush() {
        Ok(()) => Ok(()),
        Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => {
            state.closed = true;
            Ok(())
        }
        Err(error) => Err(StateError::WebSocket(error)),
    }
}

fn queue_error_close(state: &mut CodecState, error: &WebSocketError) {
    let code = CloseCode::from(close_code_for_error(error));
    let reason = match code {
        CloseCode::Invalid => "invalid UTF-8",
        CloseCode::Size => "message too large",
        CloseCode::Policy => "policy violation",
        _ => "protocol error",
    };
    let frame = CloseFrame {
        code,
        reason: Utf8Bytes::from_static(reason),
    };
    if matches!(
        state.socket.close(Some(frame)),
        Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed)
    ) {
        state.closed = true;
    }
}

const fn close_code_for_error(error: &WebSocketError) -> u16 {
    match error {
        WebSocketError::Capacity(_) => 1009,
        WebSocketError::Utf8(_) => 1007,
        WebSocketError::AttackAttempt => 1008,
        _ => 1002,
    }
}

fn close_frame(
    context: &mut Context<'_, '_, '_>,
    code: Option<i64>,
    reason: &[u8],
) -> Result<Option<CloseFrame>, Throw> {
    let Some(code) = code else {
        if reason.is_empty() {
            return Ok(None);
        }

        return Err(websocket_exception(
            context,
            "a WebSocket close reason requires a close code",
            1002,
        ));
    };
    let Ok(code) = u16::try_from(code) else {
        return Err(websocket_exception(
            context,
            "the WebSocket close code is invalid",
            1002,
        ));
    };
    let code = CloseCode::from(code);
    if !code.is_allowed() || code == CloseCode::Extension {
        return Err(websocket_exception(
            context,
            "the WebSocket close code is invalid",
            1002,
        ));
    }
    if reason.len() > 123 {
        return Err(websocket_exception(
            context,
            "a WebSocket close reason cannot exceed 123 bytes",
            1002,
        ));
    }
    let Ok(reason) = str::from_utf8(reason) else {
        return Err(websocket_exception(
            context,
            "a WebSocket close reason must be valid UTF-8",
            1007,
        ));
    };

    Ok(Some(CloseFrame {
        code,
        reason: Utf8Bytes::from(reason.to_owned()),
    }))
}

fn size(context: &mut Context<'_, '_, '_>, value: i64, message: &str) -> Result<usize, Throw> {
    usize::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| websocket_exception(context, message, 0))
}

fn websocket_exception(context: &mut Context<'_, '_, '_>, message: &str, code: i64) -> Throw {
    let class = context.vm.intern(EXCEPTION);
    context.vm.throw(class, message, code)
}
