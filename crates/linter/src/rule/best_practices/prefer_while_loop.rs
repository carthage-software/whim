use annotate_snippets::Level;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct PreferWhileLoopRule {
    meta: &'static RuleMeta,
    cfg: PreferWhileLoopConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct PreferWhileLoopConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for PreferWhileLoopConfig {
    fn default() -> Self {
        Self { level: Level::NOTE }
    }
}

impl Config for PreferWhileLoopConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for PreferWhileLoopRule {
    type Config = PreferWhileLoopConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Prefer While Loop",
            code: "prefer-while-loop",
            description: "Suggests a while loop when a for loop has no initializer or increment.",
            good_example: "while ($ready) { work(); }",
            bad_example: "for (; $ready;) { work(); }",
            category: Category::BestPractices,
        };
        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::For]
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
        let Node::For(statement) = node else {
            return;
        };

        if !statement.initializations.is_empty() || !statement.increments.is_empty() {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            (statement.span(), "this for loop has only a condition"),
            "Use a `while` loop here.",
            [],
            [
                Level::NOTE
                    .message(
                        "A for loop adds no useful structure without an initializer or increment.",
                    )
                    .into(),
                Level::HELP
                    .message(if statement.conditions.is_empty() {
                        "Write `while (true)` for this unbounded loop."
                    } else {
                        "Move the condition into a `while` loop."
                    })
                    .into(),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::PreferWhileLoopRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = condition_only_for_loop_is_rejected,
        rule = PreferWhileLoopRule,
        code = "for (; $ready;) { work(); }",
    }

    test_lint_success! {
        name = full_for_loop_is_allowed,
        rule = PreferWhileLoopRule,
        code = "for ($i = 0; $i < 10; $i++) { work($i); }",
    }
}
