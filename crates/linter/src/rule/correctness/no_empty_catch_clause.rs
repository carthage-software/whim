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
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoEmptyCatchClauseRule {
    meta: &'static RuleMeta,
    cfg: NoEmptyCatchClauseConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoEmptyCatchClauseConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoEmptyCatchClauseConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoEmptyCatchClauseConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoEmptyCatchClauseRule {
    type Config = NoEmptyCatchClauseConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Empty Catch Clause",
            code: "no-empty-catch-clause",
            description: "Warns when a catch clause discards an error without handling it.",
            good_example: indoc! {r"
                try { work(); } catch (Error $error) { report($error); }
            "},
            bad_example: indoc! {r"
                try { work(); } catch (Error $error) {}
            "},
            category: Category::Correctness,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Try]
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
        let Node::Try(statement) = node else {
            return;
        };

        for clause in statement
            .catch_clauses
            .iter()
            .filter(|clause| clause.block.statements.is_empty())
        {
            ctx.report(
                self.meta,
                self.cfg.level(),
                (clause.span(), "this catch clause is empty"),
                "Do not use an empty catch clause.",
                [],
                [
                    Level::NOTE
                        .message("An empty catch clause hides the error that reached it.")
                        .into(),
                    Level::HELP
                        .message("Handle, report, or rethrow the error.")
                        .into(),
                ],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NoEmptyCatchClauseRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = empty_catch_is_rejected,
        rule = NoEmptyCatchClauseRule,
        code = "try { work(); } catch (Error $error) {}",
    }

    test_lint_success! {
        name = handled_catch_is_allowed,
        rule = NoEmptyCatchClauseRule,
        code = "try { work(); } catch (Error $error) { throw $error; }",
    }
}
