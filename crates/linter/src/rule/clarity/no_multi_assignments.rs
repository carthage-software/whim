use annotate_snippets::Level;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoMultiAssignmentsRule {
    meta: &'static RuleMeta,
    cfg: NoMultiAssignmentsConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoMultiAssignmentsConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoMultiAssignmentsConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoMultiAssignmentsConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoMultiAssignmentsRule {
    type Config = NoMultiAssignmentsConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Multi Assignments",
            code: "no-multi-assignments",
            description: "Flags chained assignments in one expression.",
            good_example: "$b = 0; $a = $b;",
            bad_example: "$a = $b = 0;",
            category: Category::Clarity,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Assignment]
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
        let Node::Assignment(assignment) = node else {
            return;
        };

        if !matches!(
            assignment.value.unparenthesized(),
            Expression::Assignment(_)
        ) {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            (
                assignment.span(),
                "this expression assigns more than one target",
            ),
            "Avoid multiple assignments in one expression.",
            [],
            [
                Level::NOTE
                    .message("Chained assignments can hide evaluation order and intent.")
                    .into(),
                Level::HELP
                    .message("Split this chain into separate assignment statements.")
                    .into(),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::NoMultiAssignmentsRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = chained_assignment_is_rejected,
        rule = NoMultiAssignmentsRule,
        code = "$a = ($b = 0);",
    }

    test_lint_success! {
        name = separate_assignments_are_allowed,
        rule = NoMultiAssignmentsRule,
        code = "$b = 0; $a = $b;",
    }
}
