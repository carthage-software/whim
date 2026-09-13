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
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoRedundantVariableRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantVariableConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantVariableConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantVariableConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoRedundantVariableConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantVariableRule {
    type Config = NoRedundantVariableConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant Variable",
            code: "no-redundant-variable",
            description: indoc! {"
                Finds values assigned to local variables but never read.
                Parameters, `$this`, and names that start with an underscore are ignored.
                The right-hand expression may still have side effects and should be kept when needed.
            "},
            good_example: indoc! {r#"
                function greet(string $name): string {
                    $greeting = "Hello, " . $name;
                    return $greeting;
                }
            "#},
            bad_example: indoc! {r#"
                function greet(string $name): string {
                    $unused = compute();
                    return "Hello";
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

        let usage = variable_usage::analyze(ctx.arena, parts);
        let mut redundant: Vec<_> = usage
            .redundant
            .iter()
            .filter(|(name, info)| !info.do_not_flag && !variable_usage::is_silenced_name(name))
            .filter_map(|(name, info)| info.pending_write.map(|span| (span, *name)))
            .collect();
        redundant.sort_unstable_by_key(|(span, _)| *span);

        for (span, name) in redundant {
            let bare = name.strip_prefix('$').unwrap_or(name);
            ctx.report(
                self.meta,
                self.cfg.level(),
                (span, "this value is never read"),
                format!("Variable `{name}` is assigned but never used."),
                [],
                [
                    Level::NOTE
                        .message("Keep the assigned expression if it has side effects.")
                        .into(),
                    Level::HELP
                        .message(format!(
                            "Remove the assignment, use its value, or rename it to `$_{bare}` when the value is intentionally discarded."
                        ))
                        .into(),
                ],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NoRedundantVariableRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = reports_an_unused_final_value,
        rule = NoRedundantVariableRule,
        code = "function f() { $value = compute(); return 1; }",
    }

    test_lint_failure! {
        name = reports_unused_foreach_and_catch_targets,
        rule = NoRedundantVariableRule,
        count = 2,
        code = "function f() { foreach ($items as $item) {} try {} catch (Error $error) {} }",
    }

    test_lint_success! {
        name = accepts_used_and_discarded_values,
        rule = NoRedundantVariableRule,
        code = "function f() { $value = compute(); debug!($value); $_discarded = compute(); }",
    }

    test_lint_success! {
        name = closure_capture_reads_outer_value,
        rule = NoRedundantVariableRule,
        code = "function f() { $value = compute(); return fn() => $value; }",
    }

    test_lint_success! {
        name = match_bindings_do_not_replace_outer_slots,
        rule = NoRedundantVariableRule,
        code = "function f() { $value = 1; $result = match ($input) { $value => $value }; return ($value, $result); }",
    }

    test_lint_failure! {
        name = match_binding_read_does_not_use_outer_value,
        rule = NoRedundantVariableRule,
        code = "function f() { $value = 1; match ($input) { $value => $value }; }",
    }

    test_lint_success! {
        name = loop_carried_value_is_read_on_next_iteration,
        rule = NoRedundantVariableRule,
        code = "function f() { $value = 0; while (ready()) { debug!($value); $value = next(); } }",
    }

    test_lint_success! {
        name = nested_loop_reads_loop_carried_value,
        rule = NoRedundantVariableRule,
        code = "function f() { $value = 0; while (outer()) { while (inner()) { debug!($value); } $value = next(); } }",
    }

    test_lint_success! {
        name = branch_writes_are_read_after_the_join,
        rule = NoRedundantVariableRule,
        code = "function f() { if ($condition) { $value = 1; } else { $value = 2; } return $value; }",
    }

    test_lint_success! {
        name = finally_reads_value_before_return_finishes,
        rule = NoRedundantVariableRule,
        code = "function f() { try { $value = compute(); return; } finally { debug!($value); } }",
    }
}
