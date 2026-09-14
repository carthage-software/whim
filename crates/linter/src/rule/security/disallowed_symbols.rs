use annotate_snippets::Level;
use hashbrown::HashMap;
use indoc::indoc;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::Program;
use whim_syn::cst::access::ClassReference;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::statement::TopLevelStatement;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::names;
use crate::rule::utils::names::Resolver;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum DisallowedSymbol {
    Simple(String),
    Advanced {
        name: String,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        help: Option<String>,
        #[cfg_attr(
            feature = "serde",
            serde(
                default,
                skip_serializing_if = "Option::is_none",
                with = "crate::settings::optional_level"
            )
        )]
        level: Option<Level<'static>>,
    },
}

impl DisallowedSymbol {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Simple(name) | Self::Advanced { name, .. } => name,
        }
    }

    fn help(&self) -> Option<&str> {
        match self {
            Self::Simple(_) => None,
            Self::Advanced { help, .. } => help.as_deref(),
        }
    }

    fn level(&self) -> Option<Level<'static>> {
        match self {
            Self::Simple(_) => None,
            Self::Advanced { level, .. } => level.clone(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct DisallowedSymbolsConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    pub symbols: Vec<DisallowedSymbol>,
}

impl Default for DisallowedSymbolsConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
            symbols: Vec::new(),
        }
    }
}

impl Config for DisallowedSymbolsConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

#[derive(Debug, Clone)]
pub struct DisallowedSymbolsRule {
    meta: &'static RuleMeta,
    cfg: DisallowedSymbolsConfig,
    entries: HashMap<String, usize>,
}

impl LintRule for DisallowedSymbolsRule {
    type Config = DisallowedSymbolsConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Disallowed Symbols",
            code: "disallowed-symbols",
            description: indoc! {r"
                Forbids configured symbols in declarations, imports, attributes, types,
                patterns, construction, static references, constants, and direct calls.
                Names use Whim's exact, case-sensitive namespace and alias resolution.
                Dynamic callable and class values cannot be resolved and are skipped.
            "},
            good_example: "function allowed(): void { allowed(); }",
            bad_example: "Legacy\\run();",
            category: Category::Security,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Program]
    }

    fn build(settings: &RuleSettings<Self::Config>) -> Self {
        let cfg = settings.config.clone();
        let mut entries = HashMap::with_capacity(cfg.symbols.len());
        for (index, entry) in cfg.symbols.iter().enumerate() {
            entries
                .entry(names::strip_leading_separator(entry.name()).to_owned())
                .or_insert(index);
        }
        Self {
            meta: Self::meta(),
            cfg,
            entries,
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
        if self.entries.is_empty() {
            return;
        }

        self.check_region(ctx, "", global_statements(program));
        for statement in program.statements {
            if let TopLevelStatement::Namespace(namespace) = statement {
                self.check_region(ctx, namespace.name.value(), namespace.statements().iter());
            }
        }
    }
}

impl DisallowedSymbolsRule {
    fn check_region<'ast, 'arena, A: Arena>(
        &self,
        ctx: &mut LintContext<'_, 'arena, A>,
        namespace: &str,
        statements: impl IntoIterator<Item = &'ast TopLevelStatement<'arena>>,
    ) where
        'arena: 'ast,
    {
        let mut resolver = Resolver::for_namespace(namespace);
        let mut type_parameters = HashMap::new();
        let mut classes = Vec::new();
        for statement in statements {
            if let TopLevelStatement::Use(declaration) = statement {
                names::for_each_use_item(declaration, |item, target, _| {
                    self.report_if_disallowed(ctx, &target, item.name.span(), "imported here");
                });
                resolver.collect_use(declaration);
                continue;
            }

            let mut stack = vec![SymbolStep::Visit(Node::TopLevelStatement(statement))];
            while let Some(step) = stack.pop() {
                match step {
                    SymbolStep::ExitClass => {
                        classes.pop();
                    }
                    SymbolStep::ExitTypeParameters(names) => {
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
                    SymbolStep::Visit(current) => {
                        if let Some((name, span)) = names::declaration_name(current) {
                            self.report_if_disallowed(
                                ctx,
                                &resolver.qualify(name),
                                span,
                                "declared here",
                            );
                        }
                        if let Some((name, span)) = member_declaration(current, classes.last()) {
                            self.report_if_disallowed(ctx, &name, span, "declared here");
                        }
                        if let Some(identifier) = names::symbol_identifier(current)
                            && !names::is_type_parameter_reference(
                                current,
                                &identifier,
                                &type_parameters,
                            )
                        {
                            self.report_if_disallowed(
                                ctx,
                                &resolver.resolve(&identifier),
                                identifier.span(),
                                "referenced here",
                            );
                        }
                        if let Some((name, span)) =
                            member_reference(current, &resolver, &type_parameters, classes.last())
                        {
                            self.report_if_disallowed(ctx, &name, span, "referenced here");
                        }
                        if let Node::SealedPermissions(permissions) = current {
                            for identifier in permissions.types {
                                if names::is_type_parameter(identifier, &type_parameters) {
                                    continue;
                                }
                                self.report_if_disallowed(
                                    ctx,
                                    &resolver.resolve(identifier),
                                    identifier.span(),
                                    "referenced here",
                                );
                            }
                        }

                        let class = class_scope(current, &resolver);
                        if let Some(class) = class {
                            classes.push(class);
                            stack.push(SymbolStep::ExitClass);
                        }
                        let mut entered = Vec::new();
                        if let Some(parameters) = names::type_parameters(current) {
                            for parameter in &parameters.parameters {
                                entered.push(parameter.name.value);
                                *type_parameters.entry(parameter.name.value).or_insert(0) += 1;
                            }
                        }
                        if !entered.is_empty() {
                            stack.push(SymbolStep::ExitTypeParameters(entered));
                        }
                        let mut children = Vec::new();
                        current.visit_children(&mut |child| children.push(child));
                        stack.extend(children.into_iter().rev().map(SymbolStep::Visit));
                    }
                }
            }
        }
    }

    fn report_if_disallowed<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        resolved: &str,
        span: Span,
        label: &'static str,
    ) {
        let Some(index) = self.entries.get(resolved) else {
            return;
        };
        let entry = &self.cfg.symbols[*index];
        let help = entry
            .help()
            .unwrap_or("Use an allowed symbol or remove this name from the deny list.");
        ctx.report(
            self.meta,
            entry.level().unwrap_or_else(|| self.cfg.level()),
            (span, label),
            format!("Symbol `{resolved}` is disallowed."),
            [],
            [
                Level::NOTE
                    .message("The project configuration forbids this exact symbol name.")
                    .into(),
                Level::HELP.message(help.to_owned()).into(),
            ],
        );
    }
}

#[derive(Debug)]
struct ClassScope {
    name: String,
    parent: Option<String>,
}

enum SymbolStep<'ast, 'arena> {
    Visit(Node<'ast, 'arena>),
    ExitTypeParameters(Vec<&'arena str>),
    ExitClass,
}

fn class_scope(node: Node<'_, '_>, resolver: &Resolver) -> Option<ClassScope> {
    match node {
        Node::Class(declaration) => Some(ClassScope {
            name: resolver.qualify(declaration.name.value),
            parent: declaration
                .extends
                .as_ref()
                .and_then(|extends| extends.types.iter().next())
                .map(|parent| resolver.resolve(&parent.identifier)),
        }),
        Node::Interface(declaration) => Some(ClassScope {
            name: resolver.qualify(declaration.name.value),
            parent: None,
        }),
        Node::Enum(declaration) => Some(ClassScope {
            name: resolver.qualify(declaration.name.value),
            parent: None,
        }),
        _ => None,
    }
}

fn member_declaration(node: Node<'_, '_>, class: Option<&ClassScope>) -> Option<(String, Span)> {
    let class = class?;
    let (name, span) = match node {
        Node::Method(member) => (member.name.value, member.name.span),
        Node::ClassLikeConstant(member) => (member.name.value, member.name.span),
        Node::EnumCase(member) => (member.name.value, member.name.span),
        Node::Property(member) => (member.variable.name, member.variable.span),
        _ => return None,
    };
    Some((format!("{}::{name}", class.name), span))
}

fn member_reference(
    node: Node<'_, '_>,
    resolver: &Resolver,
    type_parameters: &HashMap<&str, usize>,
    class: Option<&ClassScope>,
) -> Option<(String, Span)> {
    let (owner, member, span) = match node {
        Node::StaticMethodCall(call) => (
            resolve_class_reference(&call.class, resolver, type_parameters, class)?,
            call.method.value,
            call.method.span,
        ),
        Node::StaticMethodPartialApplication(call) => (
            resolve_class_reference(&call.class, resolver, type_parameters, class)?,
            call.method.value,
            call.method.span,
        ),
        Node::ClassConstantAccess(access) => (
            resolve_class_reference(&access.class, resolver, type_parameters, class)?,
            access.constant.value,
            access.constant.span,
        ),
        Node::StaticPropertyAccess(access) => (
            resolve_class_reference(&access.class, resolver, type_parameters, class)?,
            access.property.name,
            access.property.span,
        ),
        Node::NamedType(named) if named.member.is_some() => {
            if names::is_type_parameter(&named.identifier, type_parameters) {
                return None;
            }
            let member = named.member.as_ref()?;
            (
                resolver.resolve(&named.identifier),
                member.name.value,
                member.name.span,
            )
        }
        Node::SelfType(self_type) if self_type.member.is_some() => {
            let member = self_type.member.as_ref()?;
            (class?.name.clone(), member.name.value, member.name.span)
        }
        _ => return None,
    };
    Some((format!("{owner}::{member}"), span))
}

fn resolve_class_reference(
    reference: &ClassReference<'_>,
    resolver: &Resolver,
    type_parameters: &HashMap<&str, usize>,
    class: Option<&ClassScope>,
) -> Option<String> {
    match reference {
        ClassReference::Named(named) => {
            if names::is_type_parameter(&named.identifier, type_parameters) {
                None
            } else {
                Some(resolver.resolve(&named.identifier))
            }
        }
        ClassReference::Self_(_) => Some(class?.name.clone()),
        ClassReference::Parent(_) => class?.parent.clone(),
        ClassReference::Static(_) | ClassReference::Expression(_) => None,
    }
}

fn global_statements<'ast, 'arena>(
    program: &'ast Program<'arena>,
) -> impl Iterator<Item = &'ast TopLevelStatement<'arena>> {
    program
        .statements
        .iter()
        .filter(|statement| !matches!(statement, TopLevelStatement::Namespace(_)))
}

#[cfg(test)]
mod tests {
    use annotate_snippets::Level;

    use super::DisallowedSymbol;
    use super::DisallowedSymbolsRule;
    use crate::settings::Settings;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_success! {
        name = empty_deny_list_does_nothing,
        rule = DisallowedSymbolsRule,
        code = "class Any {} function any(Any $value): Any { return new Any(); }",
    }

    test_lint_failure! {
        name = imported_alias_resolves_to_denied_symbol,
        rule = DisallowedSymbolsRule,
        count = 2,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Vendor\\Thing".to_owned()),
            ];
        },
        code = "use Vendor\\Thing as Alias; function f(Alias $value): void {}",
    }

    test_lint_failure! {
        name = declarations_use_fully_qualified_names,
        rule = DisallowedSymbolsRule,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Advanced {
                    name: "App\\Bad".to_owned(),
                    help: Some("Choose another public name.".to_owned()),
                    level: Some(Level::ERROR),
                },
            ];
        },
        code = "namespace App; class Bad {}",
    }

    test_lint_failure! {
        name = attributes_types_patterns_calls_and_construction_are_checked,
        rule = DisallowedSymbolsRule,
        count = 5,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Bad".to_owned()),
            ];
        },
        code = "#[\\Bad] function f(\\Bad $value): void { \\Bad(); new \\Bad(); match ($value) { \\Bad #{} => null }; }",
    }

    test_lint_success! {
        name = matching_is_case_sensitive,
        rule = DisallowedSymbolsRule,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Vendor\\Thing".to_owned()),
            ];
        },
        code = "function f(\\Vendor\\thing $value): void {}",
    }

    test_lint_success! {
        name = generic_bindings_are_local_symbols,
        rule = DisallowedSymbolsRule,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("App\\T".to_owned()),
                DisallowedSymbol::Simple("App\\U".to_owned()),
                DisallowedSymbol::Simple("App\\V".to_owned()),
            ];
        },
        code = "namespace App; class Box<T> { public function map<U>(T $value, U $other): T { $copy = new T(); $f = fn<V>(V $inner): V => $inner; return $value; } }",
    }

    test_lint_failure! {
        name = generic_bindings_do_not_shadow_disallowed_function_calls,
        rule = DisallowedSymbolsRule,
        count = 2,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Vendor\\T".to_owned()),
            ];
        },
        code = "use Vendor\\T; function f<T>(T $value): T { T(); return $value; }",
    }

    test_lint_failure! {
        name = generic_bindings_do_not_shadow_disallowed_attributes_or_constants,
        rule = DisallowedSymbolsRule,
        count = 4,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Vendor\\A".to_owned()),
                DisallowedSymbol::Simple("Vendor\\C".to_owned()),
            ];
        },
        code = "use Vendor\\A; use Vendor\\C; #[A] function f<A, C>(): C { return C; }",
    }

    test_lint_failure! {
        name = generic_bindings_do_not_shadow_disallowed_function_partials,
        rule = DisallowedSymbolsRule,
        count = 2,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Vendor\\P".to_owned()),
            ];
        },
        code = "use Vendor\\P; function f<P>() { return P(?); }",
    }

    test_lint_failure! {
        name = static_member_references_resolve_import_aliases,
        rule = DisallowedSymbolsRule,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Vendor\\Thing::make".to_owned()),
            ];
        },
        code = "use Vendor\\Thing as Alias; Alias::make();",
    }

    test_lint_failure! {
        name = member_declarations_and_self_references_are_checked,
        rule = DisallowedSymbolsRule,
        count = 5,
        settings = |settings: &mut Settings| {
            settings.rules.disallowed_symbols.config.symbols = vec![
                DisallowedSymbol::Simple("Thing::make".to_owned()),
                DisallowedSymbol::Simple("Thing::Case".to_owned()),
            ];
        },
        code = "class Thing { public const Case = 1; public static function make(): void { self::make(); debug!(self::Case); } } Thing::make();",
    }
}
