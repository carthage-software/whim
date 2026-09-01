//! Token-separated sequence container.

use std::slice::Iter;

use crate::arena::Arena;
use crate::arena::Vec;
use whim_span::HasSpan;
use whim_span::Position;
use whim_span::Span;

use crate::token::Token;

/// A sequence of CST nodes separated by tokens (commas, semicolons, ...).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct TokenSeparatedSequence<'arena, T> {
    pub nodes: &'arena [T],
    pub tokens: &'arena [Token<'arena>],
}

impl<'arena, T> TokenSeparatedSequence<'arena, T> {
    /// Freezes arena-allocated node and token vectors into a separated
    /// sequence.
    #[inline]
    #[must_use]
    pub fn new<A: Arena>(nodes: Vec<'arena, T, A>, tokens: Vec<'arena, Token<'arena>, A>) -> Self {
        Self {
            nodes: nodes.leak(),
            tokens: tokens.leak(),
        }
    }

    /// Wraps existing arena slices as a separated sequence.
    #[inline]
    #[must_use]
    pub const fn from_slices(nodes: &'arena [T], tokens: &'arena [Token<'arena>]) -> Self {
        Self { nodes, tokens }
    }

    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            nodes: &[],
            tokens: &[],
        }
    }

    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.nodes.get(index)
    }

    #[inline]
    #[must_use]
    pub const fn first(&self) -> Option<&T> {
        self.nodes.first()
    }

    #[inline]
    #[must_use]
    pub const fn last(&self) -> Option<&T> {
        self.nodes.last()
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        self.nodes.iter()
    }

    #[inline]
    #[must_use]
    pub const fn as_slice(&self) -> &'arena [T] {
        self.nodes
    }
}

impl<'arena, T: HasSpan> TokenSeparatedSequence<'arena, T> {
    /// Returns the trailing separator token, if any.
    #[inline]
    #[must_use]
    pub fn get_trailing_token(&self) -> Option<&Token<'arena>> {
        self.tokens.last().filter(|token| {
            token.start.offset >= self.nodes.last().map_or(0, |node| node.span().end.offset)
        })
    }

    /// The span of the first element (node or separator), if any.
    #[inline]
    #[must_use]
    pub fn first_span(&self) -> Option<Span> {
        match (self.tokens.first(), self.nodes.first()) {
            (Some(token), Some(node)) => {
                let token_span = token.compute_span();
                if token_span.end.offset <= node.span().start.offset {
                    Some(token_span)
                } else {
                    Some(node.span())
                }
            }
            (Some(token), None) => Some(token.compute_span()),
            (None, Some(node)) => Some(node.span()),
            (None, None) => None,
        }
    }

    /// The span of the last element (node or separator), if any.
    #[inline]
    #[must_use]
    pub fn last_span(&self) -> Option<Span> {
        match (self.tokens.last(), self.nodes.last()) {
            (Some(token), Some(node)) => {
                if token.start.offset >= node.span().end.offset {
                    Some(token.compute_span())
                } else {
                    Some(node.span())
                }
            }
            (Some(token), None) => Some(token.compute_span()),
            (None, Some(node)) => Some(node.span()),
            (None, None) => None,
        }
    }

    /// The span covering the whole sequence, or an empty span at `from`
    /// when the sequence is empty.
    #[inline]
    #[must_use]
    pub fn span(&self, from: Position) -> Span {
        match (self.first_span(), self.last_span()) {
            (Some(first), Some(last)) => Span::new(first.start, last.end),
            _ => Span::new(from, from),
        }
    }
}

impl<'arena, T> IntoIterator for TokenSeparatedSequence<'arena, T> {
    type Item = &'arena T;
    type IntoIter = Iter<'arena, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.nodes.iter()
    }
}

impl<'seq, T> IntoIterator for &'seq TokenSeparatedSequence<'_, T> {
    type Item = &'seq T;
    type IntoIter = Iter<'seq, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
