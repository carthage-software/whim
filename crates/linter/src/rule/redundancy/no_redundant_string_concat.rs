use annotate_snippets::Level;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::atom::LiteralString;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::Binary;
use whim_syn::cst::operation::BinaryOperator;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoRedundantStringConcatRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantStringConcatConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantStringConcatConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantStringConcatConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for NoRedundantStringConcatConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantStringConcatRule {
    type Config = NoRedundantStringConcatConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant String Concat",
            code: "no-redundant-string-concat",
            description: "Detects adjacent string literals joined with the concatenation operator.",
            good_example: "$message = 'Hello world';",
            bad_example: "$message = 'Hello' . ' world';",
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Binary]
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
        let Node::Binary(binary) = node else {
            return;
        };

        if !matches!(binary.operator, BinaryOperator::StringConcat(_)) || has_concat_parent(ctx) {
            return;
        }

        let mut operands = Vec::new();
        collect_operands(binary, &mut operands);
        let mut index = 0;
        while index < operands.len() {
            let Expression::Literal(Literal::String(first)) = operands[index].unparenthesized()
            else {
                index += 1;
                continue;
            };

            let mut last = first;
            let mut next = index + 1;
            while next < operands.len()
                && let Expression::Literal(Literal::String(candidate)) =
                    operands[next].unparenthesized()
                && pair_can_be_merged(ctx, last, candidate)
            {
                last = candidate;
                next += 1;
            }

            if next > index + 1 {
                ctx.report(
                    self.meta,
                    self.cfg.level(),
                    (
                        first.span().join(last.span()),
                        "these literals can form one string",
                    ),
                    "String concatenation can be simplified.",
                    [],
                    [Level::HELP
                        .message("Combine these literals into one string.")
                        .into()],
                );
            }

            index = next;
        }
    }
}

fn has_concat_parent<A: Arena>(ctx: &LintContext<'_, '_, A>) -> bool {
    let parent = match ctx.get_parent() {
        Some(Node::Expression(_)) => ctx.get_nth_parent(1),
        parent => parent,
    };

    matches!(parent, Some(Node::Binary(binary)) if matches!(binary.operator, BinaryOperator::StringConcat(_)))
}

fn collect_operands<'arena>(
    binary: &Binary<'arena>,
    operands: &mut Vec<&'arena Expression<'arena>>,
) {
    match binary.lhs.unparenthesized() {
        Expression::Binary(inner) if matches!(inner.operator, BinaryOperator::StringConcat(_)) => {
            collect_operands(inner, operands);
        }
        expression => operands.push(expression),
    }

    operands.push(binary.rhs);
}

fn pair_can_be_merged<A: Arena>(
    ctx: &LintContext<'_, '_, A>,
    left: &LiteralString<'_>,
    right: &LiteralString<'_>,
) -> bool {
    left.kind == right.kind
        && !ctx
            .source_for(left.span().join(right.span()))
            .contains(['\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::NoRedundantStringConcatRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = literal_runs_each_report_once,
        rule = NoRedundantStringConcatRule,
        count = 2,
        code = "$value = 'a' . 'b' . $middle . 'c' . 'd';",
    }

    test_lint_success! {
        name = mixed_quotes_and_dynamic_values_are_allowed,
        rule = NoRedundantStringConcatRule,
        code = "$a = 'a' . \"b\"; $b = 'a' . $value;",
    }
}
