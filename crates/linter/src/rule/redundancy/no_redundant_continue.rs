use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::statement::Statement;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoRedundantContinueRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantContinueConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantContinueConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantContinueConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for NoRedundantContinueConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantContinueRule {
    type Config = NoRedundantContinueConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant Continue",
            code: "no-redundant-continue",
            description: indoc! {"
                Detects redundant `continue` statements in loops.
            "},
            good_example: indoc! {r#"
                while (true) {
                    write_line!("Hello, world!");
                }
            "#},
            bad_example: indoc! {r#"
                while (true) {
                    write_line!("Hello, world!");
                    continue; // Redundant `continue` statement
                }
            "#},
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[
            NodeKind::Foreach,
            NodeKind::For,
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
        let body = match node {
            Node::Foreach(statement) => &statement.body,
            Node::For(statement) => &statement.body,
            Node::While(statement) => &statement.body,
            Node::DoWhile(statement) => &statement.body,
            _ => return,
        };

        let Some(Statement::Expression(statement)) = body.statements.last() else {
            return;
        };

        let Expression::Continue(continuation) = statement.expression else {
            return;
        };

        if continuation
            .level
            .as_ref()
            .is_some_and(|level| level.value != 1)
        {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            (
                statement.span(),
                "nothing follows this `continue` in the loop body",
            ),
            "Redundant `continue` at the end of a loop.",
            [AnnotationKind::Context
                .span(body.right_brace.into())
                .label("reaching this point also advances the loop")],
            [Level::HELP
                .message("Remove this `continue`; the loop proceeds the same way without it.")
                .into()],
        );
    }
}
