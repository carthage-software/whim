use annotate_snippets::Level;
use indoc::indoc;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::BinaryOperator;
use whim_syn::cst::operation::UnaryPrefixOperator;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct ConstantConditionRule {
    meta: &'static RuleMeta,
    cfg: ConstantConditionConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct ConstantConditionConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for ConstantConditionConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for ConstantConditionConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for ConstantConditionRule {
    type Config = ConstantConditionConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Constant Condition",
            code: "constant-condition",
            description: "Detects if conditions whose Boolean result is known from the source.",
            good_example: "if ($ready) { run(); }",
            bad_example: indoc! {r"
                if (true) { run(); }
            "},
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::If]
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
        let Node::If(statement) = node else {
            return;
        };

        let Some(value) = constant_bool(statement.condition) else {
            return;
        };

        let value = if value { "true" } else { "false" };
        ctx.report(
            self.meta,
            self.cfg.level(),
            (
                statement.condition.span(),
                format!("this condition is always `{value}`"),
            ),
            "Redundant `if` statement.",
            [],
            [
                Level::NOTE
                    .message(if value == "true" {
                        "The body always runs."
                    } else {
                        "The body never runs."
                    })
                    .into(),
                Level::HELP
                    .message(
                        "Remove the dead branch or replace the condition with the intended check.",
                    )
                    .into(),
            ],
        );
    }
}

fn constant_bool(expression: &Expression<'_>) -> Option<bool> {
    match expression.unparenthesized() {
        Expression::Literal(Literal::True(_)) => Some(true),
        Expression::Literal(Literal::False(_)) => Some(false),
        Expression::UnaryPrefix(prefix)
            if matches!(prefix.operator, UnaryPrefixOperator::Not(_)) =>
        {
            constant_bool(prefix.operand).map(|value| !value)
        }
        Expression::Binary(binary) => match binary.operator {
            BinaryOperator::And(_) => match constant_bool(binary.lhs) {
                Some(false) => Some(false),
                Some(true) => constant_bool(binary.rhs),
                None => None,
            },
            BinaryOperator::Or(_) => match constant_bool(binary.lhs) {
                Some(true) => Some(true),
                Some(false) => constant_bool(binary.rhs),
                None => None,
            },
            BinaryOperator::NullCoalesce(_) => constant_bool(binary.lhs),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::ConstantConditionRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = boolean_constants_and_logic_are_rejected,
        rule = ConstantConditionRule,
        count = 4,
        code = "if (true) {} if (!false) {} if (false && effect()) {} if (true || effect()) {}",
    }

    test_lint_success! {
        name = dynamic_and_non_boolean_values_are_not_folded,
        rule = ConstantConditionRule,
        code = "if ($ready) {} if (1 / 1) {} if ('') {} if (1 && false) {} if (effect() || true) {} if ($x = true) {}",
    }
}
