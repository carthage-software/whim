use crate::token::kind::TokenKind;

#[inline(always)]
#[must_use]
pub(in crate::lexer) fn lookup(bytes: &[u8]) -> Option<TokenKind> {
    match bytes.len() {
        2 => lookup_len2(bytes),
        3 => lookup_len3(bytes),
        4 => lookup_len4(bytes),
        5 => lookup_len5(bytes),
        6 => lookup_len6(bytes),
        7 => lookup_len7(bytes),
        8 => lookup_len8(bytes),
        9 => lookup_len9(bytes),
        10 => lookup_len10(bytes),
        _ => None,
    }
}

#[inline]
fn lookup_len2(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'a' if bytes == b"as" => Some(TokenKind::As),
        b'd' if bytes == b"do" => Some(TokenKind::Do),
        b'f' if bytes == b"fn" => Some(TokenKind::Fn),
        b'i' => match bytes[1] {
            b'f' => Some(TokenKind::If),
            b'n' => Some(TokenKind::In),
            b's' => Some(TokenKind::Is),
            _ => None,
        },
        _ => None,
    }
}

#[inline]
fn lookup_len3(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'f' if bytes == b"for" => Some(TokenKind::For),
        b'i' if bytes == b"int" => Some(TokenKind::Int),
        b'n' if bytes == b"new" => Some(TokenKind::New),
        b'o' if bytes == b"out" => Some(TokenKind::Out),
        b't' if bytes == b"try" => Some(TokenKind::Try),
        b'u' if bytes == b"use" => Some(TokenKind::Use),
        b'v' if bytes == b"vec" => Some(TokenKind::Vec),
        _ => None,
    }
}

#[inline]
fn lookup_len4(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'b' if bytes == b"bool" => Some(TokenKind::Bool),
        b'c' if bytes == b"case" => Some(TokenKind::Case),
        b'd' if bytes == b"dict" => Some(TokenKind::Dict),
        b'e' => match bytes[1] {
            b'l' if bytes == b"else" => Some(TokenKind::Else),
            b'n' if bytes == b"enum" => Some(TokenKind::Enum),
            _ => None,
        },
        b'n' if bytes == b"null" => Some(TokenKind::Null),
        b's' if bytes == b"self" => Some(TokenKind::Self_),
        b't' => match bytes[1] {
            b'r' if bytes == b"true" => Some(TokenKind::True),
            b'y' if bytes == b"type" => Some(TokenKind::Type),
            _ => None,
        },
        b'v' if bytes == b"void" => Some(TokenKind::Void),
        _ => None,
    }
}

#[inline]
fn lookup_len5(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'a' if bytes == b"array" => Some(TokenKind::Array),
        b'b' if bytes == b"break" => Some(TokenKind::Break),
        b'c' => match bytes[1] {
            b'a' if bytes == b"catch" => Some(TokenKind::Catch),
            b'l' if bytes == b"class" => Some(TokenKind::Class),
            b'o' if bytes == b"const" => Some(TokenKind::Const),
            _ => None,
        },
        b'f' => match bytes[1] {
            b'a' if bytes == b"false" => Some(TokenKind::False),
            b'i' if bytes == b"final" => Some(TokenKind::Final),
            b'l' if bytes == b"float" => Some(TokenKind::Float),
            _ => None,
        },
        b'm' => match bytes[1] {
            b'a' if bytes == b"match" => Some(TokenKind::Match),
            b'i' if bytes == b"mixed" => Some(TokenKind::Mixed),
            _ => None,
        },
        b'n' if bytes == b"never" => Some(TokenKind::Never),
        b't' if bytes == b"throw" => Some(TokenKind::Throw),
        b'u' if bytes == b"using" => Some(TokenKind::Using),
        b'w' if bytes == b"while" => Some(TokenKind::While),
        _ => None,
    }
}

#[inline]
fn lookup_len6(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'o' if bytes == b"object" => Some(TokenKind::Object),
        b'p' => match bytes[1] {
            b'a' if bytes == b"parent" => Some(TokenKind::Parent),
            b'u' if bytes == b"public" => Some(TokenKind::Public),
            _ => None,
        },
        b'r' if bytes == b"return" => Some(TokenKind::Return),
        b's' if bytes == b"static" => Some(TokenKind::Static),
        b's' if bytes == b"string" => Some(TokenKind::String),
        _ => None,
    }
}

#[inline]
fn lookup_len7(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'n' if bytes == b"newtype" => Some(TokenKind::Newtype),
        b'd' if bytes == b"default" => Some(TokenKind::Default),
        b'e' if bytes == b"extends" => Some(TokenKind::Extends),
        b'f' => match bytes[1] {
            b'i' if bytes == b"finally" => Some(TokenKind::Finally),
            b'o' if bytes == b"foreach" => Some(TokenKind::Foreach),
            _ => None,
        },
        b'p' if bytes == b"private" => Some(TokenKind::Private),
        _ => None,
    }
}

#[inline]
fn lookup_len8(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'a' if bytes == b"abstract" => Some(TokenKind::Abstract),
        b'c' if bytes == b"continue" => Some(TokenKind::Continue),
        b'f' if bytes == b"function" => Some(TokenKind::Function),
        b'r' if bytes == b"readonly" => Some(TokenKind::Readonly),
        _ => None,
    }
}

#[inline]
fn lookup_len9(bytes: &[u8]) -> Option<TokenKind> {
    match bytes[0] {
        b'c' if bytes == b"classname" => Some(TokenKind::Classname),
        b'i' if bytes == b"interface" => Some(TokenKind::Interface),
        b'n' if bytes == b"namespace" => Some(TokenKind::Namespace),
        b'p' if bytes == b"protected" => Some(TokenKind::Protected),
        _ => None,
    }
}

#[inline]
fn lookup_len10(bytes: &[u8]) -> Option<TokenKind> {
    if bytes == b"implements" {
        Some(TokenKind::Implements)
    } else {
        None
    }
}
