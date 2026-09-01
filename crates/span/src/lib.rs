//! Byte positions and spans within one Whim source file.

#![forbid(unsafe_code)]

use std::fmt;
use std::ops;
use std::ops::Bound;
use std::ops::Range;
use std::ops::RangeBounds;

use serde::Deserialize;
use serde::Serialize;

/// Represents a specific byte offset within a single source file.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Position {
    pub offset: u32,
}

/// Represents a contiguous range of source code within a single file.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

pub trait HasPosition {
    fn position(&self) -> Position;

    #[inline]
    fn offset(&self) -> u32 {
        self.position().offset
    }
}

pub trait HasSpan {
    fn span(&self) -> Span;

    #[inline]
    fn start_position(&self) -> Position {
        self.span().start
    }

    #[inline]
    fn start_offset(&self) -> u32 {
        self.start_position().offset
    }

    #[inline]
    fn end_position(&self) -> Position {
        self.span().end
    }

    #[inline]
    fn end_offset(&self) -> u32 {
        self.end_position().offset
    }
}

impl Position {
    #[inline]
    #[must_use]
    pub const fn new(offset: u32) -> Self {
        Self { offset }
    }

    #[inline]
    #[must_use]
    pub const fn zero() -> Self {
        Self { offset: 0 }
    }

    #[inline]
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.offset == 0
    }

    #[inline]
    #[must_use]
    pub const fn forward(&self, offset: u32) -> Self {
        Self {
            offset: self.offset.saturating_add(offset),
        }
    }

    #[inline]
    #[must_use]
    pub const fn backward(&self, offset: u32) -> Self {
        Self {
            offset: self.offset.saturating_sub(offset),
        }
    }

    #[inline]
    #[must_use]
    pub const fn range_for(&self, length: u32) -> Range<u32> {
        self.offset..self.offset.saturating_add(length)
    }
}

impl Span {
    #[inline]
    #[must_use]
    pub const fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    #[inline]
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            start: Position::zero(),
            end: Position::zero(),
        }
    }

    #[inline]
    #[must_use]
    pub const fn join(self, other: Self) -> Self {
        let start = if self.start.offset <= other.start.offset {
            self.start
        } else {
            other.start
        };
        let end = if self.end.offset >= other.end.offset {
            self.end
        } else {
            other.end
        };
        Self::new(start, end)
    }

    #[inline]
    pub fn contains(&self, position: &impl HasPosition) -> bool {
        self.has_offset(position.offset())
    }

    #[inline]
    #[must_use]
    pub const fn has_offset(&self, offset: u32) -> bool {
        self.start.offset <= offset && offset < self.end.offset
    }

    #[inline]
    #[must_use]
    pub const fn to_range(&self) -> Range<u32> {
        self.start.offset..self.end.offset
    }
}

impl HasPosition for Position {
    #[inline]
    fn position(&self) -> Position {
        *self
    }
}

impl HasPosition for u32 {
    #[inline]
    fn position(&self) -> Position {
        Position::new(*self)
    }
}

impl HasSpan for Span {
    #[inline]
    fn span(&self) -> Span {
        *self
    }
}

impl RangeBounds<u32> for Span {
    #[inline]
    fn start_bound(&self) -> Bound<&u32> {
        Bound::Included(&self.start.offset)
    }

    #[inline]
    fn end_bound(&self) -> Bound<&u32> {
        Bound::Excluded(&self.end.offset)
    }
}

impl<T: HasSpan> HasPosition for T {
    #[inline]
    fn position(&self) -> Position {
        self.start_position()
    }
}

impl<T: HasSpan> HasSpan for &T {
    #[inline]
    fn span(&self) -> Span {
        (*self).span()
    }
}

impl<T: HasSpan> HasSpan for Box<T> {
    #[inline]
    fn span(&self) -> Span {
        self.as_ref().span()
    }
}

impl From<Span> for Range<u32> {
    #[inline]
    fn from(span: Span) -> Self {
        span.to_range()
    }
}

impl From<&Span> for Range<u32> {
    #[inline]
    fn from(span: &Span) -> Self {
        span.to_range()
    }
}

impl From<Span> for Range<usize> {
    #[inline]
    fn from(span: Span) -> Self {
        let start = span.start.offset as usize;
        let end = span.end.offset as usize;

        start..end
    }
}

impl From<&Span> for Range<usize> {
    #[inline]
    fn from(span: &Span) -> Self {
        let start = span.start.offset as usize;
        let end = span.end.offset as usize;

        start..end
    }
}

impl From<Position> for u32 {
    #[inline]
    fn from(position: Position) -> Self {
        position.offset
    }
}

impl From<&Position> for u32 {
    #[inline]
    fn from(position: &Position) -> Self {
        position.offset
    }
}

impl From<u32> for Position {
    #[inline]
    fn from(offset: u32) -> Self {
        Self { offset }
    }
}

impl ops::Add<u32> for Position {
    type Output = Self;

    #[inline]
    fn add(self, rhs: u32) -> Self::Output {
        self.forward(rhs)
    }
}

impl ops::Sub<u32> for Position {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: u32) -> Self::Output {
        self.backward(rhs)
    }
}

impl ops::AddAssign<u32> for Position {
    #[inline]
    fn add_assign(&mut self, rhs: u32) {
        self.offset = self.offset.saturating_add(rhs);
    }
}

impl ops::SubAssign<u32> for Position {
    #[inline]
    fn sub_assign(&mut self, rhs: u32) {
        self.offset = self.offset.saturating_sub(rhs);
    }
}

impl fmt::Display for Position {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.offset)
    }
}

impl fmt::Display for Span {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start.offset, self.end.offset)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    fn span(start: u32, end: u32) -> Span {
        Span::new(Position::new(start), Position::new(end))
    }

    #[test]
    fn forward_and_backward_saturate_instead_of_overflowing() {
        assert_eq!(Position::new(u32::MAX).forward(10), Position::new(u32::MAX));
        assert_eq!(Position::new(0).backward(10), Position::new(0));
    }

    #[test]
    fn has_offset_is_end_exclusive() {
        let subject = span(2, 5);
        assert!(!subject.has_offset(1));
        assert!(subject.has_offset(2));
        assert!(subject.has_offset(4));
        assert!(!subject.has_offset(5), "the end offset is not contained");
        assert!(!subject.has_offset(6));
    }

    #[test]
    fn adjacent_spans_do_not_overlap() {
        let left = span(0, 3);
        let right = span(3, 6);
        assert!(left.has_offset(2) && !right.has_offset(2));
        assert!(!left.has_offset(3) && right.has_offset(3));
    }

    #[test]
    fn an_empty_span_contains_nothing() {
        let empty = span(4, 4);
        assert!(!empty.has_offset(3));
        assert!(!empty.has_offset(4));
        assert!(!empty.has_offset(5));
    }

    #[test]
    fn the_end_of_file_offset_is_outside_the_last_span() {
        let last = span(7, 10);
        assert!(last.has_offset(9));
        assert!(!last.has_offset(10));
    }

    #[test]
    fn join_covers_both_spans_in_either_order() {
        let left = span(2, 5);
        let right = span(8, 11);
        let forward = left.join(right);
        let backward = right.join(left);
        assert_eq!(forward, span(2, 11));
        assert_eq!(
            backward,
            span(2, 11),
            "a reversed join still covers both spans"
        );
    }

    #[test]
    fn join_of_overlapping_and_nested_spans_covers_them() {
        assert_eq!(span(0, 6).join(span(4, 9)), span(0, 9));
        assert_eq!(span(4, 9).join(span(0, 6)), span(0, 9));
        assert_eq!(span(0, 10).join(span(3, 5)), span(0, 10));
        assert_eq!(span(3, 5).join(span(0, 10)), span(0, 10));
    }

    #[test]
    fn contains_follows_the_end_exclusive_rule() {
        let subject = span(1, 4);
        assert!(subject.contains(&Position::new(1)));
        assert!(subject.contains(&Position::new(3)));
        assert!(!subject.contains(&Position::new(4)));
    }
}
