use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::security::get_password_from_target;
use crate::rule::utils::security::is_password;
use crate::rule::utils::security::is_password_literal;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoLiteralPasswordRule {
    meta: &'static RuleMeta,
    cfg: NoLiteralPasswordConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoLiteralPasswordConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoLiteralPasswordConfig {
    fn default() -> Self {
        Self {
            level: Level::ERROR,
        }
    }
}

impl Config for NoLiteralPasswordConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoLiteralPasswordRule {
    type Config = NoLiteralPasswordConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Literal Password",
            code: "no-literal-password",
            description: indoc! {r"
                Detects the use of literal values for passwords or sensitive data.
                Storing passwords or sensitive information as literals in code is a security risk
                and should be avoided. Use environment variables or secure configuration management instead.
            "},
            good_example: indoc! {r"
                $password = Whim\Env\get('DB_PASSWORD');
            "},
            bad_example: indoc! {r#"
                $password = "supersecret";
            "#},
            category: Category::Security,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[
            NodeKind::Assignment,
            NodeKind::DictPair,
            NodeKind::Constant,
            NodeKind::ClassLikeConstant,
            NodeKind::Property,
            NodeKind::Parameter,
            NodeKind::NamedArgument,
            NodeKind::FinalLocal,
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
        match node {
            Node::Assignment(assignment) => {
                if let Some(password) = get_password_from_target(&assignment.target) {
                    self.check_value(ctx, password, assignment.value);
                }
            }
            Node::DictPair(pair) => {
                if let Expression::Literal(Literal::String(literal)) = pair.key
                    && is_password_literal(literal)
                {
                    self.check_value(ctx, pair.key.span(), pair.value);
                }
            }
            Node::Constant(constant) if is_password(constant.name.value.as_bytes()) => {
                self.check_value(ctx, constant.name.span(), constant.value)
            }
            Node::ClassLikeConstant(constant) if is_password(constant.name.value.as_bytes()) => {
                self.check_value(ctx, constant.name.span(), constant.value)
            }
            Node::Property(property) if is_password(property.variable.name.as_bytes()) => {
                if let Some(default) = &property.default {
                    self.check_value(ctx, property.variable.span(), default.value);
                }
            }
            Node::Parameter(parameter) if is_password(parameter.variable.name.as_bytes()) => {
                if let Some(default) = &parameter.default {
                    self.check_value(ctx, parameter.variable.span(), default.value);
                }
            }
            Node::NamedArgument(argument) if is_password(argument.name.value.as_bytes()) => {
                self.check_value(ctx, argument.name.span(), argument.value)
            }
            Node::FinalLocal(local) if is_password(local.variable.name.as_bytes()) => {
                self.check_value(ctx, local.variable.span(), local.value)
            }
            _ => {}
        }
    }
}

impl NoLiteralPasswordRule {
    fn check_value<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        name: Span,
        value: &Expression<'_>,
    ) {
        let literal = match value {
            Expression::Literal(Literal::String(literal)) => literal.raw.len() > 2,
            Expression::Literal(Literal::Integer(_)) => true,
            _ => false,
        };

        if !literal {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            name,
            "Literal passwords or sensitive data should not be stored in code.",
            [Level::HELP
                .message("Use environment variables or secure configuration instead.")
                .into()],
        );
    }
}
