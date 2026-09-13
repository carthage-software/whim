use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::access::Access;
use whim_syn::cst::call::Call;
use whim_syn::cst::construct::Construct;
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
pub struct YodaConditionsRule {
    meta: &'static RuleMeta,
    cfg: YodaConditionsConfig,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum YodaConditionsMode {
    #[default]
    Require,
    Deny,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct YodaConditionsConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    pub mode: YodaConditionsMode,
}

impl Default for YodaConditionsConfig {
    fn default() -> Self {
        Self {
            level: Level::HELP,
            mode: YodaConditionsMode::Require,
        }
    }
}

impl Config for YodaConditionsConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for YodaConditionsRule {
    type Config = YodaConditionsConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Yoda Conditions",
            code: "yoda-conditions",
            description: indoc! {"
                This rule controls the use of \"Yoda\" conditions for comparisons, where the constant, literal,
                or function call appears on the left side and the variable on the right.

                In `require` mode (default), Yoda style is enforced. Placing the constant on the left prevents
                the accidental-assignment bug (`=` instead of `==`), which causes a fatal error rather than a
                silent logical bug in a Yoda condition.

                In `deny` mode, Yoda style is forbidden. The variable must appear on the left for readability.
                When using `deny` mode, consider enabling the `no-assign-in-condition` rule to guard against
                accidental assignments (`=` instead of `==`) that Yoda conditions would otherwise catch.
            "},
            good_example: indoc! {r#"
                // configured mode: "require"
                if ( true == $is_active ) { /* ... */ }
                if ( 5 == $count ) { /* ... */ }
            "#},
            bad_example: indoc! {r#"
                // configured mode: "require"
                if ( $is_active == true ) { /* ... */ }
            "#},
            category: Category::BestPractices,
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
        let (message, left, right, help) = match self.cfg.mode {
            YodaConditionsMode::Require
                if is_writable_variable(binary.lhs) && is_constant_like(binary.rhs) =>
            {
                (
                    "Use Yoda condition style.",
                    "move this variable to the right",
                    "move this value to the left",
                    "Put the value on the left and the variable on the right of the comparison.",
                )
            }
            YodaConditionsMode::Deny
                if is_constant_like(binary.lhs) && is_writable_variable(binary.rhs) =>
            {
                (
                    "Avoid Yoda condition style.",
                    "move this value to the right",
                    "move this variable to the left",
                    "Put the variable on the left and the value on the right of the comparison.",
                )
            }
            _ => return,
        };
        ctx.report(
            self.meta,
            self.cfg.level(),
            (
                binary.operator.span(),
                "swap the operands of this comparison",
            ),
            message,
            [
                AnnotationKind::Context
                    .span(binary.lhs.span().into())
                    .label(left),
                AnnotationKind::Context
                    .span(binary.rhs.span().into())
                    .label(right),
            ],
            [Level::HELP.message(help).into()],
        );
    }
}

fn is_constant_like(expression: &Expression<'_>) -> bool {
    matches!(
        expression,
        Expression::Literal(_)
            | Expression::Access(Access::Constant(_) | Access::ClassConstant(_))
            | Expression::Vec(_)
            | Expression::Dict(_)
            | Expression::Tuple(_)
            | Expression::Call(Call::Function(_))
            | Expression::Construct(Construct::File(_) | Construct::Directory(_))
    )
}

fn is_writable_variable(expression: &Expression<'_>) -> bool {
    matches!(
        expression,
        Expression::Variable(_)
            | Expression::ArrayAccess(_)
            | Expression::Access(
                Access::Property(_) | Access::NullSafeProperty(_) | Access::StaticProperty(_)
            )
    )
}
