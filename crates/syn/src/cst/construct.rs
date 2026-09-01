use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::atom::LiteralString;
use crate::cst::atom::LocalIdentifier;
use crate::cst::atom::Variable;
use crate::cst::expression::Expression;
use crate::cst::sequence::TokenSeparatedSequence;

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Construct<'arena> {
    Require(RequireConstruct<'arena>),
    RequireOnce(RequireOnceConstruct<'arena>),
    Length(LengthConstruct<'arena>),
    Contains(ContainsConstruct<'arena>),
    ContainsKey(ContainsKeyConstruct<'arena>),
    Clone(CloneConstruct<'arena>),
    Remove(RemoveConstruct<'arena>),
    SwapRemove(SwapRemoveConstruct<'arena>),
    RemoveFirst(RemoveFirstConstruct<'arena>),
    RemoveLast(RemoveLastConstruct<'arena>),
    Assert(AssertConstruct<'arena>),
    Exit(ExitConstruct<'arena>),
    Panic(PanicConstruct<'arena>),
    Write(WriteConstruct<'arena>),
    WriteLine(WriteLineConstruct<'arena>),
    WriteError(WriteErrorConstruct<'arena>),
    WriteErrorLine(WriteErrorLineConstruct<'arena>),
    Debug(DebugConstruct<'arena>),
    Discard(DiscardConstruct<'arena>),
    Drop(DropConstruct<'arena>),
    File(FileConstruct<'arena>),
    Directory(DirectoryConstruct<'arena>),
    Embed(EmbedConstruct<'arena>),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct RequireConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub value: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct RequireOnceConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub value: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct LengthConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub value: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ContainsConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub array: &'arena Expression<'arena>,
    pub comma: Span,
    pub value: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ContainsKeyConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub array: &'arena Expression<'arena>,
    pub comma: Span,
    pub key: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct CloneConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub object: &'arena Expression<'arena>,
    pub fields: &'arena [CloneField<'arena>],
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct CloneField<'arena> {
    pub comma: Span,
    pub name: LocalIdentifier<'arena>,
    pub colon: Span,
    pub value: &'arena Expression<'arena>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct RemoveConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub array: &'arena Expression<'arena>,
    pub comma: Span,
    pub key: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct SwapRemoveConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub vector: &'arena Expression<'arena>,
    pub comma: Span,
    pub index: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct RemoveFirstConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub array: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct RemoveLastConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub array: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct AssertConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub condition: &'arena Expression<'arena>,
    pub message: Option<AssertMessage<'arena>>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct AssertMessage<'arena> {
    pub comma: Span,
    pub value: &'arena Expression<'arena>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ExitConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub code: Option<&'arena Expression<'arena>>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PanicConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub message: LiteralString<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct WriteConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub arguments: TokenSeparatedSequence<'arena, ConstructArgument<'arena>>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct WriteLineConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub arguments: TokenSeparatedSequence<'arena, ConstructArgument<'arena>>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct WriteErrorConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub arguments: TokenSeparatedSequence<'arena, ConstructArgument<'arena>>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct WriteErrorLineConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub arguments: TokenSeparatedSequence<'arena, ConstructArgument<'arena>>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DebugConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub arguments: TokenSeparatedSequence<'arena, ConstructArgument<'arena>>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DiscardConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub value: &'arena Expression<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DropConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub variables: TokenSeparatedSequence<'arena, Variable<'arena>>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct FileConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DirectoryConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct EmbedConstruct<'arena> {
    pub name: LocalIdentifier<'arena>,
    pub bang: Span,
    pub left_parenthesis: Span,
    pub path: LiteralString<'arena>,
    pub trailing_comma: Option<Span>,
    pub right_parenthesis: Span,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ConstructArgument<'arena> {
    pub value: &'arena Expression<'arena>,
}

impl HasSpan for Construct<'_> {
    fn span(&self) -> Span {
        match self {
            Construct::Require(construct) => construct.span(),
            Construct::RequireOnce(construct) => construct.span(),
            Construct::Length(construct) => construct.span(),
            Construct::Contains(construct) => construct.span(),
            Construct::ContainsKey(construct) => construct.span(),
            Construct::Clone(construct) => construct.span(),
            Construct::Remove(construct) => construct.span(),
            Construct::SwapRemove(construct) => construct.span(),
            Construct::RemoveFirst(construct) => construct.span(),
            Construct::RemoveLast(construct) => construct.span(),
            Construct::Assert(construct) => construct.span(),
            Construct::Exit(construct) => construct.span(),
            Construct::Panic(construct) => construct.span(),
            Construct::Write(construct) => construct.span(),
            Construct::WriteLine(construct) => construct.span(),
            Construct::WriteError(construct) => construct.span(),
            Construct::WriteErrorLine(construct) => construct.span(),
            Construct::Debug(construct) => construct.span(),
            Construct::Discard(construct) => construct.span(),
            Construct::Drop(construct) => construct.span(),
            Construct::File(construct) => construct.span(),
            Construct::Directory(construct) => construct.span(),
            Construct::Embed(construct) => construct.span(),
        }
    }
}

impl HasSpan for RequireConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for RequireOnceConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for LengthConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for ContainsConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for ContainsKeyConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for CloneConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for RemoveConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for SwapRemoveConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for RemoveFirstConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for RemoveLastConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for AssertConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for ExitConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for PanicConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for WriteConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for WriteLineConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for WriteErrorConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for WriteErrorLineConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for DebugConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for DiscardConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for DropConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for FileConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for DirectoryConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for EmbedConstruct<'_> {
    fn span(&self) -> Span {
        self.name.span().join(self.right_parenthesis)
    }
}

impl HasSpan for CloneField<'_> {
    fn span(&self) -> Span {
        self.comma.join(self.value.span())
    }
}

impl HasSpan for AssertMessage<'_> {
    fn span(&self) -> Span {
        self.comma.join(self.value.span())
    }
}

impl HasSpan for ConstructArgument<'_> {
    fn span(&self) -> Span {
        self.value.span()
    }
}
