use annotate_snippets::Level;
use indoc::indoc;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::function::ClosureBody;
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
pub struct PreferEarlyReturnRule {
    meta: &'static RuleMeta,
    cfg: PreferEarlyReturnConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct PreferEarlyReturnConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    pub max_allowed_statements: usize,
}

impl Default for PreferEarlyReturnConfig {
    fn default() -> Self {
        Self {
            level: Level::HELP,
            max_allowed_statements: 0,
        }
    }
}

impl Config for PreferEarlyReturnConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for PreferEarlyReturnRule {
    type Config = PreferEarlyReturnConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Prefer Early Return",
            code: "prefer-early-return",
            description: "Suggests an early return when one if statement wraps a function body.",
            good_example: indoc! {r"
                function process(bool $ready): void {
                    if (!$ready) { return; }
                    work();
                }
            "},
            bad_example: indoc! {r"
                function process(bool $ready): void {
                    if ($ready) { work(); }
                }
            "},
            category: Category::BestPractices,
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
        let (statements, span) = match node {
            Node::Function(function) => (function.body.statements, function.span()),
            Node::Method(method) => {
                let MethodBody::Concrete(body) = &method.body else {
                    return;
                };
                (body.statements, method.span())
            }
            Node::Closure(closure) => {
                let ClosureBody::Block(body) = &closure.body else {
                    return;
                };
                (body.statements, closure.span())
            }
            _ => return,
        };
        let Some(statement) = extract_single_if(statements) else {
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
                title: "Consider an early return to reduce nesting.",
                primary: "this if statement wraps the function body",
                context: "this function can return before its main path",
                help: "Invert the condition, return early, then move the main path after the if statement.",
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::PreferEarlyReturnRule;
    use crate::settings::Settings;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = single_wrapping_if_is_rejected,
        rule = PreferEarlyReturnRule,
        code = "function f(bool $ready): void { if ($ready) { work(); } }",
    }

    test_lint_success! {
        name = else_and_existing_early_exit_are_allowed,
        rule = PreferEarlyReturnRule,
        code = "function a(bool $ready): void { if ($ready) { work(); } else { wait(); } } function b(bool $ready): void { if ($ready) { return; } }",
    }

    test_lint_success! {
        name = configured_small_body_is_allowed,
        rule = PreferEarlyReturnRule,
        settings = |settings: &mut Settings| settings.rules.prefer_early_return.config.max_allowed_statements = 1,
        code = "function f(bool $ready): void { if ($ready) { work(); } }",
    }
}
