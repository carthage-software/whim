//! The Whim concrete syntax tree (CST).

pub mod access;
pub mod array;
pub mod atom;
pub mod binding;
pub mod call;
pub mod class;
pub mod construct;
pub mod control_flow;
pub mod declaration;
pub mod expression;
pub mod function;
pub mod node;
pub mod operation;
pub mod pattern;
pub mod sequence;
pub mod statement;
pub mod trivia;
pub mod r#type;
pub mod walker;

use whim_span::HasSpan;
use whim_span::Position;
use whim_span::Span;

use crate::cst::statement::Statement;
use crate::cst::trivia::Trivia;

/// A fully parsed Whim source file.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Program<'arena> {
    pub source_text: &'arena str,
    pub trivia: &'arena [Trivia<'arena>],
    pub statements: &'arena [Statement<'arena>],
}

impl HasSpan for Program<'_> {
    fn span(&self) -> Span {
        let start = self
            .statements
            .first()
            .map_or_else(Position::zero, |statement| statement.span().start);
        let end = self
            .statements
            .last()
            .map_or_else(Position::zero, |statement| statement.span().end);

        Span::new(start, end)
    }
}
