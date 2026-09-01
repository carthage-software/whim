use std::ops::Range;

use whim_span::HasSpan;
use whim_syn::arena::LocalArena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;
use whim_syn::input::Input;
use whim_syn::lexer::Lexer;
use whim_syn::parser;
use whim_syn::token::Token;
use whim_syn::token::kind::TokenKind;

use crate::server::text::LineIndex;

#[derive(Clone, Copy)]
pub(super) struct Element {
    pub(super) kind: NodeKind,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) depth: usize,
}

impl Element {
    pub(super) const fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    const fn width(self) -> usize {
        self.end - self.start
    }
}

pub(super) struct Analysis<'source> {
    source: &'source str,
    lines: LineIndex<'source>,
    elements: Vec<Element>,
    tokens: Vec<Token<'source>>,
}

impl<'source> Analysis<'source> {
    pub(super) fn new(source: &'source str) -> Self {
        let arena = LocalArena::new();
        let mut elements = Vec::new();
        if let Ok(program) = parser::parse(&arena, source) {
            let mut collector = Collector {
                elements: &mut elements,
                depth: 0,
            };

            walk(Node::Program(program), &mut collector);
            elements.sort_by_key(|element| (element.start, element.depth));
        }

        Self {
            source,
            lines: LineIndex::new(source),
            elements,
            tokens: tokenize(source),
        }
    }

    pub(super) const fn source(&self) -> &'source str {
        self.source
    }

    pub(super) const fn lines(&self) -> &LineIndex<'source> {
        &self.lines
    }

    pub(super) fn elements(&self) -> &[Element] {
        &self.elements
    }

    pub(super) fn tokens(&self) -> &[Token<'source>] {
        &self.tokens
    }

    pub(super) fn token_at(&self, offset: usize) -> Option<&Token<'source>> {
        let after = self
            .tokens
            .partition_point(|token| token.start.offset as usize <= offset);
        let index = after.checked_sub(1)?;
        let token = &self.tokens[index];
        let start = token.start.offset as usize;

        (offset < start + token.value.len()).then_some(token)
    }

    pub(super) fn enclosing(&self, offset: usize) -> Vec<Element> {
        let mut found: Vec<Element> = self
            .elements
            .iter()
            .copied()
            .filter(|element| element.contains(offset))
            .collect();
        found.sort_by_key(|element| element.width());
        found
    }

    pub(super) fn enclosing_scope(&self, offset: usize) -> Range<usize> {
        self.enclosing(offset)
            .into_iter()
            .find(|element| is_scope(element.kind))
            .map_or(0..self.source.len(), |element| element.start..element.end)
    }

    pub(super) fn follows_member_operator(&self, offset: usize) -> bool {
        let after = self
            .tokens
            .partition_point(|token| token.start.offset as usize <= offset);
        self.tokens[..after.saturating_sub(1)]
            .iter()
            .rev()
            .find(|token| !token.kind.is_comment())
            .is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenKind::MinusGreaterThan
                        | TokenKind::QuestionMinusGreaterThan
                        | TokenKind::ColonColon
                        | TokenKind::ColonColonLessThan
                )
            })
    }
}

struct Collector<'elements> {
    elements: &'elements mut Vec<Element>,
    depth: usize,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for Collector<'_> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        let span = node.span();
        self.elements.push(Element {
            kind: node.kind(),
            start: span.start.offset as usize,
            end: span.end.offset as usize,
            depth: self.depth,
        });
        self.depth += 1;
        Flow::Descend
    }

    fn leave(&mut self, _node: Node<'ast, 'arena>) {
        self.depth -= 1;
    }
}

fn tokenize(source: &str) -> Vec<Token<'_>> {
    let arena = LocalArena::new();
    let mut lexer = Lexer::new(&arena, Input::new(source));
    let mut tokens = Vec::new();
    while let Some(Ok(token)) = lexer.advance() {
        if token.kind != TokenKind::Whitespace {
            tokens.push(token);
        }
    }

    tokens
}

pub(super) const fn is_identifier(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Identifier
            | NodeKind::LocalIdentifier
            | NodeKind::QualifiedIdentifier
            | NodeKind::FullyQualifiedIdentifier
    )
}

pub(super) const fn is_name_token(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier
            | TokenKind::QualifiedIdentifier
            | TokenKind::FullyQualifiedIdentifier
    )
}

const fn is_scope(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Function | NodeKind::Method | NodeKind::ShortClosure | NodeKind::Closure
    )
}

#[cfg(test)]
mod tests {
    use whim_syn::cst::node::NodeKind;

    use super::Analysis;
    use super::Element;

    const SOURCE: &str = "\
final class Holder {
  public function first(): int {
    $total = 1;
    return $total;
  }
}
";

    #[test]
    fn invalid_source_keeps_lexical_tokens() {
        let analysis = Analysis::new("final class {");
        assert!(analysis.elements().is_empty());
        assert!(!analysis.tokens().is_empty());
    }

    #[test]
    fn enclosing_elements_are_ordered_inside_out() {
        let analysis = Analysis::new(SOURCE);
        let offset = SOURCE.find("$total").expect("the local variable");
        let elements = analysis.enclosing(offset);
        assert!(
            elements
                .windows(2)
                .all(|pair| pair[0].width() <= pair[1].width())
        );
        assert!(
            elements
                .iter()
                .any(|element| element.kind == NodeKind::Method)
        );
    }

    #[test]
    fn variables_use_the_enclosing_function_as_their_scope() {
        let analysis = Analysis::new(SOURCE);
        let offset = SOURCE.find("$total").expect("the local variable");
        let scope = analysis.enclosing_scope(offset);
        assert!(scope.start > 0);
        assert!(scope.end < SOURCE.len());
    }

    #[test]
    fn syntax_element_ends_are_exclusive() {
        let element = Element {
            kind: NodeKind::Program,
            start: 2,
            end: 5,
            depth: 0,
        };

        assert!(element.contains(2));
        assert!(element.contains(4));
        assert!(!element.contains(5));
    }
}
