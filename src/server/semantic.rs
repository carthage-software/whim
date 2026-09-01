use std::collections::BTreeMap;

use lsp_types::SemanticToken;
use lsp_types::SemanticTokenModifier;
use lsp_types::SemanticTokenType;
use lsp_types::SemanticTokens;
use lsp_types::SemanticTokensLegend;
use whim_syn::cst::node::NodeKind;
use whim_syn::token::kind::TokenKind;

use crate::server::analysis::Analysis;
use crate::server::analysis::is_identifier;

const NAMESPACE: u32 = 0;
const TYPE: u32 = 1;
const TYPE_PARAMETER: u32 = 2;
const PARAMETER: u32 = 3;
const VARIABLE: u32 = 4;
const PROPERTY: u32 = 5;
const ENUM_MEMBER: u32 = 6;
const FUNCTION: u32 = 7;
const METHOD: u32 = 8;
const KEYWORD: u32 = 9;
const COMMENT: u32 = 10;
const STRING: u32 = 11;
const NUMBER: u32 = 12;
const OPERATOR: u32 = 13;
const DECORATOR: u32 = 14;
const MACRO: u32 = 15;

const NONE: u32 = 0;
const DECLARATION: u32 = 1 << 0;
const READONLY: u32 = 1 << 1;
const STATIC: u32 = 1 << 2;
const ABSTRACT: u32 = 1 << 3;
const FINAL: u32 = 1 << 4;

#[derive(Clone, Copy)]
struct Marked {
    end: usize,
    kind: u32,
    modifiers: u32,
}

pub(super) fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::NAMESPACE,
            SemanticTokenType::TYPE,
            SemanticTokenType::TYPE_PARAMETER,
            SemanticTokenType::PARAMETER,
            SemanticTokenType::VARIABLE,
            SemanticTokenType::PROPERTY,
            SemanticTokenType::ENUM_MEMBER,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::METHOD,
            SemanticTokenType::KEYWORD,
            SemanticTokenType::COMMENT,
            SemanticTokenType::STRING,
            SemanticTokenType::NUMBER,
            SemanticTokenType::OPERATOR,
            SemanticTokenType::DECORATOR,
            SemanticTokenType::MACRO,
        ],
        token_modifiers: vec![
            SemanticTokenModifier::DECLARATION,
            SemanticTokenModifier::READONLY,
            SemanticTokenModifier::STATIC,
            SemanticTokenModifier::ABSTRACT,
            SemanticTokenModifier::new("final"),
        ],
    }
}

pub(super) fn tokens(analysis: &Analysis<'_>) -> SemanticTokens {
    let mut marked = BTreeMap::new();
    mark_tree(analysis, &mut marked);
    mark_lexical(analysis, &mut marked);

    SemanticTokens {
        result_id: None,
        data: encode(analysis, &marked),
    }
}

fn mark_tree(analysis: &Analysis<'_>, marked: &mut BTreeMap<usize, Marked>) {
    let elements = analysis.elements();
    let mut ancestors: Vec<usize> = Vec::new();

    for (index, element) in elements.iter().enumerate() {
        while ancestors
            .last()
            .is_some_and(|parent| elements[*parent].depth >= element.depth)
        {
            ancestors.pop();
        }

        let named = is_identifier(element.kind)
            || (element.kind == NodeKind::Variable
                && ancestors.last().is_some_and(|parent| {
                    matches!(
                        elements[*parent].kind,
                        NodeKind::Property | NodeKind::Parameter | NodeKind::FunctionTypeParameter
                    )
                }));

        if named {
            let (kind, mut modifiers) = role(analysis, &ancestors, element.start);
            if modifiers & DECLARATION != 0 {
                modifiers |= declared_modifiers(analysis, &ancestors, element.start);
            }

            mark_name(
                analysis,
                marked,
                element.start,
                element.end,
                kind,
                modifiers,
            );
        }

        ancestors.push(index);
    }
}

fn mark_name(
    analysis: &Analysis<'_>,
    marked: &mut BTreeMap<usize, Marked>,
    start: usize,
    end: usize,
    kind: u32,
    modifiers: u32,
) {
    if start >= end || end > analysis.source().len() {
        return;
    }

    let text = &analysis.source()[start..end];
    if let Some(separator) = text.rfind('\\') {
        let boundary = start + separator + 1;
        insert(marked, start, boundary, NAMESPACE, NONE);
        if boundary < end {
            insert(marked, boundary, end, kind, modifiers);
        }
    } else {
        insert(marked, start, end, kind, modifiers);
    }
}

fn insert(
    marked: &mut BTreeMap<usize, Marked>,
    start: usize,
    end: usize,
    kind: u32,
    modifiers: u32,
) {
    marked.entry(start).or_insert(Marked {
        end,
        kind,
        modifiers,
    });
}

fn role(analysis: &Analysis<'_>, ancestors: &[usize], start: usize) -> (u32, u32) {
    let elements = analysis.elements();
    for parent in ancestors.iter().rev() {
        let role = match elements[*parent].kind {
            NodeKind::Attribute => (DECORATOR, NONE),
            kind if is_construct(kind) => (MACRO, NONE),
            NodeKind::Pattern => {
                if analysis.follows_member_operator(start) {
                    (ENUM_MEMBER, NONE)
                } else {
                    (TYPE, NONE)
                }
            }
            NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Enum
            | NodeKind::TypeAlias
            | NodeKind::Newtype => (TYPE, DECLARATION),
            NodeKind::Function => (FUNCTION, DECLARATION),
            NodeKind::Method => (METHOD, DECLARATION),
            NodeKind::EnumCase => (ENUM_MEMBER, DECLARATION),
            NodeKind::Property | NodeKind::ClassLikeConstant => (PROPERTY, DECLARATION),
            NodeKind::Parameter | NodeKind::FunctionTypeParameter => (PARAMETER, NONE),
            NodeKind::TypeParameter => (TYPE_PARAMETER, DECLARATION),
            NodeKind::Namespace | NodeKind::UseItemAlias => (NAMESPACE, NONE),
            NodeKind::UseItem | NodeKind::UseItems => (TYPE, NONE),
            kind if is_type_position(kind) => {
                if analysis.follows_member_operator(start) {
                    (ENUM_MEMBER, NONE)
                } else {
                    (TYPE, NONE)
                }
            }
            NodeKind::MethodCall
            | NodeKind::NullSafeMethodCall
            | NodeKind::StaticMethodCall
            | NodeKind::MethodPartialApplication
            | NodeKind::StaticMethodPartialApplication => (METHOD, NONE),
            NodeKind::FunctionCall | NodeKind::FunctionPartialApplication | NodeKind::Callee => {
                (FUNCTION, NONE)
            }
            NodeKind::PropertyAccess
            | NodeKind::NullSafePropertyAccess
            | NodeKind::StaticPropertyAccess
            | NodeKind::ConstantAccess
            | NodeKind::Constant => (PROPERTY, NONE),
            NodeKind::ClassConstantAccess => (ENUM_MEMBER, NONE),
            _ => continue,
        };

        return role;
    }

    (VARIABLE, NONE)
}

fn declared_modifiers(analysis: &Analysis<'_>, ancestors: &[usize], name: usize) -> u32 {
    let elements = analysis.elements();
    let Some(declaration) = ancestors
        .iter()
        .rev()
        .find(|parent| declares(elements[**parent].kind))
        .map(|parent| elements[*parent].start)
    else {
        return NONE;
    };

    let mut modifiers = NONE;
    for token in analysis.tokens() {
        let offset = token.start.offset as usize;
        if offset < declaration {
            continue;
        }

        if offset >= name {
            break;
        }

        modifiers |= match token.kind {
            TokenKind::Readonly => READONLY,
            TokenKind::Static => STATIC,
            TokenKind::Abstract => ABSTRACT,
            TokenKind::Final => FINAL,
            _ => NONE,
        };
    }

    modifiers
}

fn mark_lexical(analysis: &Analysis<'_>, marked: &mut BTreeMap<usize, Marked>) {
    for token in analysis.tokens() {
        let start = token.start.offset as usize;
        if covered(marked, start) {
            continue;
        }

        if let Some(kind) = lexical_role(token.kind) {
            insert(marked, start, start + token.value.len(), kind, NONE);
        }
    }
}

fn covered(marked: &BTreeMap<usize, Marked>, offset: usize) -> bool {
    marked
        .range(..=offset)
        .next_back()
        .is_some_and(|(_, span)| offset < span.end)
}

fn lexical_role(kind: TokenKind) -> Option<u32> {
    if kind.is_comment() || kind == TokenKind::Shebang {
        return Some(COMMENT);
    }

    match kind {
        TokenKind::Variable => Some(VARIABLE),
        TokenKind::LiteralString | TokenKind::StringPart => Some(STRING),
        TokenKind::LiteralInteger | TokenKind::LiteralFloat => Some(NUMBER),
        TokenKind::HashLeftBracket => Some(DECORATOR),
        TokenKind::Bool
        | TokenKind::Int
        | TokenKind::Float
        | TokenKind::String
        | TokenKind::Array
        | TokenKind::Vec
        | TokenKind::Dict
        | TokenKind::Mixed
        | TokenKind::Never
        | TokenKind::Void
        | TokenKind::Object
        | TokenKind::Classname => Some(TYPE),
        TokenKind::LeftParenthesis
        | TokenKind::RightParenthesis
        | TokenKind::LeftBracket
        | TokenKind::RightBracket
        | TokenKind::LeftBrace
        | TokenKind::RightBrace
        | TokenKind::Comma
        | TokenKind::Semicolon => None,
        other if other.is_keyword() => Some(KEYWORD),
        other if is_operator(other) => Some(OPERATOR),
        _ => None,
    }
}

const fn is_operator(kind: TokenKind) -> bool {
    kind.is_infix()
        || kind.is_unary_prefix()
        || kind.is_postfix()
        || kind.is_assignment()
        || matches!(
            kind,
            TokenKind::EqualGreaterThan
                | TokenKind::MinusGreaterThan
                | TokenKind::QuestionMinusGreaterThan
                | TokenKind::ColonColon
                | TokenKind::ColonColonLessThan
                | TokenKind::DotDot
                | TokenKind::DotDotEqual
                | TokenKind::DotDotDot
                | TokenKind::Question
                | TokenKind::Colon
        )
}

const fn declares(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Enum
            | NodeKind::Method
            | NodeKind::Property
            | NodeKind::ClassLikeConstant
            | NodeKind::Function
            | NodeKind::EnumCase
            | NodeKind::TypeAlias
            | NodeKind::Newtype
    )
}

const fn is_type_position(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::NamedType
            | NodeKind::Type
            | NodeKind::TypeArgument
            | NodeKind::TypeParameterBound
            | NodeKind::TypeParameterDefault
            | NodeKind::ReturnType
            | NodeKind::NegatedType
            | NodeKind::UnionType
            | NodeKind::IntersectionType
            | NodeKind::ClassnameType
            | NodeKind::EnumBackingType
            | NodeKind::Extends
            | NodeKind::Implements
            | NodeKind::Instantiation
            | NodeKind::ClassReference
            | NodeKind::NamedClassReference
    )
}

const fn is_construct(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Construct
            | NodeKind::RequireConstruct
            | NodeKind::RequireOnceConstruct
            | NodeKind::LengthConstruct
            | NodeKind::ContainsConstruct
            | NodeKind::ContainsKeyConstruct
            | NodeKind::CloneConstruct
            | NodeKind::RemoveConstruct
            | NodeKind::SwapRemoveConstruct
            | NodeKind::RemoveFirstConstruct
            | NodeKind::RemoveLastConstruct
            | NodeKind::AssertConstruct
            | NodeKind::ExitConstruct
            | NodeKind::PanicConstruct
            | NodeKind::WriteConstruct
            | NodeKind::WriteLineConstruct
            | NodeKind::WriteErrorConstruct
            | NodeKind::WriteErrorLineConstruct
            | NodeKind::DebugConstruct
            | NodeKind::DiscardConstruct
            | NodeKind::DropConstruct
            | NodeKind::FileConstruct
            | NodeKind::DirectoryConstruct
            | NodeKind::EmbedConstruct
    )
}

fn encode(analysis: &Analysis<'_>, marked: &BTreeMap<usize, Marked>) -> Vec<SemanticToken> {
    let mut data = Vec::new();
    let mut previous_line = 0;
    let mut previous_start = 0;

    for (start, span) in marked {
        let mut offset = *start;
        for piece in analysis.source()[*start..span.end.min(analysis.source().len())].split('\n') {
            let length = u32::try_from(piece.trim_end_matches('\r').encode_utf16().count())
                .unwrap_or(u32::MAX);
            if length > 0 {
                let position = analysis.lines().position(offset);
                let delta_line = position.line - previous_line;
                let delta_start = if delta_line == 0 {
                    position.character - previous_start
                } else {
                    position.character
                };

                data.push(SemanticToken {
                    delta_line,
                    delta_start,
                    length,
                    token_type: span.kind,
                    token_modifiers_bitset: span.modifiers,
                });

                previous_line = position.line;
                previous_start = position.character;
            }

            offset += piece.len() + 1;
        }
    }

    data
}

#[cfg(test)]
mod tests {
    use super::COMMENT;
    use super::KEYWORD;
    use super::STRING;
    use super::tokens;
    use crate::server::analysis::Analysis;

    fn kinds(source: &str) -> Vec<u32> {
        tokens(&Analysis::new(source))
            .data
            .into_iter()
            .map(|token| token.token_type)
            .collect()
    }

    #[test]
    fn broken_source_keeps_lexical_highlighting() {
        let found = kinds("final class { /* note */ 'text'");
        assert!(found.contains(&KEYWORD));
        assert!(found.contains(&COMMENT));
        assert!(found.contains(&STRING));
    }

    #[test]
    fn tokens_never_cross_lines() {
        let found = tokens(&Analysis::new("/* one\ntwo */"));
        assert_eq!(found.data.len(), 2);
        assert!(found.data.iter().all(|token| token.length > 0));
    }
}
