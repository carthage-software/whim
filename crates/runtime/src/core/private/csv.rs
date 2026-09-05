//! CSV encoding and strict incremental decoding.

use std::cell::OnceCell;
use std::cell::RefCell;
use std::mem;

use csv_core::ReadRecordResult;
use csv_core::ReaderBuilder;
use csv_core::Terminator;
use csv_core::WriteResult;
use csv_core::WriterBuilder;
use whim_macros::whim_class;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;

const READER: &str = "Whim\\_Private\\CSVReader";
const WRITER: &str = "Whim\\_Private\\CSVWriter";
const UNTERMINATED_QUOTED_FIELD: i64 = 1;
const INVALID_CLOSING_ENCLOSURE: i64 = 2;
const INVALID_OPENING_ENCLOSURE: i64 = 3;

type RawRecord = (Vec<u8>, Vec<usize>);
type ReadOutcome = Result<(usize, Option<RawRecord>), i64>;

#[derive(Clone, Copy, Eq, PartialEq)]
enum StrictState {
    FieldStart,
    Unquoted,
    Quoted,
    ClosingEnclosure,
    CarriageReturn,
}

struct ReaderState {
    reader: csv_core::Reader,
    output: Vec<u8>,
    ends: Vec<usize>,
    strict: StrictState,
    fresh: bool,
    delimiter: u8,
    enclosure: u8,
}

impl ReaderState {
    fn new(delimiter: u8, enclosure: u8) -> Self {
        let mut builder = ReaderBuilder::new();
        builder.delimiter(delimiter).quote(enclosure);
        Self {
            reader: builder.build(),
            output: Vec::new(),
            ends: Vec::new(),
            strict: StrictState::FieldStart,
            fresh: true,
            delimiter,
            enclosure,
        }
    }

    fn read(&mut self, input: &[u8]) -> ReadOutcome {
        if input.is_empty() && self.strict == StrictState::Quoted {
            return Err(UNTERMINATED_QUOTED_FIELD);
        }
        if let Some(length) = self.blank_record_length(input) {
            let mut output = [0; 2];
            let mut ends = [0; 2];
            let (_, read, written, ended) =
                self.reader
                    .read_record(&input[..length], &mut output, &mut ends);
            self.validate(&input[..read])?;
            self.output.extend_from_slice(&output[..written]);
            self.ends.extend_from_slice(&ends[..ended]);
            self.fresh = false;
            return Ok((read, Some((Vec::new(), vec![0]))));
        }

        let mut consumed = 0;
        let mut output = [0; 4096];
        let mut ends = [0; 128];
        loop {
            let end = if self.fresh && !input.is_empty() {
                1
            } else {
                input.len()
            };
            let (result, read, written, ended) =
                self.reader
                    .read_record(&input[consumed..end], &mut output, &mut ends);
            self.validate(&input[consumed..consumed + read])?;
            self.output.extend_from_slice(&output[..written]);
            self.ends.extend_from_slice(&ends[..ended]);
            consumed += read;
            self.fresh = false;

            match result {
                ReadRecordResult::Record => {
                    self.finish_record()?;
                    return Ok((consumed, Some(self.take_record())));
                }
                ReadRecordResult::End => return Ok((consumed, None)),
                ReadRecordResult::InputEmpty if consumed == input.len() => {
                    return Ok((consumed, None));
                }
                ReadRecordResult::InputEmpty
                | ReadRecordResult::OutputFull
                | ReadRecordResult::OutputEndsFull => {}
            }
        }
    }

    fn blank_record_length(&self, input: &[u8]) -> Option<usize> {
        if input.is_empty()
            || !matches!(
                self.strict,
                StrictState::FieldStart | StrictState::CarriageReturn
            )
        {
            return None;
        }
        let start = usize::from(self.strict == StrictState::CarriageReturn && input[0] == b'\n');
        let byte = *input.get(start)?;
        match byte {
            b'\r' if input.get(start + 1) == Some(&b'\n') => Some(start + 2),
            b'\n' | b'\r' => Some(start + 1),
            _ => None,
        }
    }

    const fn validate(&mut self, input: &[u8]) -> Result<(), i64> {
        let mut position = 0;
        while position < input.len() {
            let byte = input[position];
            match self.strict {
                StrictState::CarriageReturn => {
                    self.strict = StrictState::FieldStart;
                    if byte == b'\n' {
                        position += 1;
                    }
                }
                StrictState::FieldStart => {
                    self.strict = match byte {
                        byte if byte == self.enclosure => StrictState::Quoted,
                        b'\r' => StrictState::CarriageReturn,
                        b'\n' => StrictState::FieldStart,
                        byte if byte == self.delimiter => StrictState::FieldStart,
                        _ => StrictState::Unquoted,
                    };
                    position += 1;
                }
                StrictState::Unquoted => {
                    if byte == self.enclosure {
                        return Err(INVALID_OPENING_ENCLOSURE);
                    }
                    self.strict = match byte {
                        b'\r' => StrictState::CarriageReturn,
                        b'\n' => StrictState::FieldStart,
                        byte if byte == self.delimiter => StrictState::FieldStart,
                        _ => StrictState::Unquoted,
                    };
                    position += 1;
                }
                StrictState::Quoted => {
                    if byte == self.enclosure {
                        self.strict = StrictState::ClosingEnclosure;
                    }
                    position += 1;
                }
                StrictState::ClosingEnclosure => {
                    if byte == self.enclosure {
                        self.strict = StrictState::Quoted;
                    } else if byte == self.delimiter {
                        self.strict = StrictState::FieldStart;
                    } else if byte == b'\r' {
                        self.strict = StrictState::CarriageReturn;
                    } else if byte == b'\n' {
                        self.strict = StrictState::FieldStart;
                    } else {
                        return Err(INVALID_CLOSING_ENCLOSURE);
                    }
                    position += 1;
                }
            }
        }
        Ok(())
    }

    fn finish_record(&mut self) -> Result<(), i64> {
        if self.strict == StrictState::Quoted {
            return Err(UNTERMINATED_QUOTED_FIELD);
        }
        if self.strict != StrictState::CarriageReturn {
            self.strict = StrictState::FieldStart;
        }
        Ok(())
    }

    fn take_record(&mut self) -> RawRecord {
        (mem::take(&mut self.output), mem::take(&mut self.ends))
    }
}

#[whim_class("Whim\\_Private\\CSVReader", final)]
#[derive(Default)]
pub(crate) struct CsvReader {
    state: OnceCell<RefCell<ReaderState>>,
}

default_built_in_state!(CsvReader);

#[whim_methods]
impl CsvReader {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "create(string[1] $delimiter, string[1] $enclosure): Whim\\_Private\\CSVReader",
        static
    )]
    fn create<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let delimiter = arguments.bytes(0)[0];
        let enclosure = arguments.bytes(1)[0];
        let object = context.new_built_in_instance(READER)?;
        // SAFETY: the surrounding invariant proves this option contains a value.
        let reader = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&object),
                "a new CSV reader has built-in state",
            )
        };
        if reader
            .state
            .set(RefCell::new(ReaderState::new(delimiter, enclosure)))
            .is_err()
        {
            return Err(context.type_error("the CSV reader is already initialized"));
        }
        Ok(object)
    }

    #[whim_method("read(string $input): (int, null|vec<string>, int)")]
    fn read<'call>(context: &Context<'call, '_, '_>, arguments: Arguments<'call>) -> Value {
        let input = arguments.bytes(0);
        let receiver = context.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let reader = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a validated CSV reader has built-in state",
            )
        };
        let state =
            // SAFETY: the surrounding invariant proves this option contains a value.
            unsafe { unwrap_option_invariant(reader.state.get(), "a CSV reader is initialized") };
        let parsed = state.borrow_mut().read(input);
        let (consumed, record, error) = match parsed {
            Ok((consumed, record)) => (consumed, record, 0),
            Err(error) => (0, None, error),
        };
        let record = record.map_or_else(Value::null, |(bytes, ends)| {
            let mut start = 0;
            context.vec(ends.into_iter().map(|end| {
                let field = context.string(&bytes[start..end]);
                start = end;
                field
            }))
        });
        // SAFETY: the surrounding invariant proves this result is successful.
        let consumed = unsafe {
            unwrap_result_invariant(
                i64::try_from(consumed),
                "a Whim string length fits in an integer",
            )
        };
        context.tuple([Value::int(consumed), record, Value::int(error)])
    }
}

struct WriterState {
    writer: csv_core::Writer,
    crlf: bool,
}

#[whim_class("Whim\\_Private\\CSVWriter", final)]
#[derive(Default)]
pub(crate) struct CsvWriter {
    state: OnceCell<RefCell<WriterState>>,
}

default_built_in_state!(CsvWriter);

#[whim_methods]
impl CsvWriter {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}

    #[whim_method(
        "create(string[1] $delimiter, string[1] $enclosure, (\"\\n\"|\"\\r\\n\") $lineEnding): Whim\\_Private\\CSVWriter",
        static
    )]
    fn create<'call>(
        context: &mut Context<'call, '_, '_>,
        arguments: Arguments<'call>,
    ) -> Result<Value, Throw> {
        let delimiter = arguments.bytes(0)[0];
        let enclosure = arguments.bytes(1)[0];
        let crlf = arguments.bytes(2) == b"\r\n";
        let mut builder = WriterBuilder::new();
        builder
            .delimiter(delimiter)
            .quote(enclosure)
            .terminator(if crlf {
                Terminator::CRLF
            } else {
                Terminator::Any(b'\n')
            });
        let object = context.new_built_in_instance(WRITER)?;
        // SAFETY: the surrounding invariant proves this option contains a value.
        let writer = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&object),
                "a new CSV writer has built-in state",
            )
        };
        if writer
            .state
            .set(RefCell::new(WriterState {
                writer: builder.build(),
                crlf,
            }))
            .is_err()
        {
            return Err(context.type_error("the CSV writer is already initialized"));
        }
        Ok(object)
    }

    #[whim_method("encode(vec<string> $fields): string")]
    fn encode<'call>(context: &Context<'call, '_, '_>, arguments: Arguments<'call>) -> Value {
        let fields = arguments.vec(0);
        let receiver = context.receiver();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let writer = unsafe {
            unwrap_option_invariant(
                state_ref::<Self>(&receiver),
                "a validated CSV writer has built-in state",
            )
        };
        let state =
            // SAFETY: the surrounding invariant proves this option contains a value.
            unsafe { unwrap_option_invariant(writer.state.get(), "a CSV writer is initialized") };
        let only_empty = fields.len() == 1
            && fields
                .iter()
                .next()
                .and_then(Value::as_string_bytes)
                .is_some_and(<[u8]>::is_empty);
        let mut state = state.borrow_mut();
        if fields.is_empty() || only_empty {
            return context.string(if state.crlf { b"\r\n" } else { b"\n" });
        }

        let mut output = Vec::new();
        let mut buffer = [0; 4096];
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                loop {
                    let (result, written) = state.writer.delimiter(&mut buffer);
                    output.extend_from_slice(&buffer[..written]);
                    if result == WriteResult::InputEmpty {
                        break;
                    }
                }
            }
            // SAFETY: the surrounding invariant proves this option contains a value.
            let field = unsafe {
                unwrap_option_invariant(
                    field.as_string_bytes(),
                    "a validated CSV field vector contains strings",
                )
            };
            let mut consumed = 0;
            loop {
                let (result, read, written) = state.writer.field(&field[consumed..], &mut buffer);
                output.extend_from_slice(&buffer[..written]);
                consumed += read;
                if result == WriteResult::InputEmpty {
                    break;
                }
            }
        }
        loop {
            let (result, written) = state.writer.terminator(&mut buffer);
            output.extend_from_slice(&buffer[..written]);
            if result == WriteResult::InputEmpty {
                break;
            }
        }
        context.owned_string(output)
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::INVALID_CLOSING_ENCLOSURE;
    use super::INVALID_OPENING_ENCLOSURE;
    use super::RawRecord;
    use super::ReaderState;
    use super::UNTERMINATED_QUOTED_FIELD;

    fn fields((bytes, ends): RawRecord) -> Vec<Vec<u8>> {
        let mut start = 0;
        ends.into_iter()
            .map(|end| {
                let field = bytes[start..end].to_vec();
                start = end;
                field
            })
            .collect()
    }

    fn decode_bytewise(input: &[u8]) -> Result<Vec<Vec<Vec<u8>>>, i64> {
        let mut reader = ReaderState::new(b',', b'"');
        let mut records = Vec::new();
        for byte in input {
            let (consumed, record) = reader.read(slice::from_ref(byte))?;
            if consumed != 1 {
                return Err(-1);
            }
            if let Some(record) = record {
                records.push(fields(record));
            }
        }
        loop {
            let (_, record) = reader.read(&[])?;
            let Some(record) = record else {
                return Ok(records);
            };
            records.push(fields(record));
        }
    }

    #[test]
    fn preserves_records_across_every_byte_boundary() {
        assert_eq!(
            decode_bytewise(b"\xef\xbb\xbfa,\"b,c\"\r\n\r\nd,\"e\"\"f\""),
            Ok(vec![
                vec![b"\xef\xbb\xbfa".to_vec(), b"b,c".to_vec()],
                vec![Vec::new()],
                vec![b"d".to_vec(), b"e\"f".to_vec()],
            ])
        );
    }

    #[test]
    fn preserves_strict_quote_errors_across_chunks() {
        assert_eq!(
            decode_bytewise(b"\"unterminated"),
            Err(UNTERMINATED_QUOTED_FIELD)
        );
        assert_eq!(
            decode_bytewise(b"\"closed\"x"),
            Err(INVALID_CLOSING_ENCLOSURE)
        );
        assert_eq!(
            decode_bytewise(b"not\"start"),
            Err(INVALID_OPENING_ENCLOSURE)
        );
    }
}
