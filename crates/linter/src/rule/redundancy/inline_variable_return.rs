use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::AssignmentOperator;
use whim_syn::cst::operation::AssignmentTarget;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::statement::TopLevelStatement;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct InlineVariableReturnRule {
    meta: &'static RuleMeta,
    cfg: InlineVariableReturnConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct InlineVariableReturnConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for InlineVariableReturnConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for InlineVariableReturnConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for InlineVariableReturnRule {
    type Config = InlineVariableReturnConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Inline Variable Return",
            code: "inline-variable-return",
            description: indoc! {"
                Finds a local assignment followed at once by a return of that local.
                The assigned expression can be returned directly.
                Cases where an enclosing finally block mentions the local are skipped.
            "},
            good_example: "function value(): int { return compute(); }",
            bad_example: "function value(): int { $result = compute(); return $result; }",
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[
            NodeKind::Program,
            NodeKind::Block,
            NodeKind::NamespaceImplicitBody,
            NodeKind::NamespaceBraceDelimitedBody,
        ]
    }

    fn build(settings: &RuleSettings<Self::Config>) -> Self {
        Self {
            meta: Self::meta(),
            cfg: settings.config.clone(),
        }
    }

    fn check<'arena, A: Arena>(
        &self,
        ctx: &mut LintContext<'_, 'arena, A>,
        node: Node<'_, 'arena>,
    ) {
        let statements = match node {
            Node::Program(program) => program.statements,
            Node::Block(block) => {
                for pair in block.statements.windows(2) {
                    self.check_pair(ctx, &pair[0], &pair[1]);
                }

                return;
            }
            Node::NamespaceImplicitBody(body) => body.statements,
            Node::NamespaceBraceDelimitedBody(body) => body.statements,
            _ => return,
        };

        for pair in statements.windows(2) {
            if let [
                TopLevelStatement::Statement(left),
                TopLevelStatement::Statement(right),
            ] = pair
            {
                self.check_pair(ctx, left, right);
            }
        }
    }
}

impl InlineVariableReturnRule {
    fn check_pair<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        left: &Statement<'_>,
        right: &Statement<'_>,
    ) {
        let (Statement::Expression(assignment_statement), Statement::Expression(return_statement)) =
            (left, right)
        else {
            return;
        };

        let Expression::Assignment(assignment) = assignment_statement.expression.unparenthesized()
        else {
            return;
        };

        let (AssignmentOperator::Assign(_), AssignmentTarget::Variable(variable)) =
            (assignment.operator, &assignment.target)
        else {
            return;
        };

        let Expression::Return(return_expression) = return_statement.expression.unparenthesized()
        else {
            return;
        };

        let Some(value) = return_expression.value else {
            return;
        };

        let Expression::Variable(returned) = value.unparenthesized() else {
            return;
        };

        if returned.name != variable.name || finally_mentions(ctx, variable.name) {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            (variable.span, "temporary assigned here"),
            format!("Variable `{}` can be returned directly.", variable.name),
            [AnnotationKind::Context
                .span(return_expression.span().into())
                .label("returned without any intervening use")],
            [
                Level::NOTE
                    .message(
                        "Whim collections have value semantics, so this does not change aliasing.",
                    )
                    .into(),
                Level::HELP
                    .message(format!(
                        "Return `{}` directly.",
                        ctx.source_for(assignment.value.span())
                    ))
                    .into(),
            ],
        );
    }
}

fn finally_mentions<A: Arena>(ctx: &LintContext<'_, '_, A>, name: &str) -> bool {
    let mut depth = 0;
    while let Some(ancestor) = ctx.get_nth_parent(depth) {
        depth += 1;
        let Node::Try(statement) = ancestor else {
            continue;
        };
        let Some(finally) = &statement.finally_clause else {
            continue;
        };
        let mut stack = vec![Node::Block(&finally.block)];
        while let Some(node) = stack.pop() {
            if matches!(node, Node::Variable(variable) if variable.name == name) {
                return true;
            }
            node.visit_children(&mut |child| stack.push(child));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::InlineVariableReturnRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = reports_immediate_variable_return,
        rule = InlineVariableReturnRule,
        code = "function f(): int { $result = compute(); return $result; }",
    }

    test_lint_success! {
        name = keeps_intervening_use,
        rule = InlineVariableReturnRule,
        code = "function f(): int { $result = compute(); debug!($result); return $result; }",
    }

    test_lint_success! {
        name = keeps_value_read_by_finally,
        rule = InlineVariableReturnRule,
        code = "function f(): int { try { $result = compute(); return $result; } finally { debug!($result); } }",
    }

    test_lint_failure! {
        name = unrelated_finally_value_does_not_block_report,
        rule = InlineVariableReturnRule,
        code = "function f(): int { try { $result = compute(); return $result; } finally { debug!($other); } }",
    }
}
