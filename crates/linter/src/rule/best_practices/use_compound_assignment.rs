use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::expression::Expression;
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
pub struct UseCompoundAssignmentRule {
    meta: &'static RuleMeta,
    cfg: UseCompoundAssignmentConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct UseCompoundAssignmentConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for UseCompoundAssignmentConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for UseCompoundAssignmentConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for UseCompoundAssignmentRule {
    type Config = UseCompoundAssignmentConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Use Compound Assignment",
            code: "use-compound-assignment",
            description: indoc! {"
                Enforces the use of compound assignment operators (e.g., `+=`, `.=`)
                over their more verbose equivalents (`$var = $var + ...`).

                Using compound assignments is more concise and idiomatic. For string
                concatenation (`.=`), it can also be more performant as it avoids
                creating an intermediate copy of the string.
            "},
            good_example: indoc! {r"
                $count += 1;
                $message .= ' Hello';
            "},
            bad_example: indoc! {r"
                $count = $count + 1;
                $message = $message . ' Hello';
            "},
            category: Category::BestPractices,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Assignment]
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
        let Node::Assignment(assignment) = node else {
            return;
        };

        if !assignment.operator.is_assign() {
            return;
        }

        let Expression::Binary(binary) = assignment.value else {
            return;
        };

        let Some(operator) = get_compound_operator(&binary.operator) else {
            return;
        };

        if ctx.source_for(assignment.target.span()) != ctx.source_for(binary.lhs.span()) {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            assignment.span(),
            "Use a compound assignment for clarity and performance.",
            [Level::HELP
                .message(format!(
                    "Use {operator} instead of repeating the assignment target."
                ))
                .into()],
        );
    }
}

fn get_compound_operator(operator: &BinaryOperator) -> Option<&'static str> {
    Some(match operator {
        BinaryOperator::Addition(_) => "+=",
        BinaryOperator::Subtraction(_) => "-=",
        BinaryOperator::Multiplication(_) => "*=",
        BinaryOperator::Division(_) => "/=",
        BinaryOperator::Modulo(_) => "%=",
        BinaryOperator::Exponentiation(_) => "**=",
        BinaryOperator::StringConcat(_) => ".=",
        BinaryOperator::BitwiseAnd(_) => "&=",
        BinaryOperator::BitwiseOr(_) => "|=",
        BinaryOperator::BitwiseXor(_) => "^=",
        BinaryOperator::LeftShift(_) => "<<=",
        BinaryOperator::RightShift(_) => ">>=",
        BinaryOperator::NullCoalesce(_) => "??=",
        _ => return None,
    })
}
