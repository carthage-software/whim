use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::class::Method;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::function::ClosureBody;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::BinaryOperator;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct CyclomaticComplexityRule {
    meta: &'static RuleMeta,
    cfg: CyclomaticComplexityConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct CyclomaticComplexityConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    pub threshold: usize,
    pub method_threshold: Option<usize>,
}

impl Default for CyclomaticComplexityConfig {
    fn default() -> Self {
        Self {
            level: Level::ERROR,
            threshold: 15,
            method_threshold: None,
        }
    }
}

impl Config for CyclomaticComplexityConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for CyclomaticComplexityRule {
    type Config = CyclomaticComplexityConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Cyclomatic Complexity",
            code: "cyclomatic-complexity",
            description: indoc! {"
                Checks the number of paths through classes, interfaces, enums, functions, and closures.
                Branches, loops, match arms, guarded catches, short-circuit operators, coalescing, and spaceship comparisons add paths.
            "},
            good_example: indoc! {"
                function validate($value): bool {
                    if (!$value->valid()) { return false; }
                    return true;
                }
            "},
            bad_example: indoc! {"
                function validate($value): bool {
                    if ($value->a01()) {} if ($value->a02()) {}
                    if ($value->a03()) {} if ($value->a04()) {}
                    if ($value->a05()) {} if ($value->a06()) {}
                    if ($value->a07()) {} if ($value->a08()) {}
                    if ($value->a09()) {} if ($value->a10()) {}
                    if ($value->a11()) {} if ($value->a12()) {}
                    if ($value->a13()) {} if ($value->a14()) {}
                    if ($value->a15()) {} if ($value->a16()) {}
                }
            "},
            category: Category::Maintainability,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[
            NodeKind::Class,
            NodeKind::Interface,
            NodeKind::Enum,
            NodeKind::Function,
            NodeKind::Closure,
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
        match node {
            Node::Class(declaration) => {
                self.check_class_like(ctx, "Class", declaration.name.span, declaration.members)
            }
            Node::Interface(declaration) => {
                self.check_class_like(ctx, "Interface", declaration.name.span, declaration.members)
            }
            Node::Enum(declaration) => {
                self.check_class_like(ctx, "Enum", declaration.name.span, declaration.members)
            }
            Node::Function(function) => self.report_if_high(
                ctx,
                "Function",
                function.name.span,
                complexity(Node::Block(&function.body)),
                self.cfg.threshold,
            ),
            Node::Closure(closure) => {
                let body = match &closure.body {
                    ClosureBody::Block(block) => Node::Block(block),
                    ClosureBody::Expression { expression, .. } => Node::Expression(expression),
                };
                self.report_if_high(
                    ctx,
                    "Closure",
                    closure.r#fn.span(),
                    complexity(body),
                    self.cfg.threshold,
                );
            }
            _ => {}
        }
    }
}

impl CyclomaticComplexityRule {
    fn check_class_like<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        kind: &'static str,
        span: Span,
        members: &[ClassLikeMember<'_>],
    ) {
        let class_complexity = members
            .iter()
            .filter_map(|member| match member {
                ClassLikeMember::Method(method) => method_complexity(method),
                _ => None,
            })
            .map(|complexity| complexity - 1)
            .sum();
        self.report_if_high(ctx, kind, span, class_complexity, self.cfg.threshold);

        let Some(threshold) = self.cfg.method_threshold else {
            return;
        };
        for member in members {
            let ClassLikeMember::Method(method) = member else {
                continue;
            };
            let Some(value) = method_complexity(method) else {
                continue;
            };
            self.report_if_high(ctx, "Method", method.name.span, value, threshold);
        }
    }

    fn report_if_high<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        kind: &'static str,
        span: Span,
        complexity: usize,
        threshold: usize,
    ) {
        if complexity <= threshold {
            return;
        }
        ctx.report(
            self.meta,
            self.cfg.level(),
            (
                span,
                format!("complexity {complexity} exceeds the limit of {threshold}"),
            ),
            format!("{kind} has high cyclomatic complexity."),
            [],
            [
                Level::NOTE
                    .message("Each decision adds another path through the code.")
                    .into(),
                Level::HELP
                    .message("Split the logic into smaller functions with fewer decisions.")
                    .into(),
            ],
        );
    }
}

fn method_complexity(method: &Method<'_>) -> Option<usize> {
    match &method.body {
        MethodBody::Concrete(body) => Some(complexity(Node::Block(body)) + 1),
        MethodBody::Abstract(_) => Some(1),
    }
}

fn complexity(root: Node<'_, '_>) -> usize {
    let mut total = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if matches!(
            node,
            Node::Function(_)
                | Node::Method(_)
                | Node::Closure(_)
                | Node::Class(_)
                | Node::Interface(_)
                | Node::Enum(_)
        ) {
            continue;
        }
        total += match node {
            Node::If(_)
            | Node::For(_)
            | Node::Foreach(_)
            | Node::While(_)
            | Node::DoWhile(_)
            | Node::TryCatchClause(_)
            | Node::TryCatchGuard(_)
            | Node::MatchArm(_) => 1,
            Node::Binary(binary) => match binary.operator {
                BinaryOperator::And(_)
                | BinaryOperator::Or(_)
                | BinaryOperator::NullCoalesce(_) => 1,
                BinaryOperator::Spaceship(_) => 2,
                _ => 0,
            },
            _ => 0,
        };
        node.visit_children(&mut |child| stack.push(child));
    }
    total
}

#[cfg(test)]
mod tests {
    use super::CyclomaticComplexityRule;
    use crate::settings::Settings;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_success! {
        name = simple_function_stays_below_default,
        rule = CyclomaticComplexityRule,
        code = "function f(): bool { if ($a) { return false; } return true; }",
    }

    test_lint_failure! {
        name = configured_function_limit_is_enforced,
        rule = CyclomaticComplexityRule,
        settings = |settings: &mut Settings| {
            settings.rules.cyclomatic_complexity.config.threshold = 1;
        },
        code = "function f(): void { if ($a) {} while ($b) {} }",
    }

    test_lint_failure! {
        name = class_and_method_limits_are_independent,
        rule = CyclomaticComplexityRule,
        count = 2,
        settings = |settings: &mut Settings| {
            settings.rules.cyclomatic_complexity.config.threshold = 1;
            settings.rules.cyclomatic_complexity.config.method_threshold = Some(2);
        },
        code = "class C { public function f(): void { if ($a) {} if ($b) {} } }",
    }

    test_lint_failure! {
        name = match_arms_and_catch_guards_add_paths,
        rule = CyclomaticComplexityRule,
        count = 2,
        settings = |settings: &mut Settings| {
            settings.rules.cyclomatic_complexity.config.threshold = 1;
        },
        code = "function f($value) { return match ($value) { 1 => true, _ => false }; } function g() { try {} catch (Error $error) if (ready()) {} }",
    }

    test_lint_failure! {
        name = nested_closure_is_counted_only_in_its_own_scope,
        rule = CyclomaticComplexityRule,
        count = 1,
        settings = |settings: &mut Settings| {
            settings.rules.cyclomatic_complexity.config.threshold = 1;
        },
        code = "function f() { return fn() { if ($a) {} if ($b) {} }; }",
    }
}
