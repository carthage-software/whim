use lsp_types::Position;
use lsp_types::SelectionRange;

use crate::server::analysis::Analysis;

pub(super) fn ranges(analysis: &Analysis<'_>, positions: &[Position]) -> Vec<SelectionRange> {
    positions
        .iter()
        .copied()
        .map(|position| chain(analysis, position))
        .collect()
}

fn chain(analysis: &Analysis<'_>, position: Position) -> SelectionRange {
    let offset = analysis.lines().offset(position);
    let mut widening: Vec<(usize, usize)> = analysis
        .enclosing(offset)
        .into_iter()
        .map(|element| (element.start, element.end))
        .filter(|(start, end)| start < end)
        .collect();

    widening.dedup();
    let mut parent = None;
    for (start, end) in widening.into_iter().rev() {
        parent = Some(Box::new(SelectionRange {
            range: analysis.lines().range(start, end),
            parent,
        }));
    }

    parent.map_or_else(
        || SelectionRange {
            range: analysis.lines().range(offset, offset),
            parent: None,
        },
        |range| *range,
    )
}

#[cfg(test)]
mod tests {
    use super::ranges;
    use crate::server::analysis::Analysis;

    #[test]
    fn selections_expand_through_the_syntax_tree() {
        let source = "function work(): void { $total = 1 + 2; }";
        let analysis = Analysis::new(source);
        let offset = source.find("total").expect("the variable");
        let found = ranges(&analysis, &[analysis.lines().position(offset)]);
        let mut count = 0;
        let mut current = found.first();
        while let Some(range) = current {
            count += 1;
            current = range.parent.as_deref();
        }
        assert!(count > 3);
    }

    #[test]
    fn invalid_source_returns_the_cursor_as_a_valid_range() {
        let analysis = Analysis::new("final class {");
        let position = analysis.lines().position(3);
        let found = ranges(&analysis, &[position]);
        assert_eq!(found[0].range.start, position);
        assert_eq!(found[0].range.end, position);
    }
}
