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
pub struct NoAssignInConditionRule {
    meta: &'static RuleMeta,
    cfg: NoAssignInConditionConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoAssignInConditionConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    pub ignore_while_statements: bool,
}

impl Default for NoAssignInConditionConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
            ignore_while_statements: false,
        }
    }
}

impl Config for NoAssignInConditionConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoAssignInConditionRule {
    type Config = NoAssignInConditionConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Assign In Condition",
            code: "no-assign-in-condition",
            description: indoc! {"
                Detects assignments in conditions which can lead to unexpected behavior and make the code harder
                to read and understand.
            "},
            good_example: indoc! {r"
                $x = 1;
                if ($x == 1) {
                    // ...
                }
            "},
            bad_example: indoc! {r"
                if ($x = 1) {
                    // ...
                }
            "},
            category: Category::Correctness,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::If, NodeKind::While, NodeKind::DoWhile]
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
        let condition = match node {
            Node::If(statement) => statement.condition,
            Node::While(statement) if !self.cfg.ignore_while_statements => statement.condition,
            Node::DoWhile(statement) if !self.cfg.ignore_while_statements => statement.condition,
            _ => return,
        };

        let Expression::Assignment(assignment) = condition.unparenthesized() else {
            return;
        };

        ctx.report(
            self.meta,
            self.cfg.level(),
            (assignment.operator.span(), "this operator assigns a value"),
            "Avoid assignments in conditions.",
            [AnnotationKind::Context
                .span(condition.span().into())
                .label("the condition tests the assigned value")],
            [Level::HELP
                .message(if matches!(node, Node::If(_)) {
                    "Assign before the `if`, then test the result."
                } else {
                    "Separate the assignment from the test, keeping it on each iteration."
                })
                .into()]
            .into_iter()
            .chain(assignment.operator.is_assign().then(|| {
                Level::HELP
                    .message("Use `==` instead of `=` if you meant to compare values.")
                    .into()
            })),
        );
    }
}
