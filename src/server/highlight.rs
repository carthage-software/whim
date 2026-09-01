use lsp_types::DocumentHighlight;
use lsp_types::DocumentHighlightKind;
use lsp_types::Position;
use whim_syn::token::kind::TokenKind;

use crate::server::analysis::Analysis;
use crate::server::analysis::is_name_token;

pub(super) fn occurrences(analysis: &Analysis<'_>, position: Position) -> Vec<DocumentHighlight> {
    let offset = analysis.lines().offset(position);
    let Some(token) = analysis.token_at(offset) else {
        return Vec::new();
    };

    let scope = if token.kind == TokenKind::Variable {
        analysis.enclosing_scope(offset)
    } else if is_name_token(token.kind) {
        0..analysis.source().len()
    } else {
        return Vec::new();
    };

    analysis
        .tokens()
        .iter()
        .filter(|other| {
            other.kind == token.kind
                && other.value == token.value
                && scope.contains(&(other.start.offset as usize))
        })
        .map(|other| {
            let start = other.start.offset as usize;
            DocumentHighlight {
                range: analysis.lines().range(start, start + other.value.len()),
                kind: Some(DocumentHighlightKind::TEXT),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::occurrences;
    use crate::server::analysis::Analysis;

    #[test]
    fn variables_are_scoped_to_their_function() {
        let source = "\
function first(): void { $value = 1; write_line!($value); }
function second(): void { $value = 2; write_line!($value); }
";
        let analysis = Analysis::new(source);
        let offset = source.find("$value").expect("the first variable");
        let found = occurrences(&analysis, analysis.lines().position(offset));
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|item| item.range.start.line == 0));
    }
}
