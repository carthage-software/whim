use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::BinaryOperator;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::security::get_password;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoInsecureComparisonRule {
    meta: &'static RuleMeta,
    cfg: NoInsecureComparisonConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoInsecureComparisonConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoInsecureComparisonConfig {
    fn default() -> Self {
        Self {
            level: Level::ERROR,
        }
    }
}

impl Config for NoInsecureComparisonConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoInsecureComparisonRule {
    type Config = NoInsecureComparisonConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Insecure Comparison",
            code: "no-insecure-comparison",
            description: indoc! {r"
                Detects insecure comparison of passwords or tokens using `==` or `!=`.

                These operators are vulnerable to timing attacks, which can expose sensitive information.
                Instead, use `Whim\Hash\equals()` for comparing strings or `Whim\Password\verify()` for validating hashes.
            "},
            good_example: indoc! {r"
                if (Whim\Hash\equals($storedToken, $userToken)) {
                    // Valid token
                }
            "},
            bad_example: indoc! {r"
                if ($storedToken == $userToken) {
                    // Vulnerable to timing attacks
                }
            "},
            category: Category::Security,
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

        if !matches!(
            binary.operator,
            BinaryOperator::Equal(_) | BinaryOperator::NotEqual(_)
        ) {
            return;
        }

        let left = get_password(binary.lhs).is_some();
        let right = get_password(binary.rhs).is_some();
        if !left && !right
            || left && is_simple_literal(binary.rhs)
            || right && is_simple_literal(binary.lhs)
        {
            return;
        }

        ctx.report(self.meta, self.cfg.level(), binary.operator.span(), "Insecure comparison of sensitive data.",
            [Level::NOTE.message("Equality comparisons can reveal secret data through their timing.").into(),
                Level::HELP.message("Use Whim\\Hash\\equals() for secrets or Whim\\Password\\verify() for password hashes.").into()]);
    }
}

fn is_simple_literal(expression: &Expression<'_>) -> bool {
    match expression.unparenthesized() {
        Expression::Literal(Literal::String(literal)) => literal.raw.len() == 2,
        Expression::Literal(_) => true,
        _ => false,
    }
}
