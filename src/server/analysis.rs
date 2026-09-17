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
            };

            walk(Node::Program(program), &mut collector);
        }

        Self {
            lines: LineIndex::new(source),
            elements,
            tokens: tokenize(source),
        }
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
}

struct Collector<'elements> {
    elements: &'elements mut Vec<Element>,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for Collector<'_> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        let span = node.span();
        self.elements.push(Element {
            kind: node.kind(),
            start: span.start.offset as usize,
            end: span.end.offset as usize,
        });
        Flow::Descend
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
    fn syntax_element_ends_are_exclusive() {
        let element = Element {
            kind: NodeKind::Program,
            start: 2,
            end: 5,
        };

        assert!(element.contains(2));
        assert!(element.contains(4));
        assert!(!element.contains(5));
    }
}
