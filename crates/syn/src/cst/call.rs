//! Calls: call expressions, argument lists, and partial application.

use std::ops::ControlFlow;

use crate::arena::Arena;
use crate::arena::Vec;
use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::access::ClassReference;
use crate::cst::atom::Identifier;
use crate::cst::atom::LocalIdentifier;
use crate::cst::expression::Expression;
use crate::cst::expression::LeftmostStep;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::cst::r#type::TypeArgumentList;

/// The callee of a function call: a name known at parse time (`foo(...)`),
/// or an expression evaluating to a callable (`$fn(...)`, `(expr)(...)`).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Callee<'arena> {
    Identifier(Identifier<'arena>),
    Expression(&'arena Expression<'arena>),
}

/// A call expression.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Call<'arena> {
    Function(FunctionCall<'arena>),
    Method(MethodCall<'arena>),
    NullSafeMethod(NullSafeMethodCall<'arena>),
    StaticMethod(StaticMethodCall<'arena>),
}

/// `foo($bar)`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FunctionCall<'arena> {
    pub function: Callee<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub argument_list: ArgumentList<'arena>,
}

/// `$object->method($bar)`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct MethodCall<'arena> {
    pub object: &'arena Expression<'arena>,
    pub arrow: Span,
    pub method: LocalIdentifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub argument_list: ArgumentList<'arena>,
}

/// `$object?->method($bar)`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NullSafeMethodCall<'arena> {
    pub object: &'arena Expression<'arena>,
    pub question_mark_arrow: Span,
    pub method: LocalIdentifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub argument_list: ArgumentList<'arena>,
}

/// `Foo::method($bar)`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct StaticMethodCall<'arena> {
    pub class: ClassReference<'arena>,
    pub double_colon: Span,
    pub method: LocalIdentifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub argument_list: ArgumentList<'arena>,
}

impl<'arena> Call<'arena> {
    #[inline]
    #[must_use]
    pub const fn get_argument_list(&self) -> &ArgumentList<'arena> {
        match self {
            Call::Function(call) => &call.argument_list,
            Call::Method(call) => &call.argument_list,
            Call::NullSafeMethod(call) => &call.argument_list,
            Call::StaticMethod(call) => &call.argument_list,
        }
    }

    pub(crate) fn leftmost_step(&self) -> LeftmostStep<'arena> {
        match self {
            Call::Function(call) => call.function.leftmost_step(),
            Call::Method(call) => ControlFlow::Continue(call.object),
            Call::NullSafeMethod(call) => ControlFlow::Continue(call.object),
            Call::StaticMethod(call) => call.class.leftmost_step(),
        }
    }
}

impl<'arena> Callee<'arena> {
    pub(crate) fn leftmost_step(&self) -> LeftmostStep<'arena> {
        match self {
            Callee::Expression(expression) => ControlFlow::Continue(expression),
            Callee::Identifier(identifier) => ControlFlow::Break(identifier.span()),
        }
    }

    /// The span of this callee's leftmost descendant; see
    /// [`Expression::leftmost_span`].
    #[must_use]
    pub fn leftmost_span(&self) -> Span {
        match self.leftmost_step() {
            ControlFlow::Continue(expression) => expression.leftmost_span(),
            ControlFlow::Break(span) => span,
        }
    }
}

impl<'arena> PartialApplication<'arena> {
    pub(crate) fn leftmost_step(&self) -> LeftmostStep<'arena> {
        match self {
            PartialApplication::Function(application) => application.function.leftmost_step(),
            PartialApplication::Method(application) => ControlFlow::Continue(application.object),
            PartialApplication::StaticMethod(application) => application.class.leftmost_step(),
        }
    }
}

impl HasSpan for Callee<'_> {
    fn span(&self) -> Span {
        match self {
            Callee::Identifier(identifier) => identifier.span(),
            Callee::Expression(expression) => expression.span(),
        }
    }
}

impl HasSpan for Call<'_> {
    fn span(&self) -> Span {
        match self {
            Call::Function(call) => call.span(),
            Call::Method(call) => call.span(),
            Call::NullSafeMethod(call) => call.span(),
            Call::StaticMethod(call) => call.span(),
        }
    }
}

impl HasSpan for FunctionCall<'_> {
    fn span(&self) -> Span {
        self.function
            .leftmost_span()
            .join(self.argument_list.span())
    }
}

impl HasSpan for MethodCall<'_> {
    fn span(&self) -> Span {
        self.object.leftmost_span().join(self.argument_list.span())
    }
}

impl HasSpan for NullSafeMethodCall<'_> {
    fn span(&self) -> Span {
        self.object.leftmost_span().join(self.argument_list.span())
    }
}

impl HasSpan for StaticMethodCall<'_> {
    fn span(&self) -> Span {
        self.class.leftmost_span().join(self.argument_list.span())
    }
}

/// A list of arguments.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ArgumentList<'arena> {
    pub left_parenthesis: Span,
    pub arguments: TokenSeparatedSequence<'arena, Argument<'arena>>,
    pub right_parenthesis: Span,
}

/// A list of arguments in a partial function application.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PartialArgumentList<'arena> {
    pub left_parenthesis: Span,
    pub arguments: TokenSeparatedSequence<'arena, PartialArgument<'arena>>,
    pub right_parenthesis: Span,
}

/// An argument.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Argument<'arena> {
    Positional(PositionalArgument<'arena>),
    Named(NamedArgument<'arena>),
}

/// An argument or placeholder in a partial function application.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum PartialArgument<'arena> {
    Positional(PositionalArgument<'arena>),
    Named(NamedArgument<'arena>),
    NamedPlaceholder(NamedPlaceholderArgument<'arena>),
    Placeholder(PlaceholderArgument),
    VariadicPlaceholder(VariadicPlaceholderArgument),
}

/// A positional argument.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PositionalArgument<'arena> {
    pub value: &'arena Expression<'arena>,
}

/// A named argument.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NamedArgument<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub colon: Span,
    pub value: &'arena Expression<'arena>,
}

/// A named placeholder in a partial function application.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct NamedPlaceholderArgument<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub colon: Span,
    pub question_mark: Span,
}

/// A placeholder in a partial function application.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PlaceholderArgument {
    pub span: Span,
}

/// A variadic placeholder in a partial function application.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct VariadicPlaceholderArgument {
    pub span: Span,
}

impl<'arena> PartialArgumentList<'arena> {
    /// Whether the list is exactly `(...)`, forming a first-class callable.
    #[inline]
    #[must_use]
    pub const fn is_first_class_callable(&self) -> bool {
        self.arguments.len() == 1
            && matches!(
                self.arguments.first(),
                Some(PartialArgument::VariadicPlaceholder(_))
            )
    }

    #[inline]
    #[must_use]
    pub fn has_placeholders(&self) -> bool {
        self.arguments.iter().any(|argument| {
            matches!(
                argument,
                PartialArgument::Placeholder(_)
                    | PartialArgument::VariadicPlaceholder(_)
                    | PartialArgument::NamedPlaceholder(_)
            )
        })
    }

    #[inline]
    pub fn into_argument_list<A>(self, arena: &'arena A) -> ArgumentList<'arena>
    where
        A: Arena,
    {
        debug_assert!(
            !self.has_placeholders(),
            "cannot convert PartialArgumentList with placeholders to ArgumentList"
        );

        let mut arguments = Vec::new_in(arena);
        for argument in self.arguments.nodes {
            match argument {
                PartialArgument::Positional(positional) => {
                    arguments.push(Argument::Positional(*positional));
                }
                PartialArgument::Named(named) => arguments.push(Argument::Named(named.clone())),
                PartialArgument::Placeholder(_)
                | PartialArgument::NamedPlaceholder(_)
                | PartialArgument::VariadicPlaceholder(_) => {}
            }
        }

        ArgumentList {
            left_parenthesis: self.left_parenthesis,
            arguments: TokenSeparatedSequence::from_slices(arguments.leak(), self.arguments.tokens),
            right_parenthesis: self.right_parenthesis,
        }
    }
}

impl<'arena> Argument<'arena> {
    #[inline]
    #[must_use]
    pub const fn is_positional(&self) -> bool {
        matches!(self, Argument::Positional(_))
    }

    #[inline]
    #[must_use]
    pub const fn value(&self) -> &'arena Expression<'arena> {
        match self {
            Argument::Positional(argument) => argument.value,
            Argument::Named(argument) => argument.value,
        }
    }
}

impl<'arena> PartialArgument<'arena> {
    #[inline]
    #[must_use]
    pub const fn is_positional(&self) -> bool {
        matches!(self, PartialArgument::Positional(_))
    }

    #[inline]
    #[must_use]
    pub const fn value(&self) -> Option<&'arena Expression<'arena>> {
        match self {
            PartialArgument::Positional(argument) => Some(argument.value),
            PartialArgument::Named(argument) => Some(argument.value),
            _ => None,
        }
    }
}

impl HasSpan for ArgumentList<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for PartialArgumentList<'_> {
    fn span(&self) -> Span {
        self.left_parenthesis.join(self.right_parenthesis)
    }
}

impl HasSpan for Argument<'_> {
    fn span(&self) -> Span {
        match self {
            Argument::Positional(argument) => argument.span(),
            Argument::Named(argument) => argument.span(),
        }
    }
}

impl HasSpan for PartialArgument<'_> {
    fn span(&self) -> Span {
        match self {
            PartialArgument::Positional(argument) => argument.span(),
            PartialArgument::Named(argument) => argument.span(),
            PartialArgument::NamedPlaceholder(argument) => argument.span(),
            PartialArgument::Placeholder(placeholder) => placeholder.span(),
            PartialArgument::VariadicPlaceholder(placeholder) => placeholder.span(),
        }
    }
}

impl HasSpan for PositionalArgument<'_> {
    fn span(&self) -> Span {
        self.value.span()
    }
}

impl HasSpan for NamedArgument<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.value.span())
    }
}

impl HasSpan for NamedPlaceholderArgument<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.question_mark)
    }
}

impl HasSpan for PlaceholderArgument {
    fn span(&self) -> Span {
        self.span
    }
}

impl HasSpan for VariadicPlaceholderArgument {
    fn span(&self) -> Span {
        self.span
    }
}

/// A partial function application or first-class callable.
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum PartialApplication<'arena> {
    Function(FunctionPartialApplication<'arena>),
    Method(MethodPartialApplication<'arena>),
    StaticMethod(StaticMethodPartialApplication<'arena>),
}

/// `foo(?, 1)`, `foo(...)`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FunctionPartialApplication<'arena> {
    pub function: Callee<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub argument_list: PartialArgumentList<'arena>,
}

/// `$object->method(?, 1)`, `$object->method(...)`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct MethodPartialApplication<'arena> {
    pub object: &'arena Expression<'arena>,
    pub arrow: Span,
    pub method: LocalIdentifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub argument_list: PartialArgumentList<'arena>,
}

/// `Foo::method(?, 1)`, `Foo::method(...)`
#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct StaticMethodPartialApplication<'arena> {
    pub class: ClassReference<'arena>,
    pub double_colon: Span,
    pub method: LocalIdentifier<'arena>,
    pub type_arguments: Option<TypeArgumentList<'arena>>,
    pub argument_list: PartialArgumentList<'arena>,
}

impl<'arena> PartialApplication<'arena> {
    /// Whether the argument list is exactly `(...)`, forming a first-class
    /// callable.
    #[inline]
    #[must_use]
    pub const fn is_first_class_callable(&self) -> bool {
        self.get_argument_list().is_first_class_callable()
    }

    #[inline]
    #[must_use]
    pub const fn get_argument_list(&self) -> &PartialArgumentList<'arena> {
        match self {
            PartialApplication::Function(application) => &application.argument_list,
            PartialApplication::Method(application) => &application.argument_list,
            PartialApplication::StaticMethod(application) => &application.argument_list,
        }
    }
}

impl HasSpan for PartialApplication<'_> {
    fn span(&self) -> Span {
        match self {
            PartialApplication::Function(application) => application.span(),
            PartialApplication::Method(application) => application.span(),
            PartialApplication::StaticMethod(application) => application.span(),
        }
    }
}

impl HasSpan for FunctionPartialApplication<'_> {
    fn span(&self) -> Span {
        self.function
            .leftmost_span()
            .join(self.argument_list.span())
    }
}

impl HasSpan for MethodPartialApplication<'_> {
    fn span(&self) -> Span {
        self.object.leftmost_span().join(self.argument_list.span())
    }
}

impl HasSpan for StaticMethodPartialApplication<'_> {
    fn span(&self) -> Span {
        self.class.leftmost_span().join(self.argument_list.span())
    }
}
