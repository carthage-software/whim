use lsp_types::Position;
use lsp_types::Range;

pub(super) struct LineIndex<'source> {
    starts: Vec<usize>,
    source: &'source str,
}

impl<'source> LineIndex<'source> {
    pub(super) fn new(source: &'source str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );

        Self { starts, source }
    }

    pub(super) fn line_of(&self, offset: usize) -> u32 {
        let offset = offset.min(self.source.len());
        u32::try_from(self.starts.partition_point(|start| *start <= offset) - 1).unwrap_or(u32::MAX)
    }

    pub(super) fn position(&self, offset: usize) -> Position {
        let offset = self.clamp_to_boundary(offset);
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let offset = offset.min(self.line_end(line));
        let character = self.source[self.starts[line]..offset]
            .encode_utf16()
            .count();

        Position::new(
            u32::try_from(line).unwrap_or(u32::MAX),
            u32::try_from(character).unwrap_or(u32::MAX),
        )
    }

    pub(super) fn offset(&self, position: Position) -> usize {
        let Some(start) = self.starts.get(position.line as usize) else {
            return self.source.len();
        };

        let end = self.line_end(position.line as usize);

        let mut remaining = position.character as usize;
        let mut offset = *start;
        for character in self.source[*start..end].chars() {
            if remaining == 0 {
                break;
            }

            remaining = remaining.saturating_sub(character.len_utf16());
            offset += character.len_utf8();
        }

        offset.min(end)
    }

    pub(super) fn range(&self, start: usize, end: usize) -> Range {
        Range::new(self.position(start), self.position(end))
    }

    fn clamp_to_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.source.len());
        while offset > 0 && !self.source.is_char_boundary(offset) {
            offset -= 1;
        }

        offset
    }

    fn line_end(&self, line: usize) -> usize {
        let Some(next) = self.starts.get(line + 1).copied() else {
            return self.source.len();
        };

        let mut end = next - 1;
        if end > 0 && self.source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }

        end
    }
}

#[cfg(test)]
mod tests {
    use lsp_types::Position;

    use super::LineIndex;

    #[test]
    fn positions_use_utf16_code_units() {
        let index = LineIndex::new("é🙂x\nsecond");
        assert_eq!(index.position(2), Position::new(0, 1));
        assert_eq!(index.position(6), Position::new(0, 3));
        assert_eq!(index.position(8), Position::new(1, 0));
    }

    #[test]
    fn positions_round_trip_at_character_boundaries() {
        let source = "let é = 1;\n🙂 second\nthird";
        let index = LineIndex::new(source);
        for offset in (0..source.len()).filter(|offset| source.is_char_boundary(*offset)) {
            assert_eq!(index.offset(index.position(offset)), offset);
        }
    }

    #[test]
    fn invalid_positions_clamp_to_the_document() {
        let index = LineIndex::new("ab\ncd");
        assert_eq!(index.offset(Position::new(0, 99)), 2);
        assert_eq!(index.offset(Position::new(99, 0)), 5);
    }

    #[test]
    fn line_terminators_do_not_count_as_characters() {
        let index = LineIndex::new("ab\r\ncd");
        assert_eq!(index.offset(Position::new(0, 99)), 2);
        assert_eq!(index.position(2), Position::new(0, 2));
        assert_eq!(index.position(3), Position::new(0, 2));
        assert_eq!(index.position(4), Position::new(1, 0));
    }
}
