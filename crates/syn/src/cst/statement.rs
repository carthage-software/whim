//! Statements: the statement tree, blocks, and declarations.

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::Keyword;
use crate::cst::atom::Variable;
use crate::cst::binding::BindingTarget;
use crate::cst::class::Class;
use crate::cst::class::Enum;
use crate::cst::class::Interface;
use crate::cst::control_flow::DoWhile;
use crate::cst::control_flow::For;
use crate::cst::control_flow::Foreach;
use crate::cst::control_flow::If;
use crate::cst::control_flow::Try;
use crate::cst::control_flow::While;
use crate::cst::declaration::Constant;
use crate::cst::declaration::FileAttributeList;
use crate::cst::declaration::Namespace;
use crate::cst::declaration::Use;
use crate::cst::expression::Expression;
use crate::cst::function::Function;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::r#type::Newtype;
use crate::cst::r#type::TypeAlias;

/// A top-level Whim statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum TopLevelStatement<'arena> {
    FileAttributeList(FileAttributeList<'arena>),
    Namespace(Namespace<'arena>),
    Use(Use<'arena>),
    Class(Class<'arena>),
    Interface(Interface<'arena>),
    Enum(Enum<'arena>),
    Function(Function<'arena>),
    Constant(Constant<'arena>),
    TypeAlias(TypeAlias<'arena>),
    Newtype(Newtype<'arena>),
    Statement(Statement<'arena>),
}

/// A Whim statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Statement<'arena> {
    If(If<'arena>),
    While(While<'arena>),
    DoWhile(DoWhile<'arena>),
    For(For<'arena>),
    Foreach(Foreach<'arena>),
    Try(Try<'arena>),
    Using(Using<'arena>),
    FinalLocal(FinalLocal<'arena>),
    Expression(ExpressionStatement<'arena>),
}

/// An expression in statement position, terminated by a semicolon.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ExpressionStatement<'arena> {
    pub expression: &'arena Expression<'arena>,
    pub semicolon: Span,
}

/// An assign-once local binding.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FinalLocal<'arena> {
    pub r#final: Keyword<'arena>,
    pub variable: Variable<'arena>,
    pub equal: Span,
    pub value: &'arena Expression<'arena>,
    pub semicolon: Span,
}

/// One resource binding in a [`Using`] statement.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct UsingBinding<'arena> {
    pub target: BindingTarget<'arena>,
    pub equal: Span,
    pub value: &'arena Expression<'arena>,
}

/// A lexical resource scope.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Using<'arena> {
    pub using: Keyword<'arena>,
    pub left_parenthesis: Span,
    pub bindings: TokenSeparatedSequence<'arena, UsingBinding<'arena>>,
    pub right_parenthesis: Span,
    pub body: Block<'arena>,
}

impl HasSpan for ExpressionStatement<'_> {
    fn span(&self) -> Span {
        self.expression.span().join(self.semicolon)
    }
}

impl HasSpan for FinalLocal<'_> {
    fn span(&self) -> Span {
        self.r#final.span().join(self.semicolon)
    }
}

impl HasSpan for UsingBinding<'_> {
    fn span(&self) -> Span {
        self.target.span().join(self.value.span())
    }
}

impl HasSpan for Using<'_> {
    fn span(&self) -> Span {
        self.using.span().join(self.body.span())
    }
}

impl HasSpan for TopLevelStatement<'_> {
    fn span(&self) -> Span {
        match self {
            TopLevelStatement::FileAttributeList(statement) => statement.span(),
            TopLevelStatement::Namespace(statement) => statement.span(),
            TopLevelStatement::Use(statement) => statement.span(),
            TopLevelStatement::Class(statement) => statement.span(),
            TopLevelStatement::Interface(statement) => statement.span(),
            TopLevelStatement::Enum(statement) => statement.span(),
            TopLevelStatement::Function(statement) => statement.span(),
            TopLevelStatement::Constant(statement) => statement.span(),
            TopLevelStatement::TypeAlias(statement) => statement.span(),
            TopLevelStatement::Newtype(statement) => statement.span(),
            TopLevelStatement::Statement(statement) => statement.span(),
        }
    }
}

impl HasSpan for Statement<'_> {
    fn span(&self) -> Span {
        match self {
            Statement::If(statement) => statement.span(),
            Statement::While(statement) => statement.span(),
            Statement::DoWhile(statement) => statement.span(),
            Statement::For(statement) => statement.span(),
            Statement::Foreach(statement) => statement.span(),
            Statement::Try(statement) => statement.span(),
            Statement::Using(statement) => statement.span(),
            Statement::FinalLocal(statement) => statement.span(),
            Statement::Expression(statement) => statement.span(),
        }
    }
}

/// A brace-delimited block of statements.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Block<'arena> {
    pub left_brace: Span,
    pub statements: &'arena [Statement<'arena>],
    pub right_brace: Span,
}

impl HasSpan for Block<'_> {
    fn span(&self) -> Span {
        self.left_brace.join(self.right_brace)
    }
}
