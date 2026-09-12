use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
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
pub struct NoRedundantFinalRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantFinalConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantFinalConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantFinalConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for NoRedundantFinalConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantFinalRule {
    type Config = NoRedundantFinalConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant Final",
            code: "no-redundant-final",
            description: indoc! {"
                Detects redundant `final` modifiers on methods in final classes or enum methods.
            "},
            good_example: indoc! {r"
                final class Foo {
                    public function bar(): void {
                        // ...
                    }
                }
            "},
            bad_example: indoc! {r"
                final class Foo {
                    final public function bar(): void {
                        // ...
                    }
                }
            "},
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Class, NodeKind::Enum]
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
        let members = match node {
            Node::Class(class) if class.is_final() => class.members,
            Node::Enum(enumeration) => enumeration.members,
            _ => return,
        };

        for member in members {
            let ClassLikeMember::Method(method) = member else {
                continue;
            };

            let Some(modifier) = method.modifiers.iter().find(|modifier| modifier.is_final())
            else {
                continue;
            };

            ctx.report(self.meta, self.cfg.level(), modifier.span(),
                format!("The final modifier on method {} is redundant because this type cannot be extended.", method.name.value),
                [Level::HELP.message("Remove the final modifier from the method.").into()]);
        }
    }
}
