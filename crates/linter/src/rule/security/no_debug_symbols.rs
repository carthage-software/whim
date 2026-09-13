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
pub struct NoDebugSymbolsRule {
    meta: &'static RuleMeta,
    cfg: NoDebugSymbolsConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoDebugSymbolsConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoDebugSymbolsConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoDebugSymbolsConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoDebugSymbolsRule {
    type Config = NoDebugSymbolsConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Debug Symbols",
            code: "no-debug-symbols",
            description: "Flags `debug!` calls that should not remain in normal application paths.",
            good_example: "write_error_line!('request failed');",
            bad_example: "debug!($request);",
            category: Category::Security,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::DebugConstruct]
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
        let Node::DebugConstruct(debug) = node else {
            return;
        };

        ctx.report(
            self.meta,
            self.cfg.level(),
            (debug.span(), "remove this debug output"),
            "Do not leave `debug!` in application code.",
            [],
            [
                Level::NOTE
                    .message(
                        "Debug output can expose application data and write to standard error.",
                    )
                    .into(),
                Level::HELP
                    .message(
                        "Remove this call or replace it with the intended error reporting path.",
                    )
                    .into(),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::NoDebugSymbolsRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = debug_construct_is_rejected,
        rule = NoDebugSymbolsRule,
        code = "debug!($value);",
    }

    test_lint_success! {
        name = normal_error_output_is_allowed,
        rule = NoDebugSymbolsRule,
        code = "write_error_line!('failed');",
    }
}
