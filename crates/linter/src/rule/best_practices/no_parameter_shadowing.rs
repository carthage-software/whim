use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use hashbrown::HashMap;
use indoc::indoc;

use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::AssignmentTarget;
use whim_syn::cst::operation::DestructureTarget;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::variable_usage;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoParameterShadowingRule {
    meta: &'static RuleMeta,
    cfg: NoParameterShadowingConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoParameterShadowingConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoParameterShadowingConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoParameterShadowingConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoParameterShadowingRule {
    type Config = NoParameterShadowingConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Parameter Shadowing",
            code: "no-parameter-shadowing",
            description: indoc! {"
                Finds function parameters reused as foreach or catch targets.
                Reusing the local slot makes the parameter's earlier value unavailable.
                Match bindings are scoped to their arm and are not treated as shadowing.
            "},
            good_example: indoc! {"
                function read(vec<string> $items): void {
                    foreach ($items as $item) { debug!($item); }
                }
            "},
            bad_example: indoc! {"
                function read(vec<string> $items): void {
                    foreach (load() as $items) { debug!($items); }
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
        let Some(parts) = variable_usage::function_like_parts(node) else {
            return;
        };
        if parts.parameter_list.parameters.is_empty() {
            return;
        }

        let parameters: HashMap<_, _> = parts
            .parameter_list
            .parameters
            .iter()
            .map(|parameter| (parameter.variable.name, parameter.variable.span))
            .collect();
        let mut shadows: HashMap<&str, Vec<Span>> = HashMap::new();
        let mut stack = vec![parts.body];
        while let Some(current) = stack.pop() {
            match current {
                Node::Foreach(statement) => {
                    collect_target(statement.target.value(), &parameters, &mut shadows);
                    if let Some(key) = statement.target.key() {
                        collect_target(key, &parameters, &mut shadows);
                    }
                    current.visit_children(&mut |child| stack.push(child));
                }
                Node::TryCatchClause(clause) => {
                    if let Some(variable) = clause.variable
                        && parameters.contains_key(variable.name)
                    {
                        shadows
                            .entry(variable.name)
                            .or_default()
                            .push(variable.span);
                    }
                    current.visit_children(&mut |child| stack.push(child));
                }
                Node::Function(_)
                | Node::Method(_)
                | Node::Closure(_)
                | Node::Class(_)
                | Node::Interface(_)
                | Node::Enum(_) => {}
                _ => current.visit_children(&mut |child| stack.push(child)),
            }
        }

        let mut shadows: Vec<_> = shadows.into_iter().collect();
        shadows.sort_unstable_by_key(|(_, spans)| spans[0]);
        for (name, spans) in shadows {
            let parameter = parameters[name];
            let mut annotations = Vec::with_capacity(spans.len());
            for span in spans {
                annotations.push(
                    AnnotationKind::Context
                        .span(span.into())
                        .label("this target reuses the parameter's local slot"),
                );
            }
            ctx.report(
                self.meta,
                self.cfg.level(),
                (parameter, "parameter declared here"),
                format!("Parameter `{name}` is shadowed."),
                annotations,
                [
                    Level::NOTE
                        .message("Foreach and catch targets reuse local variable slots.")
                        .into(),
                    Level::HELP
                        .message(format!("Rename the target so it does not reuse `{name}`."))
                        .into(),
                ],
            );
        }
    }
}

fn collect_target<'arena>(
    target: &AssignmentTarget<'arena>,
    parameters: &HashMap<&'arena str, Span>,
    shadows: &mut HashMap<&'arena str, Vec<Span>>,
) {
    match target {
        AssignmentTarget::Variable(variable) if parameters.contains_key(variable.name) => {
            shadows
                .entry(variable.name)
                .or_default()
                .push(variable.span);
        }
        AssignmentTarget::Tuple(tuple) => {
            for target in &tuple.targets {
                let target = match target {
                    DestructureTarget::Target(target) => Some(target),
                    DestructureTarget::Default(default) => Some(&default.target),
                    DestructureTarget::Rest(rest) => rest.target.as_ref(),
                };
                if let Some(target) = target {
                    collect_target(target, parameters, shadows);
                }
            }
        }
        AssignmentTarget::Dict(dict) => {
            for entry in &dict.entries {
                collect_target(&entry.target, parameters, shadows);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::NoParameterShadowingRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = foreach_and_catch_reuse_parameter_slots,
        rule = NoParameterShadowingRule,
        count = 2,
        code = "function f($item, $error) { foreach ($items as $item) {} try {} catch (Error $error) {} }",
    }

    test_lint_success! {
        name = nested_callable_has_its_own_parameters,
        rule = NoParameterShadowingRule,
        code = "function f($item) { $g = fn($item) => $item; return $g; }",
    }

    test_lint_success! {
        name = match_binding_has_arm_scope,
        rule = NoParameterShadowingRule,
        code = "function f($item) { return match (load()) { $item => $item }; }",
    }
}
