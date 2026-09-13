use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::access::ClassReference;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::r#type::Type;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoRedundantStaticRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantStaticConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantStaticConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantStaticConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for NoRedundantStaticConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantStaticRule {
    type Config = NoRedundantStaticConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Redundant Static",
            code: "no-redundant-static",
            description: indoc! {"
                Detects uses of `static` for late-static binding inside final classes.

                A final class cannot be extended, so `static` and `self` resolve to the same class.
                Using `self` states that intent directly and avoids unnecessary late-static binding.
            "},
            good_example: indoc! {r"
                final class User
                {
                    public static function create(): self
                    {
                        return new self();
                    }
                }
            "},
            bad_example: indoc! {r"
                final class User
                {
                    public static function create(): static
                    {
                        return new static();
                    }
                }
            "},
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::ClassReference, NodeKind::Method]
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
        let Some(class_final) = enclosing_final_modifier(ctx) else {
            return;
        };

        match node {
            Node::ClassReference(ClassReference::Static(keyword)) => {
                self.report(ctx, keyword.span(), class_final)
            }
            Node::Method(method) => {
                if let Some(return_type) = &method.return_type {
                    self.check_type(ctx, return_type.r#type, class_final);
                }
            }
            _ => {}
        }
    }
}

impl NoRedundantStaticRule {
    fn check_type<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        hint: &Type<'_>,
        class_final: Span,
    ) {
        match hint {
            Type::Static(keyword) => self.report(ctx, keyword.span(), class_final),
            Type::Parenthesized(hint) => self.check_type(ctx, hint.r#type, class_final),
            Type::Union(hint) => {
                self.check_type(ctx, hint.left, class_final);
                self.check_type(ctx, hint.right, class_final);
            }
            Type::Intersection(hint) => {
                self.check_type(ctx, hint.left, class_final);
                self.check_type(ctx, hint.right, class_final);
            }
            _ => {}
        }
    }

    fn report<A: Arena>(&self, ctx: &mut LintContext<'_, '_, A>, span: Span, class_final: Span) {
        ctx.report(
            self.meta,
            self.cfg.level(),
            (span, "`static` resolves to the same class as `self` here"),
            "Redundant `static` reference in a final class.",
            [AnnotationKind::Context
                .span(class_final.into())
                .label("this class cannot be extended")],
            [Level::HELP.message("Replace `static` with `self`.").into()],
        );
    }
}

fn enclosing_final_modifier<A: Arena>(ctx: &LintContext<'_, '_, A>) -> Option<Span> {
    let mut depth = 0;
    loop {
        match ctx.get_nth_parent(depth) {
            Some(Node::Class(class)) => {
                return class
                    .modifiers
                    .iter()
                    .find(|modifier| modifier.is_final())
                    .map(HasSpan::span);
            }
            Some(Node::Interface(_) | Node::Enum(_)) | None => return None,
            Some(_) => depth += 1,
        }
    }
}
