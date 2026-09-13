use std::borrow::Cow;

use annotate_snippets::Annotation;
use annotate_snippets::AnnotationKind;
use annotate_snippets::Element;
use annotate_snippets::Group;
use annotate_snippets::Level;
use annotate_snippets::Snippet;

use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::arena::Vec as ArenaVec;
use whim_syn::cst::Program;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::registry::RuleRegistry;
use crate::rule_meta::RuleMeta;
use crate::scope::ScopeStack;

pub type DiagnosticCallback<'callback> = dyn FnMut(Span, &str, &Level<'static>, &str) + 'callback;

pub struct LintContext<'ctx, 'arena, A: Arena> {
    pub arena: &'arena A,
    pub registry: &'ctx RuleRegistry,
    pub program: &'ctx Program<'arena>,
    pub source: &'arena str,
    pub path: &'arena str,
    pub diagnostics: Vec<Group<'arena>>,
    pub scope: ScopeStack<'arena, A>,
    pub(crate) on_diagnostic: Option<&'ctx mut DiagnosticCallback<'ctx>>,
    ancestors: ArenaVec<'arena, Node<'ctx, 'arena>, A>,
}

impl<'ctx, 'arena, A: Arena> LintContext<'ctx, 'arena, A> {
    pub fn new(
        arena: &'arena A,
        registry: &'ctx RuleRegistry,
        path: &'arena str,
        program: &'ctx Program<'arena>,
    ) -> Self {
        Self {
            arena,
            registry,
            program,
            source: program.source_text,
            path,
            diagnostics: Vec::new(),
            scope: ScopeStack::new_in(arena),
            on_diagnostic: None,
            ancestors: ArenaVec::with_capacity_in(32, arena),
        }
    }

    pub fn report(
        &mut self,
        meta: &RuleMeta,
        level: Level<'static>,
        (span, label): (Span, impl Into<Cow<'arena, str>>),
        message: impl Into<Cow<'arena, str>>,
        annotations: impl IntoIterator<Item = Annotation<'arena>>,
        details: impl IntoIterator<Item = Element<'arena>>,
    ) {
        let message = message.into();
        if let Some(callback) = &mut self.on_diagnostic {
            callback(span, meta.code, &level, &message);
        }

        let annotation = Snippet::source(self.source)
            .path(self.path)
            .fold(true)
            .annotation(
                AnnotationKind::Primary
                    .span(span.into())
                    .label(label.into())
                    .highlight_source(true),
            )
            .annotations(annotations);

        self.diagnostics.push(
            level
                .primary_title(message)
                .id(meta.code)
                .element(annotation)
                .elements(details),
        );
    }

    pub(crate) fn push_ancestor(&mut self, node: Node<'ctx, 'arena>) {
        self.ancestors.push(node);
    }

    pub(crate) fn pop_ancestor(&mut self) {
        self.ancestors.pop();
    }

    #[must_use]
    pub fn source_for(&self, span: Span) -> &'arena str {
        &self.source[span.start.offset as usize..span.end.offset as usize]
    }

    #[must_use]
    pub fn get_parent(&self) -> Option<Node<'ctx, 'arena>> {
        self.get_nth_parent(0)
    }

    #[must_use]
    pub fn get_nth_parent(&self, n: usize) -> Option<Node<'ctx, 'arena>> {
        self.ancestors
            .len()
            .checked_sub(n.checked_add(2)?)
            .map(|index| self.ancestors[index])
    }

    #[must_use]
    pub fn is_child_of(&self, kind: NodeKind) -> bool {
        self.ancestors[..self.ancestors.len().saturating_sub(1)]
            .iter()
            .any(|node| node.kind() == kind)
    }
}
