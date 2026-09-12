use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct LoopDoesNotIterateRule {
    meta: &'static RuleMeta,
    cfg: LoopDoesNotIterateConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct LoopDoesNotIterateConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for LoopDoesNotIterateConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for LoopDoesNotIterateConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for LoopDoesNotIterateRule {
    type Config = LoopDoesNotIterateConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Loop Does Not Iterate",
            code: "loop-does-not-iterate",
            description: indoc! {r"
                Finds loops with an unconditional break or return that prevents
                a second iteration.
            "},
            good_example: indoc! {r"
                for ($i = 0; $i < 3; $i++) {
                    write_line!($i);
                    if ($some_condition) {
                        break; // This break is conditional.
                    }
                }
            "},
            bad_example: indoc! {r"
                for ($i = 0; $i < 3; $i++) {
                    break; // The loop never truly iterates, as this break is unconditional.
                }
            "},
            category: Category::BestPractices,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[
            NodeKind::For,
            NodeKind::Foreach,
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
            Node::For(statement) => &statement.body,
            Node::Foreach(statement) => &statement.body,
            Node::While(statement) => &statement.body,
            Node::DoWhile(statement) => &statement.body,
            _ => return,
        };

        for statement in body.statements {
            let mut skips = SkipsTerminator(false);
            walk(Node::Statement(statement), &mut skips);
            if skips.0 {
                return;
            }

            let Statement::Expression(statement) = statement else {
                continue;
            };

            let terminates = match statement.expression {
                Expression::Return(_) => true,
                Expression::Break(exit) => exit.level.as_ref().is_none_or(|level| level.value == 1),
                _ => false,
            };

            if terminates {
                ctx.report(
                    self.meta,
                    self.cfg.level(),
                    node.span(),
                    "Loop is unconditionally terminated and will not iterate.",
                    [Level::HELP
                        .message("Check the unconditional exit; an if statement may be clearer.")
                        .into()],
                );
                return;
            }
        }
    }
}

struct SkipsTerminator(bool);

impl Visitor<'_, '_> for SkipsTerminator {
    fn enter(&mut self, node: Node<'_, '_>) -> Flow {
        if self.0
            || matches!(
                node,
                Node::Function(_)
                    | Node::Method(_)
                    | Node::Closure(_)
                    | Node::Class(_)
                    | Node::Interface(_)
                    | Node::Enum(_)
            )
        {
            return Flow::Skip;
        }

        if matches!(node, Node::Continue(_)) {
            self.0 = true;
        }

        Flow::Descend
    }
}
