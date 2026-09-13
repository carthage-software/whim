#![forbid(unsafe_code)]

mod attributes;
pub mod category;
pub mod context;
pub mod registry;
pub mod rule;
pub mod rule_meta;
pub mod scope;
pub mod settings;

use std::path::Path;
use std::sync::Arc;

use annotate_snippets::Group;

use whim_syn::arena::Arena;
use whim_syn::arena::Vec as ArenaVec;
use whim_syn::cst::Program;
use whim_syn::cst::node::Node;

use crate::attributes::AttributeScopes;
use crate::context::DiagnosticCallback;
use crate::context::LintContext;
use crate::registry::RuleRegistry;
use crate::rule::AnyRule;
use crate::scope::Scope;
use crate::settings::Settings;

pub struct Linter<'arena, A: Arena> {
    arena: &'arena A,
    registry: Arc<RuleRegistry>,
}

impl<'arena, A: Arena> Linter<'arena, A> {
    pub fn new(
        arena: &'arena A,
        settings: &Settings,
        only: Option<&[String]>,
        include_disabled: bool,
    ) -> Result<Self, globset::Error> {
        Ok(Self {
            arena,
            registry: Arc::new(RuleRegistry::build(settings, only, include_disabled)?),
        })
    }

    #[must_use]
    pub fn from_registry(arena: &'arena A, registry: Arc<RuleRegistry>) -> Self {
        Self { arena, registry }
    }

    #[must_use]
    pub fn rules(&self) -> &[AnyRule] {
        self.registry.rules()
    }

    #[must_use]
    pub fn lint(&self, path: &'arena str, program: &Program<'arena>) -> Vec<Group<'arena>> {
        self.lint_with_diagnostics(path, Path::new(path), program, None)
    }

    pub fn lint_with_diagnostics<'ctx>(
        &'ctx self,
        path: &'arena str,
        matching_path: &Path,
        program: &'ctx Program<'arena>,
        on_diagnostic: Option<&'ctx mut DiagnosticCallback<'ctx>>,
    ) -> Vec<Group<'arena>> {
        let mut context = LintContext::new(self.arena, &self.registry, path, program);
        context.on_diagnostic = on_diagnostic;
        context.attributes = AttributeScopes::collect(&mut context);
        let mut excluded_rules = ArenaVec::new_in(self.arena);
        for (index, rule) in self.registry.all_rules().iter().enumerate() {
            if self.registry.excludes(index, matching_path)
                || (index >= self.registry.len() && !context.attributes.enables(rule.code()))
            {
                excluded_rules.push(index);
            }
        }

        walk(Node::Program(program), &mut context, &excluded_rules);
        context.diagnostics
    }
}

fn walk<'ctx, 'arena, A: Arena>(
    root: Node<'ctx, 'arena>,
    ctx: &mut LintContext<'ctx, 'arena, A>,
    excluded_rules: &[usize],
) {
    enum Op<'ctx, 'arena> {
        Enter(Node<'ctx, 'arena>),
        Exit { in_scope: bool },
    }

    let mut stack = ArenaVec::with_capacity_in(64, ctx.arena);
    stack.push(Op::Enter(root));
    while let Some(op) = stack.pop() {
        match op {
            Op::Enter(node) => {
                ctx.push_ancestor(node);
                let in_scope = if let Some(scope) = Scope::for_node(node) {
                    ctx.scope.push(scope);
                    true
                } else {
                    false
                };

                for &index in ctx.registry.for_kind(node.kind()) {
                    if !excluded_rules.contains(&index) {
                        ctx.registry.rule(index).check(ctx, node);
                    }
                }

                stack.push(Op::Exit { in_scope });
                let start = stack.len();
                node.visit_children(&mut |child| stack.push(Op::Enter(child)));
                stack[start..].reverse();
            }
            Op::Exit { in_scope } => {
                if in_scope {
                    ctx.scope.pop();
                }

                ctx.pop_ancestor();
            }
        }
    }
}
