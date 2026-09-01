//! The roots of the `Whim\Unwind` hierarchy.

use whim_macros::whim_class;
use whim_macros::whim_interface;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::value::Value;

#[whim_interface("Whim\\Unwind\\Throwable")]
#[whim_permits("Whim\\Unwind\\Error", "Whim\\Unwind\\Exception")]
trait ThrowableProtocol {}

macro_rules! throwable_root {
    ($identifier:ident, $name:literal) => {
        #[whim_class($name)]
        #[whim_implements("Whim\\Unwind\\Throwable")]
        #[whim_property("protected string $message")]
        #[whim_property("protected int $code")]
        #[whim_property("protected string $file")]
        #[whim_property("protected int $line")]
        #[whim_property("protected vec<Whim\\Unwind\\TraceFrame> $trace")]
        #[whim_property("protected null|Whim\\Unwind\\Throwable $previous")]
        struct $identifier;

        #[whim_methods]
        impl $identifier {
            #[whim_method(
                "__construct(string $message, int $code = 0, null|Whim\\Unwind\\Throwable $previous = null): void"
            )]
            fn construct(
                cx: &mut Context<'_, '_, '_>,
                arguments: Arguments<'_>,
            ) -> Result<Value, Throw> {
                let this = cx.receiver();
                let message = arguments.local(0);
                cx.set_property(&this, "message", message)?;
                let code = arguments.optional_int(1).unwrap_or(0);
                let code = Value::int(code);
                cx.set_property(&this, "code", code)?;
                let previous = match arguments.optional_instance(2) {
                    Some(previous) => Value::object(previous),
                    None => Value::null(),
                };
                cx.set_property(&this, "previous", previous)?;
                let (file, line) = cx.vm.current_location();
                cx.set_property(&this, "file", file)?;
                cx.set_property(&this, "line", line)?;
                let trace = cx.vm.capture_trace();
                cx.set_property(&this, "trace", trace)?;
                Ok(Value::null())
            }

            #[whim_method("getMessage(): string")]
            fn get_message(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
                let this = cx.receiver();
                cx.get_property(&this, "message")
            }

            #[whim_method("getCode(): int")]
            fn get_code(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
                let this = cx.receiver();
                cx.get_property(&this, "code")
            }

            #[whim_method("getFile(): string")]
            fn get_file(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
                let this = cx.receiver();
                cx.get_property(&this, "file")
            }

            #[whim_method("getLine(): int")]
            fn get_line(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
                let this = cx.receiver();
                cx.get_property(&this, "line")
            }

            #[whim_method("getTrace(): vec<Whim\\Unwind\\TraceFrame>")]
            fn get_trace(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
                let this = cx.receiver();
                cx.get_property(&this, "trace")
            }

            #[whim_method("getPrevious(): null|Whim\\Unwind\\Throwable")]
            fn get_previous(cx: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
                let this = cx.receiver();
                cx.get_property(&this, "previous")
            }

            #[whim_method("toString(): string")]
            fn to_string(cx: &mut Context<'_, '_, '_>) -> Value {
                let this = cx.receiver();
                let rendered = cx.vm.engine.render_error(&this);
                cx.string(rendered.as_bytes())
            }
        }
    };
}

throwable_root!(Error, "Whim\\Unwind\\Error");
throwable_root!(Exception, "Whim\\Unwind\\Exception");

#[whim_class("Whim\\Unwind\\TraceFrame", final)]
#[whim_property("public readonly string $function")]
#[whim_property("public readonly string $file")]
#[whim_property("public readonly int $line")]
#[whim_property("public readonly vec<mixed> $arguments")]
struct TraceFrame;

#[whim_methods]
impl TraceFrame {
    #[whim_method(
        "__construct(string $function, string $file, int $line, vec<mixed> $arguments): void"
    )]
    fn construct(cx: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
        let this = cx.receiver();
        let function = arguments.local(0);
        cx.set_property(&this, "function", function)?;
        let file = arguments.local(1);
        cx.set_property(&this, "file", file)?;
        let line = arguments.local(2);
        cx.set_property(&this, "line", line)?;
        let frame_arguments = arguments.local(3);
        cx.set_property(&this, "arguments", frame_arguments)?;
        Ok(Value::null())
    }
}
