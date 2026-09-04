//! The parser for runtime type-name spellings.

use std::str;

use crate::unwrap_option_invariant;
use crate::vm::types::FunctionTypeDescriptor;
use crate::vm::types::FunctionTypeParameterDescriptor;
use crate::vm::types::Heap;
use crate::vm::types::TypeDescriptor;
use crate::vm::types::environment::descriptor_same;

enum ParsedArguments {
    Absent,
    Present(Vec<TypeDescriptor>),
}

impl ParsedArguments {
    fn into_option(self) -> Option<Vec<TypeDescriptor>> {
        match self {
            Self::Absent => None,
            Self::Present(arguments) => Some(arguments),
        }
    }
}

/// A small byte-oriented parser for the runtime type vocabulary.
pub(in crate::vm) struct RuntimeTypeParser<'bytes, 'heap> {
    heap: &'heap Heap,
    bytes: &'bytes [u8],
    position: usize,
}

impl<'bytes, 'heap> RuntimeTypeParser<'bytes, 'heap> {
    pub(in crate::vm::types) fn new(heap: &'heap Heap, bytes: &'bytes [u8]) -> Self {
        Self {
            heap,
            bytes,
            position: 0,
        }
    }

    pub(in crate::vm::types) fn parse(mut self) -> Option<TypeDescriptor> {
        let descriptor = self.parse_union()?;
        self.skip_space();
        (self.position == self.bytes.len()).then_some(descriptor)
    }

    fn parse_union(&mut self) -> Option<TypeDescriptor> {
        let mut members = vec![self.parse_intersection()?];
        while self.consume(b'|') {
            members.push(self.parse_intersection()?);
        }
        if members.len() == 1 {
            members.pop()
        } else {
            Some(TypeDescriptor::Union(members))
        }
    }

    fn parse_intersection(&mut self) -> Option<TypeDescriptor> {
        let mut members = vec![self.parse_negated()?];
        while self.consume(b'&') {
            members.push(self.parse_negated()?);
        }
        if members.len() == 1 {
            members.pop()
        } else {
            Some(TypeDescriptor::intersection(members))
        }
    }

    fn parse_negated(&mut self) -> Option<TypeDescriptor> {
        if !self.consume(b'!') {
            return self.parse_atom();
        }
        let inner = self.parse_negated()?;
        if matches!(inner, TypeDescriptor::Void | TypeDescriptor::Wildcard) {
            return None;
        }
        Some(TypeDescriptor::Negated(Box::new(inner)))
    }

    fn parse_atom(&mut self) -> Option<TypeDescriptor> {
        self.skip_space();
        if self.consume(b'(') {
            if self.consume(b')') {
                return Some(TypeDescriptor::Tuple(Vec::new()));
            }
            if self.consume_ellipsis() {
                let rest = if self.consume(b')') {
                    TypeDescriptor::Mixed
                } else {
                    let rest = self.parse_union()?;
                    self.expect(b')')?;
                    rest
                };
                return Some(TypeDescriptor::TupleRest {
                    elements: Vec::new(),
                    rest: Box::new(rest),
                });
            }
            let first = self.parse_union()?;
            if !self.consume(b',') {
                self.expect(b')')?;
                return Some(first);
            }
            let mut members = vec![first];
            if self.consume(b')') {
                return Some(TypeDescriptor::Tuple(members));
            }
            loop {
                if self.consume_ellipsis() {
                    let rest = if self.consume(b')') {
                        TypeDescriptor::Mixed
                    } else {
                        let rest = self.parse_union()?;
                        self.expect(b')')?;
                        rest
                    };
                    return Some(TypeDescriptor::TupleRest {
                        elements: members,
                        rest: Box::new(rest),
                    });
                }
                members.push(self.parse_union()?);
                if self.consume(b')') {
                    break;
                }
                self.expect(b',')?;
            }
            return Some(TypeDescriptor::Tuple(members));
        }
        if self.peek() == Some(b'\'') {
            return self.parse_string_literal();
        }
        if let Some(range) = self.parse_integer_range() {
            return Some(range);
        }

        let token = self.take_token()?;
        let descriptor = match token {
            b"_" => TypeDescriptor::Wildcard,
            b"mixed" => TypeDescriptor::Mixed,
            b"void" => TypeDescriptor::Void,
            b"never" => TypeDescriptor::Never,
            b"null" => TypeDescriptor::Null,
            b"bool" => TypeDescriptor::Bool,
            b"int" => TypeDescriptor::Int,
            b"float" => TypeDescriptor::Float,
            b"string" => {
                if self.consume(b'[') {
                    self.parse_string_length()?
                } else {
                    TypeDescriptor::String
                }
            }
            b"object" => TypeDescriptor::Object,
            b"true" => TypeDescriptor::TrueLiteral,
            b"false" => TypeDescriptor::FalseLiteral,
            b"static" => TypeDescriptor::StaticClass,
            b"tuple" => TypeDescriptor::TupleAny,
            b"array" => match self.parse_arguments()? {
                ParsedArguments::Absent => TypeDescriptor::Array(None),
                ParsedArguments::Present(arguments) if arguments.len() == 2 => {
                    let mut arguments = arguments.into_iter();
                    TypeDescriptor::Array(Some((
                        Box::new(arguments.next()?),
                        Box::new(arguments.next()?),
                    )))
                }
                ParsedArguments::Present(_) => return None,
            },
            b"vec" => match self.parse_arguments()? {
                ParsedArguments::Absent => TypeDescriptor::Vector(None),
                ParsedArguments::Present(mut arguments) if arguments.len() == 1 => {
                    TypeDescriptor::Vector(Some(Box::new(arguments.pop()?)))
                }
                ParsedArguments::Present(_) => return None,
            },
            b"dict" => match self.parse_arguments()? {
                ParsedArguments::Absent => TypeDescriptor::Dictionary(None),
                ParsedArguments::Present(arguments) if arguments.len() == 2 => {
                    let mut arguments = arguments.into_iter();
                    TypeDescriptor::Dictionary(Some((
                        Box::new(arguments.next()?),
                        Box::new(arguments.next()?),
                    )))
                }
                ParsedArguments::Present(_) => return None,
            },
            b"classname" => {
                let ParsedArguments::Present(mut arguments) = self.parse_arguments()? else {
                    return None;
                };
                if arguments.len() != 1 {
                    return None;
                }
                TypeDescriptor::Classname(Box::new(arguments.pop()?))
            }
            b"fn" => return self.parse_callable(),
            bytes => {
                let text = str::from_utf8(bytes).ok()?;
                if let Ok(value) = text.parse::<i64>() {
                    TypeDescriptor::IntLiteral(value)
                } else if (text.contains('.') || text.contains('e') || text.contains('E'))
                    && let Ok(value) = text.parse::<f64>()
                {
                    TypeDescriptor::FloatLiteral(value)
                } else {
                    let arguments = self.parse_arguments()?.into_option();
                    let name = bytes.strip_prefix(b"\\").unwrap_or(bytes);
                    TypeDescriptor::Named {
                        name: self.heap.intern(name),
                        arguments,
                        recursive: false,
                    }
                }
            }
        };
        Some(descriptor)
    }

    fn parse_callable(&mut self) -> Option<TypeDescriptor> {
        if !self.consume(b'(') {
            return Some(TypeDescriptor::Callable(None));
        }
        let mut parameters = Vec::new();
        if !self.consume(b')') {
            loop {
                let optional = self.consume(b'=');
                parameters.push(FunctionTypeParameterDescriptor {
                    r#type: self.parse_union()?,
                    optional,
                });
                if self.consume(b')') {
                    break;
                }
                self.expect(b',')?;
            }
        }
        self.expect(b':')?;
        Some(TypeDescriptor::Callable(Some(FunctionTypeDescriptor {
            parameters,
            return_type: Box::new(self.parse_union()?),
        })))
    }

    fn parse_arguments(&mut self) -> Option<ParsedArguments> {
        if !self.consume(b'<') {
            return Some(ParsedArguments::Absent);
        }
        let mut arguments = Vec::new();
        if self.consume(b'>') {
            return Some(ParsedArguments::Present(arguments));
        }
        loop {
            arguments.push(self.parse_union()?);
            if self.consume(b'>') {
                break;
            }
            self.expect(b',')?;
        }
        Some(ParsedArguments::Present(arguments))
    }

    fn parse_string_literal(&mut self) -> Option<TypeDescriptor> {
        self.expect(b'\'')?;
        let mut value = Vec::new();
        loop {
            let byte = self.next()?;
            match byte {
                b'\'' => break,
                b'\\' => value.push(self.next()?),
                other => value.push(other),
            }
        }
        Some(TypeDescriptor::StringLiteral(self.heap.intern(&value)))
    }

    fn parse_integer_range(&mut self) -> Option<TypeDescriptor> {
        let start = self.position;
        let lower = self.parse_integer_endpoint();
        self.skip_space();
        let inclusive = if self.bytes.get(self.position..self.position + 3) == Some(b"..=") {
            self.position += 3;
            true
        } else if self.bytes.get(self.position..self.position + 2) == Some(b"..") {
            self.position += 2;
            false
        } else {
            self.position = start;
            return None;
        };
        let upper = self.parse_integer_endpoint();
        if lower.is_none() && upper.is_none() {
            self.position = start;
            return None;
        }
        let max = if inclusive {
            upper
        } else if let Some(upper) = upper {
            let Some(max) = upper.checked_sub(1) else {
                return Some(TypeDescriptor::Never);
            };
            Some(max)
        } else {
            None
        };
        Some(TypeDescriptor::integer_range(lower, max))
    }

    fn parse_string_length(&mut self) -> Option<TypeDescriptor> {
        let min = self.parse_nonnegative_integer_endpoint();
        if self.consume(b']') {
            let length = min?;
            return Some(TypeDescriptor::string_length(
                self.heap,
                length,
                Some(length),
            ));
        }

        self.skip_space();
        let inclusive = if self.bytes.get(self.position..self.position + 3) == Some(b"..=") {
            self.position += 3;
            true
        } else if self.bytes.get(self.position..self.position + 2) == Some(b"..") {
            self.position += 2;
            false
        } else {
            return None;
        };
        let upper = self.parse_nonnegative_integer_endpoint();
        self.expect(b']')?;
        if min.is_none() && upper.is_none() {
            return None;
        }
        let min = min.unwrap_or(0);
        let max = if inclusive {
            upper
        } else if let Some(upper) = upper {
            let Some(max) = upper.checked_sub(1) else {
                return Some(TypeDescriptor::Never);
            };
            Some(max)
        } else {
            None
        };
        Some(TypeDescriptor::string_length(self.heap, min, max))
    }

    fn parse_nonnegative_integer_endpoint(&mut self) -> Option<i64> {
        self.skip_space();
        let start = self.position;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.position += 1;
        }
        if self.position == start {
            return None;
        }
        str::from_utf8(&self.bytes[start..self.position])
            .ok()?
            .parse()
            .ok()
    }

    fn parse_integer_endpoint(&mut self) -> Option<i64> {
        self.skip_space();
        let start = self.position;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.position += 1;
            self.skip_space();
        }
        let digits = self.position;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.position += 1;
        }
        if self.position == digits {
            self.position = start;
            return None;
        }
        let magnitude = str::from_utf8(&self.bytes[digits..self.position])
            .ok()?
            .parse::<u64>()
            .ok()?;
        if !negative {
            return i64::try_from(magnitude).ok();
        }
        if magnitude == (i64::MAX as u64) + 1 {
            return Some(i64::MIN);
        }
        i64::try_from(magnitude).ok().map(|value| -value)
    }

    fn consume_ellipsis(&mut self) -> bool {
        self.skip_space();
        if self.bytes.get(self.position..self.position + 3) != Some(b"...") {
            return false;
        }
        self.position += 3;
        true
    }

    fn take_token(&mut self) -> Option<&'bytes [u8]> {
        self.skip_space();
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace()
                || matches!(
                    byte,
                    b'<' | b'>'
                        | b','
                        | b'('
                        | b')'
                        | b'['
                        | b']'
                        | b':'
                        | b'|'
                        | b'&'
                        | b'!'
                        | b'='
                )
            {
                break;
            }
            self.position += 1;
        }
        (self.position > start).then_some(&self.bytes[start..self.position])
    }

    fn consume(&mut self, expected: u8) -> bool {
        self.skip_space();
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: u8) -> Option<()> {
        self.consume(expected).then_some(())
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }
}

pub(in crate::vm) fn push_runtime_union_member(
    members: &mut Vec<TypeDescriptor>,
    member: TypeDescriptor,
) {
    if let TypeDescriptor::Union(nested) = member {
        for member in nested {
            push_runtime_union_member(members, member);
        }
        return;
    }
    if !members
        .iter()
        .any(|existing| descriptor_same(existing, &member))
    {
        members.push(member);
    }
}

pub(in crate::vm) fn runtime_union(mut members: Vec<TypeDescriptor>) -> TypeDescriptor {
    match members.len() {
        0 => TypeDescriptor::Never,
        // SAFETY: the match arm proves the vector holds exactly one member.
        1 => unsafe { unwrap_option_invariant(members.pop(), "the length was checked") },
        _ => TypeDescriptor::Union(members),
    }
}
