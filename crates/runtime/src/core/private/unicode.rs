//! Unicode character properties used by the Whim standard library.

use std::str::from_utf8;

use caseless::Caseless;
use unicode_general_category::GeneralCategory;
use unicode_general_category::get_general_category;
use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::unreachable_invariant;
use crate::value::Value;

const REPLACEMENT_CHARACTER: u32 = 0xfffd;

#[whim_function("Whim\\_Private\\unicode_has_property(int $codePoint, 0..=12 $property): bool")]
pub(crate) fn has_property(arguments: Arguments<'_>) -> Value {
    let Ok(code_point) = u32::try_from(arguments.int(0)) else {
        return Value::bool(false);
    };
    let Some(character) = char::from_u32(code_point) else {
        return Value::bool(false);
    };

    let property = arguments.int(1);
    let result = match property {
        0 => true,
        1 => character.is_whitespace(),
        2 => is_letter(get_general_category(character)),
        3 => is_mark(get_general_category(character)),
        4 => is_number(get_general_category(character)),
        5 => get_general_category(character) == GeneralCategory::DecimalNumber,
        6 => {
            let category = get_general_category(character);
            is_letter(category) || is_number(category)
        }
        7 => is_punctuation(get_general_category(character)),
        8 => is_symbol(get_general_category(character)),
        9 => is_separator(get_general_category(character)),
        10 => get_general_category(character) == GeneralCategory::Control,
        11 => character.is_uppercase(),
        12 => character.is_lowercase(),
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("a validated Unicode property is known") },
    };

    Value::bool(result)
}

#[whim_function("Whim\\_Private\\unicode_case_fold(string $value): null|string")]
pub(crate) fn case_fold(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let Ok(value) = from_utf8(arguments.bytes(0)) else {
        return Value::null();
    };

    let mut folded = String::with_capacity(value.len());
    folded.extend(value.chars().default_case_fold());
    context.string(folded.as_bytes())
}

#[whim_function(
    "Whim\\_Private\\unicode_code_point_at(string $bytes, int $offset): null|(0..=0xd7ff)|(0xe000..=0x10ffff)"
)]
pub(crate) fn code_point_at(arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let Ok(offset) = usize::try_from(arguments.int(1)) else {
        return Value::null();
    };
    let Some(bytes) = bytes.get(offset..) else {
        return Value::null();
    };
    if bytes.is_empty() {
        return Value::null();
    }

    let code_point =
        decode_code_point(bytes).map_or(REPLACEMENT_CHARACTER, |(code_point, _)| code_point);
    Value::int(i64::from(code_point))
}

#[whim_function(
    "Whim\\_Private\\unicode_code_point_before(string $bytes, int $offset): null|(0..=0xd7ff)|(0xe000..=0x10ffff)"
)]
pub(crate) fn code_point_before(arguments: Arguments<'_>) -> Value {
    let bytes = arguments.bytes(0);
    let Ok(end) = usize::try_from(arguments.int(1)) else {
        return Value::null();
    };
    if end == 0 || end > bytes.len() {
        return Value::null();
    }

    let lower_bound = end.saturating_sub(4);
    let mut start = end - 1;
    while start > lower_bound && is_continuation(bytes[start]) {
        start -= 1;
    }

    let code_point = match decode_code_point(&bytes[start..end]) {
        Some((code_point, width)) if width == end - start => code_point,
        _ => REPLACEMENT_CHARACTER,
    };
    Value::int(i64::from(code_point))
}

#[whim_function("Whim\\_Private\\unicode_from_code_point(int $codePoint): (string&!'')")]
pub(crate) fn from_code_point(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let code_point = u32::try_from(arguments.int(0))
        .ok()
        .and_then(char::from_u32)
        .unwrap_or(char::REPLACEMENT_CHARACTER);
    let mut buffer = [0; 4];
    let encoded = code_point.encode_utf8(&mut buffer);

    context.string(encoded.as_bytes())
}

#[inline]
fn decode_code_point(bytes: &[u8]) -> Option<(u32, usize)> {
    match *bytes {
        [first @ 0x00..=0x7f, ..] => Some((u32::from(first), 1)),
        [first @ 0xc2..=0xdf, second @ 0x80..=0xbf, ..] => {
            Some(((u32::from(first & 0x1f) << 6) | u32::from(second & 0x3f), 2))
        }
        [
            first @ 0xe0..=0xef,
            second @ 0x80..=0xbf,
            third @ 0x80..=0xbf,
            ..,
        ] if (first != 0xe0 || second >= 0xa0) && (first != 0xed || second < 0xa0) => Some((
            (u32::from(first & 0x0f) << 12)
                | (u32::from(second & 0x3f) << 6)
                | u32::from(third & 0x3f),
            3,
        )),
        [
            first @ 0xf0..=0xf4,
            second @ 0x80..=0xbf,
            third @ 0x80..=0xbf,
            fourth @ 0x80..=0xbf,
            ..,
        ] if (first != 0xf0 || second >= 0x90) && (first != 0xf4 || second < 0x90) => Some((
            (u32::from(first & 0x07) << 18)
                | (u32::from(second & 0x3f) << 12)
                | (u32::from(third & 0x3f) << 6)
                | u32::from(fourth & 0x3f),
            4,
        )),
        _ => None,
    }
}

const fn is_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

const fn is_letter(category: GeneralCategory) -> bool {
    matches!(
        category,
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    )
}

const fn is_mark(category: GeneralCategory) -> bool {
    matches!(
        category,
        GeneralCategory::NonspacingMark
            | GeneralCategory::SpacingMark
            | GeneralCategory::EnclosingMark
    )
}

const fn is_number(category: GeneralCategory) -> bool {
    matches!(
        category,
        GeneralCategory::DecimalNumber
            | GeneralCategory::LetterNumber
            | GeneralCategory::OtherNumber
    )
}

const fn is_punctuation(category: GeneralCategory) -> bool {
    matches!(
        category,
        GeneralCategory::ConnectorPunctuation
            | GeneralCategory::DashPunctuation
            | GeneralCategory::OpenPunctuation
            | GeneralCategory::ClosePunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::OtherPunctuation
    )
}

const fn is_symbol(category: GeneralCategory) -> bool {
    matches!(
        category,
        GeneralCategory::MathSymbol
            | GeneralCategory::CurrencySymbol
            | GeneralCategory::ModifierSymbol
            | GeneralCategory::OtherSymbol
    )
}

const fn is_separator(category: GeneralCategory) -> bool {
    matches!(
        category,
        GeneralCategory::SpaceSeparator
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
    )
}
