use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoAssignInArgumentRule {
    meta: &'static RuleMeta,
    cfg: NoAssignInArgumentConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoAssignInArgumentConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoAssignInArgumentConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoAssignInArgumentConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoAssignInArgumentRule {
    type Config = NoAssignInArgumentConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Assign In Argument",
            code: "no-assign-in-argument",
            description: indoc! {"
                Detects assignments in function call arguments which can lead to unexpected behavior and make
                the code harder to read and understand.
            "},
            good_example: indoc! {r"
                $x = 5;
                foo($x);
            "},
            bad_example: indoc! {r"
                foo($x = 5);
            "},
            category: Category::Correctness,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::ArgumentList, NodeKind::PartialArgumentList]
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
            Node::ArgumentList(list) => {
                for argument in &list.arguments {
                    self.check_expression(ctx, argument.value());
                }
            }
            Node::PartialArgumentList(list) => {
                for argument in &list.arguments {
                    if let Some(value) = argument.value() {
                        self.check_expression(ctx, value);
                    }
                }
            }
            _ => {}
        }
    }
}

impl NoAssignInArgumentRule {
    fn check_expression<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        expression: &Expression<'_>,
    ) {
        if let Expression::Assignment(assignment) = expression.unparenthesized() {
            ctx.report(
                self.meta,
                self.cfg.level(),
                (assignment.operator.span(), "this operator assigns a value"),
                "Avoid assignments in function call arguments.",
                [AnnotationKind::Context
                    .span(expression.span().into())
                    .label("assignment used as a call argument")],
                [Level::HELP
                    .message("Assign first, then pass the result. Keep the order in which the arguments are evaluated.")
                    .into()],
            );
        }
    }
}
