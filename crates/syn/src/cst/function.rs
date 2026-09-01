//! Function-like constructs: functions, closures, short closures, parameters, and return types.

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Keyword;
use crate::cst::atom::LocalIdentifier;
use crate::cst::atom::Modifier;
use crate::cst::atom::Variable;
use crate::cst::declaration::AttributeList;
use crate::cst::expression::Expression;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::statement::Block;
use crate::cst::r#type::Type;
use crate::cst::r#type::TypeParameterList;

/// A `function` declaration.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Function<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub function: Keyword<'arena>,
    pub name: LocalIdentifier<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub parameter_list: ParameterList<'arena>,
    pub return_type: Option<ReturnType<'arena>>,
    pub body: Block<'arena>,
}

impl HasSpan for Function<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.body.span());
        }

        self.function.span().join(self.body.span())
    }
}

/// An anonymous function.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Closure<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub function: Keyword<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub parameter_list: ParameterList<'arena>,
    pub use_clause: Option<ClosureUseClause<'arena>>,
    pub return_type: Option<ReturnType<'arena>>,
    pub body: Block<'arena>,
}

/// The capture clause of a closure.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ClosureUseClause<'arena> {
    pub r#use: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub variables: TokenSeparatedSequence<'arena, Variable<'arena>>,
    pub right_parenthesis: Span,
}

impl HasSpan for Closure<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.body.span());
        }

        self.function.span().join(self.body.span())
    }
}

impl HasSpan for ClosureUseClause<'_> {
    fn span(&self) -> Span {
        self.r#use.span().join(self.right_parenthesis)
    }
}

/// A short closure that captures enclosing variables automatically, by value.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ShortClosure<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub r#fn: Keyword<'arena>,
    pub type_parameters: Option<TypeParameterList<'arena>>,
    pub parameter_list: ParameterList<'arena>,
    pub return_type: Option<ReturnType<'arena>>,
    pub body: ShortClosureBody<'arena>,
}

/// The expression or statement block executed by a short closure.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ShortClosureBody<'arena> {
    Expression {
        arrow: Span,
        expression: &'arena Expression<'arena>,
    },
    Block(Block<'arena>),
}

impl HasSpan for ShortClosure<'_> {
    fn span(&self) -> Span {
        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(self.body.span());
        }

        self.r#fn.span().join(self.body.span())
    }
}

impl HasSpan for ShortClosureBody<'_> {
    fn span(&self) -> Span {
        match self {
            Self::Expression { arrow, expression } => arrow.join(expression.span()),
            Self::Block(block) => block.span(),
        }
    }
}

/// A parenthesized, comma-separated list of parameters.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ParameterList<'arena> {
    pub left_parenthesis: Span,
    pub parameters: TokenSeparatedSequence<'arena, Parameter<'arena>>,
    pub right_parenthesis: Span,
}

/// A function-like parameter.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Parameter<'arena> {
    pub attribute_lists: &'arena [AttributeList<'arena>],
    pub modifiers: &'arena [Modifier<'arena>],
    pub r#type: Option<&'arena Type<'arena>>,
    pub variable: Variable<'arena>,
    pub default: Option<ParameterDefault<'arena>>,
}

/// The default value of a parameter.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ParameterDefault<'arena> {
    pub equals: Span,
    pub value: &'arena Expression<'arena>,
}

impl Parameter<'_> {
    /// Whether the parameter is a promoted constructor property.
    #[inline]
    #[must_use]
    pub const fn is_promoted_property(&self) -> bool {
        !self.modifiers.is_empty()
    }
}

impl HasSpan for ParameterList<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for Parameter<'_> {
    fn span(&self) -> Span {
        let right = self
            .default
            .as_ref()
            .map_or_else(|| self.variable.span(), HasSpan::span);

        if let Some(attribute_list) = self.attribute_lists.first() {
            return attribute_list.span().join(right);
        }

        if let Some(modifier) = self.modifiers.first() {
            return modifier.span().join(right);
        }

        if let Some(r#type) = self.r#type {
            return r#type.span().join(right);
        }

        self.variable.span().join(right)
    }
}

impl HasSpan for ParameterDefault<'_> {
    fn span(&self) -> Span {
        self.equals.join(self.value.span())
    }
}

/// A function-like return type.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ReturnType<'arena> {
    pub colon: Span,
    pub r#type: &'arena Type<'arena>,
}

impl HasSpan for ReturnType<'_> {
    fn span(&self) -> Span {
        self.colon.join(self.r#type.span())
    }
}
