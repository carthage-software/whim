//! The Wadler/Prettier-style document intermediate representation.

use std::cell::RefCell;

use whim_syn::arena::Arena;
use whim_syn::arena::Vec;

/// A single layout instruction.
#[derive(Debug)]
pub(super) enum Document<'arena, A>
where
    A: Arena,
{
    String(&'arena str),
    Array(Vec<'arena, Self, A>),
    Indent(Vec<'arena, Self, A>),
    /// Increase indentation only while the enclosing group is broken.
    IndentIfBreak(Vec<'arena, Self, A>),
    /// Print one additional line after the contained document when rendering
    /// it crossed a line boundary.
    BlankLineAfterIfMultiline(&'arena Self),
    Group(Group<'arena, A>),
    Line(Line),
    /// Buffered content flushed just before the next newline; used for trailing
    /// comments so they never land in the middle of following code.
    LineSuffix(Vec<'arena, Self, A>),
    IfBreak(IfBreak<'arena, A>),
    BreakParent,
}

/// The break behaviour of a [`Group`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum BreakMode {
    /// Break only when the group does not fit on one line.
    Auto,
    /// Always break; propagates to enclosing groups.
    Force,
    /// Keep the group flat after an outer group chose a better break point.
    Never,
    /// Break when the enclosing group breaks.
    Parent,
    /// Measure the group at its printed column, even inside a flat parent.
    Independent,
}

/// A group of documents the printer treats as a single fit-or-break unit.
#[derive(Debug)]
pub(super) struct Group<'arena, A>
where
    A: Arena,
{
    pub contents: Vec<'arena, Document<'arena, A>, A>,
    pub break_mode: RefCell<BreakMode>,
}

#[derive(Default, Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct Line {
    /// Always renders as a newline, even in a flat group.
    pub hard: bool,
    /// Renders as nothing (rather than a space) while flat.
    pub soft: bool,
}

/// Content chosen by whether the enclosing group broke.
#[derive(Debug)]
pub(super) struct IfBreak<'arena, A>
where
    A: Arena,
{
    pub break_contents: &'arena Document<'arena, A>,
    pub flat_content: &'arena Document<'arena, A>,
}

impl Line {
    /// A break that renders as a space when flat and a newline when broken.
    #[inline]
    #[must_use]
    pub(super) fn soft() -> Self {
        Self {
            soft: true,
            ..Self::default()
        }
    }

    /// A break that always renders as a newline.
    #[inline]
    #[must_use]
    pub(super) fn hard() -> Self {
        Self {
            hard: true,
            ..Self::default()
        }
    }
}

impl<'arena, A> Group<'arena, A>
where
    A: Arena,
{
    #[inline]
    #[must_use]
    pub(super) const fn new(contents: Vec<'arena, Document<'arena, A>, A>) -> Self {
        Self {
            contents,
            break_mode: RefCell::new(BreakMode::Auto),
        }
    }

    #[inline]
    #[must_use]
    pub(super) const fn with_break_mode(mut self, mode: BreakMode) -> Self {
        self.break_mode = RefCell::new(mode);
        self
    }
}

impl<A> Document<'_, A>
where
    A: Arena,
{
    /// An empty document.
    #[inline]
    #[must_use]
    pub(super) const fn empty() -> Self {
        Document::String("")
    }

    /// A single hard space.
    #[inline]
    #[must_use]
    pub(super) const fn space() -> Self {
        Document::String(" ")
    }
}
