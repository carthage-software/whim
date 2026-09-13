use annotate_snippets::Level;
use hashbrown::HashMap;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::Program;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::statement::Statement;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::names;
use crate::rule::utils::names::Resolver;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoRedundantUseRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantUseConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantUseConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantUseConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for NoRedundantUseConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantUseRule {
    type Config = NoRedundantUseConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant Use",
            code: "no-redundant-use",
            description: indoc! {"
                Finds unused imports and imports that repeat the current namespace.
                Imports are matched with Whim's exact, case-sensitive alias rules.
                Namespace-prefix aliases count as used by qualified references.
            "},
            good_example: indoc! {"
                namespace App;
                use Vendor\\Mailer;
                function send(Mailer $mailer): void { $mailer->send(); }
            "},
            bad_example: indoc! {"
                namespace App;
                use Vendor\\Unused;
                function run(): void {}
            "},
            category: Category::Redundancy,
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
                self.check_region(ctx, namespace.name.value(), namespace.statements());
            }
        }
    }
}

impl NoRedundantUseRule {
    fn check_region<'ast, 'arena, A: Arena>(
        &self,
        ctx: &mut LintContext<'_, 'arena, A>,
        namespace: &str,
        statements: impl IntoIterator<Item = &'ast Statement<'arena>>,
    ) where
        'arena: 'ast,
    {
        let statements: Vec<_> = statements.into_iter().collect();
        let mut resolver = Resolver::for_namespace(namespace);
        let mut imports = Vec::new();
        let mut aliases = HashMap::new();
        let mut type_parameters = HashMap::new();

        for statement in statements {
            if let Statement::Use(declaration) = statement {
                names::for_each_use_item(declaration, |item, target, alias| {
                    let final_name = target.rsplit('\\').next().unwrap_or(&target);
                    let parent = target
                        .strip_suffix(final_name)
                        .and_then(|value| value.strip_suffix('\\'))
                        .unwrap_or("");
                    let preserves_name = item.alias.is_none() || alias == final_name;
                    let redundant = preserves_name && parent == namespace;
                    let index = imports.len();
                    imports.push(Import {
                        span: item.span(),
                        target,
                        redundant,
                        used: false,
                    });

                    aliases.insert(alias, index);
                });

                resolver.collect_use(declaration);
                continue;
            }

            let mut stack = vec![UseStep::Visit(Node::Statement(statement))];
            while let Some(step) = stack.pop() {
                match step {
                    UseStep::ExitTypeParameters(names) => {
                        for name in names {
                            let count = type_parameters
                                .get_mut(name)
                                .expect("entered type parameter is tracked");
                            *count -= 1;
                            let remove = *count == 0;
                            if remove {
                                type_parameters.remove(name);
                            }
                        }
                    }
                    UseStep::Visit(current) => {
                        if let Some(identifier) = names::symbol_identifier(current)
                            && !names::is_type_parameter_reference(
                                current,
                                &identifier,
                                &type_parameters,
                            )
                            && let Some(alias) = resolver.referenced_alias(&identifier)
                            && let Some(index) = aliases.get(alias)
                        {
                            imports[*index].used = true;
                        }

                        if let Node::SealedPermissions(permissions) = current {
                            for identifier in permissions.types {
                                if names::is_type_parameter(identifier, &type_parameters) {
                                    continue;
                                }
                                if let Some(alias) = resolver.referenced_alias(identifier)
                                    && let Some(index) = aliases.get(alias)
                                {
                                    imports[*index].used = true;
                                }
                            }
                        }

                        let mut entered = Vec::new();
                        if let Some(parameters) = names::type_parameters(current) {
                            for parameter in &parameters.parameters {
                                entered.push(parameter.name.value);
                                *type_parameters.entry(parameter.name.value).or_insert(0) += 1;
                            }
                        }

                        if !entered.is_empty() {
                            stack.push(UseStep::ExitTypeParameters(entered));
                        }
                        let mut children = Vec::new();
                        current.visit_children(&mut |child| children.push(child));
                        stack.extend(children.into_iter().rev().map(UseStep::Visit));
                    }
                }
            }
        }

        imports.sort_unstable_by_key(|import| import.span);
        for import in imports {
            let (message, label, note, help) = if import.redundant {
                (
                    format!("Import `{}` repeats the current namespace.", import.target),
                    "this import does not change name resolution",
                    "Names in the current namespace already resolve without this import.",
                    "Remove the redundant import.",
                )
            } else if !import.used {
                (
                    format!("Import `{}` is never used.", import.target),
                    "unused import",
                    "No static symbol reference uses this exact alias.",
                    "Remove the unused import or use its alias.",
                )
            } else {
                continue;
            };

            ctx.report(
                self.meta,
                self.cfg.level(),
                (import.span, label),
                message,
                [],
                [
                    Level::NOTE.message(note).into(),
                    Level::HELP.message(help).into(),
                ],
            );
        }
    }
}

struct Import {
    span: whim_span::Span,
    target: String,
    redundant: bool,
    used: bool,
}

enum UseStep<'ast, 'arena> {
    Visit(Node<'ast, 'arena>),
    ExitTypeParameters(Vec<&'arena str>),
}

fn global_statements<'ast, 'arena>(
    program: &'ast Program<'arena>,
) -> impl Iterator<Item = &'ast Statement<'arena>> {
    program
        .statements
        .iter()
        .filter(|statement| !matches!(statement, Statement::Namespace(_)))
}

#[cfg(test)]
mod tests {
    use super::NoRedundantUseRule;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = reports_unused_and_same_namespace_imports,
        rule = NoRedundantUseRule,
        count = 2,
        code = "namespace App; use Vendor\\Unused; use App\\Local; function run(): void {}",
    }

    test_lint_success! {
        name = types_attributes_patterns_calls_and_prefixes_use_imports,
        rule = NoRedundantUseRule,
        code = "namespace App; use Vendor\\Pkg; #[Pkg\\Attr] function run(Pkg\\Input $input): Pkg\\Output { return Pkg\\make(match ($input) { Pkg\\Thing #{} => $input }); }",
    }

    test_lint_success! {
        name = aliases_are_case_sensitive,
        rule = NoRedundantUseRule,
        code = "use Vendor\\Thing as Exact; function run(Exact $value): Exact { return $value; }",
    }

    test_lint_failure! {
        name = generic_bindings_do_not_use_import_aliases,
        rule = NoRedundantUseRule,
        count = 3,
        code = "use Vendor\\T; use Vendor\\U; use Vendor\\V; class Box<T> { public function map<U>(T $value, U $other): T { $copy = new T(); $f = fn<V>(V $inner): V => $inner; return $value; } }",
    }

    test_lint_success! {
        name = generic_bindings_do_not_shadow_function_imports,
        rule = NoRedundantUseRule,
        code = "use Vendor\\T; function f<T>(T $value): T { T(); return $value; }",
    }

    test_lint_success! {
        name = generic_bindings_do_not_shadow_attribute_or_constant_imports,
        rule = NoRedundantUseRule,
        code = "use Vendor\\A; use Vendor\\C; #[A] function f<A, C>(): C { return C; }",
    }

    test_lint_success! {
        name = generic_bindings_do_not_shadow_function_partial_imports,
        rule = NoRedundantUseRule,
        code = "use Vendor\\P; function f<P>() { return P(?); }",
    }
}
