use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use whim_syn::arena::Arena;
use whim_syn::cst::access::Access;
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
pub struct NoRedundantNullsafeRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantNullsafeConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantNullsafeConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantNullsafeConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for NoRedundantNullsafeConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantNullsafeRule {
    type Config = NoRedundantNullsafeConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant Nullsafe",
            code: "no-redundant-nullsafe",
            description: "Flags nullsafe property links made redundant by a coalescing fallback.",
            good_example: "$name = $user->name ?? 'guest';",
            bad_example: "$name = $user?->name ?? 'guest';",
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

        let BinaryOperator::NullCoalesce(coalesce) = binary.operator else {
            return;
        };

        let mut current = binary.lhs.unparenthesized();
        loop {
            current = match current {
                Expression::ArrayAccess(access) => access.array.unparenthesized(),
                Expression::Access(Access::Property(access)) => access.object.unparenthesized(),
                Expression::Access(Access::NullSafeProperty(access)) => {
                    ctx.report(
                        self.meta,
                        self.cfg.level(),
                        (
                            access.question_mark_arrow,
                            "this nullsafe property link is redundant",
                        ),
                        "The nullsafe operator is redundant before `??`.",
                        [AnnotationKind::Context
                            .span(coalesce.into())
                            .label("this coalescing operator already handles a missing property")],
                        [Level::HELP
                            .message("Use `->` for this property link.")
                            .into()],
                    );

                    access.object.unparenthesized()
                }
                _ => break,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NoRedundantNullsafeRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = coalescing_makes_property_nullsafe_redundant,
        rule = NoRedundantNullsafeRule,
        count = 2,
        code = "$a = $user?->profile?->name ?? 'guest';",
    }

    test_lint_success! {
        name = nullsafe_method_keeps_distinct_behavior,
        rule = NoRedundantNullsafeRule,
        code = "$a = $user?->profile() ?? 'guest'; $b = $user?->name;",
    }
}
