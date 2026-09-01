//! The [`TokenKind`] enum: every lexical token, with keyword reservation levels.

use crate::token::precedence::Precedence;

/// The most permissive name position at which a keyword may still be used as
/// an identifier.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Reservation {
    /// Usable only as a member name.
    Full,
    Soft,
    Contextual,
}

impl Reservation {
    /// Whether a keyword at this level may be used as a function name.
    #[inline]
    #[must_use]
    pub const fn allows_function_name(self) -> bool {
        matches!(self, Self::Soft | Self::Contextual)
    }

    /// Whether a keyword at this level may be used as a constant name.
    #[inline]
    #[must_use]
    pub const fn allows_constant_name(self) -> bool {
        matches!(self, Self::Contextual)
    }
}

macro_rules! reservation {
    () => {
        None
    };
    ($level:ident) => {
        Some(Reservation::$level)
    };
}

macro_rules! define_token_kinds {
    ($($kind:ident => $description:literal $(, $reservation:ident)?;)*) => {
        #[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
        pub enum TokenKind {
            $($kind,)*
        }

        impl TokenKind {
            /// Every token kind in declaration order.
            pub const ALL: &[Self] = &[$(Self::$kind,)*];

            /// A user-facing description of this token kind for diagnostics.
            #[must_use]
            pub const fn describe(&self) -> &'static str {
                match self {
                    $(Self::$kind => $description,)*
                }
            }

            /// The reservation level of this token, or `None` if it is not a keyword.
            #[must_use]
            pub const fn reservation(&self) -> Option<Reservation> {
                match self {
                    $(Self::$kind => reservation!($($reservation)?),)*
                }
            }
        }
    };
}

define_token_kinds! {
    Whitespace => "whitespace";
    SingleLineComment => "a comment";
    MultiLineComment => "a comment";
    DocBlockComment => "a comment";
    Shebang => "a shebang line";
    Variable => "a variable";
    Identifier => "an identifier";
    QualifiedIdentifier => "a qualified name";
    FullyQualifiedIdentifier => "a fully-qualified name";
    LiteralInteger => "an integer literal";
    LiteralFloat => "a float literal";
    LiteralString => "a string literal";
    StringPart => "part of an interpolated string";
    True => "`true`", Full;
    False => "`false`", Full;
    Null => "`null`", Full;
    LeftParenthesis => "`(`";
    RightParenthesis => "`)`";
    LeftBracket => "`[`";
    RightBracket => "`]`";
    LeftBrace => "`{`";
    RightBrace => "`}`";
    Comma => "`,`";
    Semicolon => "`;`";
    Colon => "`:`";
    EqualGreaterThan => "`=>`";
    DotDot => "`..`";
    DotDotEqual => "`..=`";
    DotDotDot => "`...`";
    HashLeftBracket => "`#[`";
    NamespaceSeparator => "`\\`";
    Dollar => "`$`";
    At => "`@`";
    Question => "`?`";
    Equal => "`=`";
    PlusEqual => "`+=`";
    MinusEqual => "`-=`";
    AsteriskEqual => "`*=`";
    SlashEqual => "`/=`";
    PercentEqual => "`%=`";
    AsteriskAsteriskEqual => "`**=`";
    DotEqual => "`.=`";
    AmpersandEqual => "`&=`";
    PipeEqual => "`|=`";
    CaretEqual => "`^=`";
    LeftShiftEqual => "`<<=`";
    RightShiftEqual => "`>>=`";
    QuestionQuestionEqual => "`??=`";
    AmpersandAmpersandEqual => "`&&=`";
    PipePipeEqual => "`||=`";
    EqualEqual => "`==`";
    BangEqual => "`!=`";
    LessThan => "`<`";
    LessThanEqual => "`<=`";
    GreaterThan => "`>`";
    GreaterThanEqual => "`>=`";
    LessThanEqualGreaterThan => "`<=>`";
    AmpersandAmpersand => "`&&`";
    PipePipe => "`||`";
    PipeGreaterThan => "`|>`";
    Bang => "`!`";
    QuestionQuestion => "`??`";
    Ampersand => "`&`";
    Pipe => "`|`";
    Caret => "`^`";
    Tilde => "`~`";
    LeftShift => "`<<`";
    RightShift => "`>>`";
    Plus => "`+`";
    Minus => "`-`";
    Asterisk => "`*`";
    Slash => "`/`";
    Percent => "`%`";
    AsteriskAsterisk => "`**`";
    Dot => "`.`";
    PlusPlus => "`++`";
    MinusMinus => "`--`";
    MinusGreaterThan => "`->`";
    QuestionMinusGreaterThan => "`?->`";
    ColonColon => "`::`";
    ColonColonLessThan => "`::<`";
    Abstract => "`abstract`", Contextual;
    Array => "`array`", Contextual;
    As => "`as`", Soft;
    Bool => "`bool`", Contextual;
    Break => "`break`", Full;
    Case => "`case`", Contextual;
    Catch => "`catch`", Full;
    Class => "`class`", Contextual;
    Classname => "`classname`", Contextual;
    Const => "`const`", Contextual;
    Continue => "`continue`", Full;
    Default => "`default`", Contextual;
    Dict => "`dict`", Contextual;
    Do => "`do`", Full;
    Else => "`else`", Full;
    Enum => "`enum`", Contextual;
    Extends => "`extends`", Contextual;
    Final => "`final`", Contextual;
    Finally => "`finally`", Full;
    Float => "`float`", Contextual;
    Fn => "`fn`", Full;
    For => "`for`", Full;
    Foreach => "`foreach`", Full;
    Function => "`function`", Full;
    If => "`if`", Full;
    Implements => "`implements`", Contextual;
    In => "`in`", Contextual;
    Int => "`int`", Contextual;
    Interface => "`interface`", Contextual;
    Is => "`is`", Soft;
    Match => "`match`", Full;
    Mixed => "`mixed`", Contextual;
    Namespace => "`namespace`", Contextual;
    Never => "`never`", Contextual;
    New => "`new`", Full;
    Newtype => "`newtype`", Contextual;
    Object => "`object`", Contextual;
    Out => "`out`", Contextual;
    Parent => "`parent`", Full;
    Private => "`private`", Contextual;
    Protected => "`protected`", Contextual;
    Public => "`public`", Contextual;
    Readonly => "`readonly`", Contextual;
    Return => "`return`", Full;
    Self_ => "`self`", Full;
    Static => "`static`", Full;
    String => "`string`", Contextual;
    Throw => "`throw`", Full;
    Try => "`try`", Full;
    Type => "`type`", Contextual;
    Use => "`use`", Contextual;
    Using => "`using`", Full;
    Vec => "`vec`", Contextual;
    Void => "`void`", Contextual;
    While => "`while`", Full;
}

impl TokenKind {
    #[inline]
    #[must_use]
    pub const fn is_keyword(&self) -> bool {
        self.reservation().is_some()
    }

    /// Whether this token may name a member (after `->`, `?->`, `::`, or a
    /// declaring keyword). Every keyword and every plain identifier may.
    #[inline]
    #[must_use]
    pub const fn is_member_name(&self) -> bool {
        matches!(self, Self::Identifier) || self.is_keyword()
    }

    /// Whether this token may name a function.
    #[inline]
    #[must_use]
    pub const fn is_function_name(&self) -> bool {
        if matches!(self, Self::Identifier) {
            return true;
        }

        match self.reservation() {
            Some(reservation) => reservation.allows_function_name(),
            None => false,
        }
    }

    /// Whether this token may name a constant (a bare name in expression
    /// position).
    #[inline]
    #[must_use]
    pub const fn is_constant_name(&self) -> bool {
        if matches!(self, Self::Identifier) {
            return true;
        }

        match self.reservation() {
            Some(reservation) => reservation.allows_constant_name(),
            None => false,
        }
    }

    /// Whether this token can appear as an infix operator.
    #[inline]
    #[must_use]
    pub const fn is_infix(&self) -> bool {
        !matches!(Precedence::infix(self), Precedence::Lowest)
    }

    /// Whether this token can appear as a prefix (unary) operator.
    #[inline]
    #[must_use]
    pub const fn is_unary_prefix(&self) -> bool {
        !matches!(Precedence::prefix(self), Precedence::Lowest)
    }

    /// Whether this token can appear as a postfix operator.
    #[inline]
    #[must_use]
    pub const fn is_postfix(&self) -> bool {
        !matches!(Precedence::postfix(self), Precedence::Lowest)
    }

    /// Whether this token is `=` or a compound assignment operator.
    #[inline]
    #[must_use]
    pub const fn is_assignment(&self) -> bool {
        matches!(Precedence::infix(self), Precedence::Assignment)
    }

    #[inline]
    #[must_use]
    pub const fn is_visibility_modifier(&self) -> bool {
        matches!(self, Self::Public | Self::Protected | Self::Private)
    }

    #[inline]
    #[must_use]
    pub const fn is_modifier(&self) -> bool {
        if self.is_visibility_modifier() {
            return true;
        }

        matches!(
            self,
            Self::Static | Self::Final | Self::Abstract | Self::Readonly
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_literal(&self) -> bool {
        matches!(
            self,
            Self::True
                | Self::False
                | Self::Null
                | Self::LiteralFloat
                | Self::LiteralInteger
                | Self::LiteralString
        )
    }

    #[inline]
    #[must_use]
    pub const fn is_trivia(&self) -> bool {
        if self.is_comment() {
            return true;
        }

        matches!(self, Self::Whitespace | Self::Shebang)
    }

    #[inline]
    #[must_use]
    pub const fn is_comment(&self) -> bool {
        matches!(
            self,
            Self::SingleLineComment | Self::MultiLineComment | Self::DocBlockComment
        )
    }
}
