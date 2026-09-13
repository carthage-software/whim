use annotate_snippets::Level;
use indoc::indoc;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::atom::Modifier;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoRedundantReadonlyRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantReadonlyConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantReadonlyConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantReadonlyConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for NoRedundantReadonlyConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantReadonlyRule {
    type Config = NoRedundantReadonlyConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant Readonly",
            code: "no-redundant-readonly",
            description: "Detects readonly properties repeated inside a readonly class.",
            good_example: indoc! {r"
                readonly class User { public string $name; }
            "},
            bad_example: indoc! {r"
                readonly class User { public readonly string $name; }
            "},
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Class]
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
        let Node::Class(class) = node else {
            return;
        };

        let Some(class_readonly) = readonly_modifier(class.modifiers) else {
            return;
        };

        for member in class.members {
            match member {
                ClassLikeMember::Property(property) => {
                    if let Some(readonly) = readonly_modifier(property.modifiers) {
                        self.report(ctx, readonly, class_readonly);
                    }
                }
                ClassLikeMember::Method(method) if method.name.value == "__construct" => {
                    for parameter in &method.parameter_list.parameters {
                        if parameter.is_promoted_property()
                            && let Some(readonly) = readonly_modifier(parameter.modifiers)
                        {
                            self.report(ctx, readonly, class_readonly);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl NoRedundantReadonlyRule {
    fn report<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        readonly: &Modifier<'_>,
        class_readonly: &Modifier<'_>,
    ) {
        ctx.report(
            self.meta,
            self.cfg.level(),
            (readonly.span(), "this modifier is redundant"),
            "The property is already readonly because its class is readonly.",
            [annotate_snippets::AnnotationKind::Context
                .span(class_readonly.span().into())
                .label("the class sets readonly for each instance property")],
            [Level::HELP
                .message("Remove this `readonly` modifier.")
                .into()],
        );
    }
}

fn readonly_modifier<'a>(modifiers: &'a [Modifier<'a>]) -> Option<&'a Modifier<'a>> {
    modifiers.iter().find(|modifier| modifier.is_readonly())
}

#[cfg(test)]
mod tests {
    use super::NoRedundantReadonlyRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = readonly_class_repeats_property_modifiers,
        rule = NoRedundantReadonlyRule,
        count = 2,
        code = "readonly class C { public readonly string $a; public function __construct(public readonly int $b) {} }",
    }

    test_lint_success! {
        name = readonly_modifiers_outside_readonly_class_are_allowed,
        rule = NoRedundantReadonlyRule,
        code = "class C { public readonly string $a; public function __construct(public readonly int $b) {} }",
    }
}
