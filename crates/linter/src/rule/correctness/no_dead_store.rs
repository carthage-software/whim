use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use indoc::indoc;

use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::variable_usage;
use crate::rule::utils::variable_usage::DeadStoreRecorder;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoDeadStoreRule {
    meta: &'static RuleMeta,
    cfg: NoDeadStoreConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoDeadStoreConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoDeadStoreConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoDeadStoreConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoDeadStoreRule {
    type Config = NoDeadStoreConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Dead Store",
            code: "no-dead-store",
            description: indoc! {"
                Finds assignments overwritten before a read in the same function.
                Tracks branches separately and treats closure captures as reads.
                Skips parameters, scoped bindings, variables used by catch or finally,
                and names that start with an underscore.
            "},
            good_example: indoc! {r#"
                function f() {
                    $x = compute();
                    return $x;
                }
            "#},
            bad_example: indoc! {r#"
                function f() {
                    $x = 1; // dead - overwritten before being read
                    $x = compute();
                    return $x;
                }
            "#},
            category: Category::Correctness,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Function, NodeKind::Method, NodeKind::Closure]
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
        let Some(parts) = variable_usage::function_like_parts(node) else {
            return;
        };

        let usage: DeadStoreRecorder<'_> = variable_usage::analyze(ctx.arena, parts);
        let mut stores: Vec<_> = usage
            .info
            .iter()
            .filter(|(name, _)| !variable_usage::is_silenced_name(name))
            .flat_map(|(name, info)| {
                info.dead_stores
                    .iter()
                    .map(move |(span, overwrite)| (*span, *overwrite, *name))
            })
            .collect();

        stores.sort_unstable_by_key(|(span, _, _)| *span);
        for (span, overwrite, name) in stores {
            ctx.report(
                self.meta,
                self.cfg.level(),
                (span, "this value is never read"),
                format!("Variable `{name}` is overwritten before its value is read."),
                [AnnotationKind::Context.span(overwrite.into()).label("this assignment overwrites it")],
                [
                    Level::NOTE.message("Keep any side effects of the assigned expression, such as a function call.").into(),
                    Level::HELP.message("Remove the earlier assignment, or use its value before overwriting it.").into(),
                ],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NoDeadStoreRule;
    use crate::test_lint_success;

    test_lint_success! {
        name = loop_rescan_does_not_create_a_dead_store,
        rule = NoDeadStoreRule,
        code = "function f(bool $flag) { $x = 0; while ($flag) { $x = 1; } return $x; }",
    }
}
