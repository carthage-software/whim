use annotate_snippets::Level;
use indoc::indoc;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::access::Access;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::AssignmentTarget;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoSelfAssignmentRule {
    meta: &'static RuleMeta,
    cfg: NoSelfAssignmentConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoSelfAssignmentConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoSelfAssignmentConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoSelfAssignmentConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoSelfAssignmentRule {
    type Config = NoSelfAssignmentConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Self Assignment",
            code: "no-self-assignment",
            description: "Detects standalone assignments that assign a variable or property to itself.",
            good_example: indoc! {r"
                $a = $b;
                $this->x = $other->x;
            "},
            bad_example: indoc! {r"
                $a = $a;
                $this->x = $this->x;
            "},
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::ExpressionStatement]
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
        let Node::ExpressionStatement(statement) = node else {
            return;
        };

        let Expression::Assignment(assignment) = statement.expression.unparenthesized() else {
            return;
        };

        if !assignment.operator.is_assign()
            || !target_matches_expression(&assignment.target, assignment.value)
        {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            (
                assignment.span(),
                "this assignment writes a value back to itself",
            ),
            "Self-assignment has no effect.",
            [],
            [
                Level::NOTE
                    .message("Self-assignments often remain after a rename or refactor.")
                    .into(),
                Level::HELP
                    .message("Remove this assignment, or assign the intended value.")
                    .into(),
            ],
        );
    }
}

fn target_matches_expression(target: &AssignmentTarget<'_>, value: &Expression<'_>) -> bool {
    match (target, value.unparenthesized()) {
        (AssignmentTarget::Variable(left), Expression::Variable(right)) => left.name == right.name,
        (AssignmentTarget::Property(left), Expression::Access(Access::Property(right))) => {
            left.property.value == right.property.value
                && expressions_are_equivalent(left.object, right.object)
        }
        _ => false,
    }
}

fn expressions_are_equivalent(left: &Expression<'_>, right: &Expression<'_>) -> bool {
    match (left.unparenthesized(), right.unparenthesized()) {
        (Expression::Variable(left), Expression::Variable(right)) => left.name == right.name,
        (
            Expression::Access(Access::Property(left)),
            Expression::Access(Access::Property(right)),
        ) => {
            left.property.value == right.property.value
                && expressions_are_equivalent(left.object, right.object)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::NoSelfAssignmentRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = direct_and_property_self_assignments,
        rule = NoSelfAssignmentRule,
        count = 2,
        code = "$a = $a; $this->item->name = $this->item->name;",
    }

    test_lint_success! {
        name = assignments_with_distinct_values,
        rule = NoSelfAssignmentRule,
        code = "$a = $b; $this->name = $other->name; $a += $a;",
    }
}
