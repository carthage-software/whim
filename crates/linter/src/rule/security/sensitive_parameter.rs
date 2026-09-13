use annotate_snippets::Level;
use indoc::indoc;

use whim_syn::arena::Arena;
use whim_syn::cst::Program;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::function::Parameter;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::r#type::Type;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::names::Resolver;
use crate::rule::utils::security::is_password;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

const SENSITIVE_PARAMETER: &str = "Whim\\Marker\\SensitiveParameter";

#[derive(Debug, Clone)]
pub struct SensitiveParameterRule {
    meta: &'static RuleMeta,
    cfg: SensitiveParameterConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct SensitiveParameterConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for SensitiveParameterConfig {
    fn default() -> Self {
        Self {
            level: Level::ERROR,
        }
    }
}

impl Config for SensitiveParameterConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for SensitiveParameterRule {
    type Config = SensitiveParameterConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Sensitive Parameter",
            code: "sensitive-parameter",
            description: indoc! {"
                Finds parameters with secret-like names that lack `SensitiveParameter`.
                The marker redacts argument values from diagnostic stack traces.
                Boolean-only parameters are ignored because they do not carry secret text.
            "},
            good_example: indoc! {"
                use Whim\\Marker\\SensitiveParameter;
                function login(#[SensitiveParameter] string $password): void {}
            "},
            bad_example: "function login(string $password): void {}",
            category: Category::Security,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Program]
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
        let Node::Program(program) = node else {
            return;
        };

        self.check_region(ctx, "", global_statements(program));
        for statement in program.statements {
            if let Statement::Namespace(namespace) = statement {
                self.check_region(ctx, namespace.name.value(), namespace.statements().iter());
            }
        }
    }
}

impl SensitiveParameterRule {
    fn check_region<'ast, 'arena, A: Arena>(
        &self,
        ctx: &mut LintContext<'_, 'arena, A>,
        namespace: &str,
        statements: impl IntoIterator<Item = &'ast Statement<'arena>>,
    ) where
        'arena: 'ast,
    {
        let mut resolver = Resolver::for_namespace(namespace);
        for statement in statements {
            if let Statement::Use(declaration) = statement {
                resolver.collect_use(declaration);
                continue;
            }
            let mut stack = vec![Node::Statement(statement)];
            while let Some(node) = stack.pop() {
                if let Node::Parameter(parameter) = node {
                    self.check_parameter(ctx, parameter, &resolver);
                }
                node.visit_children(&mut |child| stack.push(child));
            }
        }
    }

    fn check_parameter<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        parameter: &Parameter<'_>,
        resolver: &Resolver,
    ) {
        if !is_password(parameter.variable.name.as_bytes())
            || parameter.r#type.is_some_and(boolean_only)
            || has_sensitive_marker(parameter, resolver)
        {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            (
                parameter.variable.span,
                "this value may appear in diagnostic traces",
            ),
            format!(
                "Sensitive parameter `{}` is not marked for redaction.",
                parameter.variable.name
            ),
            [],
            [
                Level::NOTE
                    .message("Whim redacts arguments marked with `Whim\\Marker\\SensitiveParameter`.")
                    .into(),
                Level::HELP
                    .message("Import `Whim\\Marker\\SensitiveParameter` and add `#[SensitiveParameter]` to this parameter.")
                    .into(),
            ],
        );
    }
}

fn has_sensitive_marker(parameter: &Parameter<'_>, resolver: &Resolver) -> bool {
    parameter.attribute_lists.iter().any(|list| {
        list.attributes
            .iter()
            .any(|attribute| resolver.resolve(&attribute.name) == SENSITIVE_PARAMETER)
    })
}

fn global_statements<'ast, 'arena>(
    program: &'ast Program<'arena>,
) -> impl Iterator<Item = &'ast Statement<'arena>> {
    program
        .statements
        .iter()
        .filter(|statement| !matches!(statement, Statement::Namespace(_)))
}

fn boolean_only(r#type: &Type<'_>) -> bool {
    match r#type.unparenthesized() {
        Type::Bool(_) => true,
        Type::Literal(Literal::True(_) | Literal::False(_) | Literal::Null(_)) => true,
        Type::Union(union) => boolean_only(union.left) && boolean_only(union.right),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::SensitiveParameterRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = secret_like_text_parameter_needs_marker,
        rule = SensitiveParameterRule,
        code = "function login(string $password): void {}",
    }

    test_lint_success! {
        name = imported_alias_resolves_to_marker,
        rule = SensitiveParameterRule,
        code = "use Whim\\Marker\\SensitiveParameter as Redact; function login(#[Redact] string $password): void {}",
    }

    test_lint_success! {
        name = fully_qualified_marker_is_accepted,
        rule = SensitiveParameterRule,
        code = "function login(#[\\Whim\\Marker\\SensitiveParameter] string $token): void {}",
    }

    test_lint_success! {
        name = boolean_only_secret_flag_is_not_sensitive,
        rule = SensitiveParameterRule,
        code = "function login(null|bool $hasPassword): void {}",
    }

    test_lint_failure! {
        name = unrelated_short_attribute_does_not_count,
        rule = SensitiveParameterRule,
        code = "namespace App; function login(#[SensitiveParameter] string $secret): void {}",
    }
}
