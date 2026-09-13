use annotate_snippets::Level;
use indoc::indoc;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::early_exit::EarlyExitPattern;
use crate::rule::utils::early_exit::check_early_exit;
use crate::rule::utils::early_exit::extract_single_if;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct PreferEarlyContinueRule {
    meta: &'static RuleMeta,
    cfg: PreferEarlyContinueConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct PreferEarlyContinueConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    pub max_allowed_statements: usize,
}

impl Default for PreferEarlyContinueConfig {
    fn default() -> Self {
        Self {
            level: Level::HELP,
            max_allowed_statements: 0,
        }
    }
}

impl Config for PreferEarlyContinueConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for PreferEarlyContinueRule {
    type Config = PreferEarlyContinueConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Prefer Early Continue",
            code: "prefer-early-continue",
            description: "Suggests an early continue when one if statement wraps a loop body.",
            good_example: indoc! {r"
                foreach ($items as $item) {
                    if (!$item->ready) { continue; }
                    work($item);
                }
            "},
            bad_example: indoc! {r"
                foreach ($items as $item) {
                    if ($item->ready) { work($item); }
                }
            "},
            category: Category::BestPractices,
        };
        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[
            NodeKind::For,
            NodeKind::Foreach,
            NodeKind::While,
            NodeKind::DoWhile,
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
        let (body, span) = match node {
            Node::For(statement) => (&statement.body, statement.span()),
            Node::Foreach(statement) => (&statement.body, statement.span()),
            Node::While(statement) => (&statement.body, statement.span()),
            Node::DoWhile(statement) => (&statement.body, statement.span()),
            _ => return,
        };
        let Some(statement) = extract_single_if(body.statements) else {
            return;
        };
        check_early_exit(
            ctx,
            self.meta,
            self.cfg.level(),
            statement,
            span,
            self.cfg.max_allowed_statements,
            &EarlyExitPattern {
                title: "Consider an early continue to reduce nesting.",
                primary: "this if statement wraps the loop body",
                context: "this loop can skip before its main path",
                help: "Invert the condition, continue early, then move the main path after the if statement.",
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::PreferEarlyContinueRule;
    use crate::settings::Settings;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = wrapped_loop_bodies_are_rejected,
        rule = PreferEarlyContinueRule,
        count = 4,
        code = "for ($i = 0; $i < 1; $i++) { if ($ready) { work(); } } foreach ($items as $item) { if ($ready) { work(); } } while ($ready) { if ($valid) { work(); } } do { if ($valid) { work(); } } while ($ready);",
    }

    test_lint_success! {
        name = else_and_existing_exit_are_allowed,
        rule = PreferEarlyContinueRule,
        code = "while ($ready) { if ($valid) { work(); } else { wait(); } } foreach ($items as $item) { if (!$item->ready) { continue; } }",
    }

    test_lint_success! {
        name = configured_small_body_is_allowed,
        rule = PreferEarlyContinueRule,
        settings = |settings: &mut Settings| settings.rules.prefer_early_continue.config.max_allowed_statements = 1,
        code = "while ($ready) { if ($valid) { work(); } }",
    }
}
