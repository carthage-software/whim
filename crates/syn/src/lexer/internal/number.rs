use crate::float_exponent;
use crate::float_separator;
use crate::input::Input;
use crate::number_sign;
use crate::start_of_binary_number;
use crate::start_of_hexadecimal_number;
use crate::start_of_octal_number;
use crate::token::kind::TokenKind;
use crate::utils::read_digits_of_base;

#[inline]
#[must_use]
pub(in crate::lexer) fn scan(input: &Input<'_>) -> (TokenKind, usize) {
    let mut length = 1;
    let base: u8 = match input.read(3) {
        start_of_binary_number!() => {
            length += 1;

            2
        }
        start_of_octal_number!() => {
            length += 1;

            8
        }
        start_of_hexadecimal_number!() => {
            length += 1;

            16
        }
        _ => 10,
    };

    length = read_digits_of_base(input, length, base);

    if base != 10 {
        return (TokenKind::LiteralInteger, length);
    }

    if matches!(input.peek(length, 2), [b'.', b'.']) {
        return (TokenKind::LiteralInteger, length);
    }

    if !matches!(input.peek(length, 3), float_separator!()) {
        return (TokenKind::LiteralInteger, length);
    }

    if let [b'.'] = input.peek(length, 1) {
        length += 1;
        if matches!(input.peek(length, 1), [b'0'..=b'9']) {
            length = read_digits_of_base(input, length, 10);
        }
    }

    (TokenKind::LiteralFloat, scan_exponent(input, length))
}

#[inline]
#[must_use]
pub(in crate::lexer) fn scan_leading_dot_float(input: &Input<'_>) -> usize {
    let length = read_digits_of_base(input, 2, 10);

    scan_exponent(input, length)
}

/// Extends `length` past a trailing exponent (`e5`, `E+5`, `e-3`) if one is
/// present; the exponent marker is only included when digits follow it.
#[inline]
fn scan_exponent(input: &Input<'_>, mut length: usize) -> usize {
    if let float_exponent!() = input.peek(length, 1) {
        let mut exponent_length = length + 1;
        if let number_sign!() = input.peek(exponent_length, 1) {
            exponent_length += 1;
        }

        if matches!(input.peek(exponent_length, 1), [b'0'..=b'9']) {
            let after_exponent = read_digits_of_base(input, exponent_length, 10);
            if after_exponent > exponent_length {
                length = after_exponent;
            }
        }
    }

    length
}
