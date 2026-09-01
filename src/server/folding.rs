use lsp_types::FoldingRange;
use lsp_types::FoldingRangeKind;
use whim_syn::cst::node::NodeKind;
use whim_syn::token::kind::TokenKind;

use crate::server::analysis::Analysis;

pub(super) fn ranges(analysis: &Analysis<'_>) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    for element in analysis
        .elements()
        .iter()
        .filter(|element| foldable(element.kind))
    {
        push(&mut ranges, analysis, element.start, element.end, None);
    }

    for token in analysis.tokens() {
        if matches!(
            token.kind,
            TokenKind::MultiLineComment | TokenKind::DocBlockComment
        ) {
            let start = token.start.offset as usize;
            push(
                &mut ranges,
                analysis,
                start,
                start + token.value.len(),
                Some(FoldingRangeKind::Comment),
            );
        }
    }

    ranges
}

const fn foldable(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Block
            | NodeKind::MethodBody
            | NodeKind::NamespaceBody
            | NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Enum
            | NodeKind::Function
            | NodeKind::Method
            | NodeKind::Match
            | NodeKind::VecExpression
            | NodeKind::DictExpression
            | NodeKind::TupleExpression
            | NodeKind::AttributeList
    )
}

fn push(
    ranges: &mut Vec<FoldingRange>,
    analysis: &Analysis<'_>,
    start: usize,
    end: usize,
    kind: Option<FoldingRangeKind>,
) {
    let first = analysis.lines().line_of(start);
    let last = analysis.lines().line_of(end.saturating_sub(1));
    if last <= first {
        return;
    }

    let end_line = match kind {
        Some(FoldingRangeKind::Comment) => last,
        _ => last - 1,
    };

    ranges.push(FoldingRange {
        start_line: first,
        end_line,
        kind,
        ..FoldingRange::default()
    });
}

#[cfg(test)]
mod tests {
    use lsp_types::FoldingRangeKind;

    use super::ranges;
    use crate::server::analysis::Analysis;

    #[test]
    fn blocks_and_comments_fold_without_hiding_the_closing_line() {
        let source = "\
/* one
   two */
final class Holder {
  public function work(): void {
    $value = 1;
  }
}
";
        let ranges = ranges(&Analysis::new(source));
        assert!(ranges.iter().any(|range| {
            range.start_line == 0
                && range.end_line == 1
                && range.kind == Some(FoldingRangeKind::Comment)
        }));
        assert!(
            ranges
                .iter()
                .any(|range| range.start_line == 2 && range.end_line == 5)
        );
    }
}
