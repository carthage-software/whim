use std::path::Path;

use annotate_snippets::Level;
use annotate_snippets::Renderer;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::Linter;
use crate::rule::DisallowedSymbol;
use crate::rule::DisallowedSymbolsRule;
use crate::rule::NoDebugSymbolsRule;
use crate::rule::NoInsecureComparisonRule;
use crate::rule::NoLiteralPasswordRule;
use crate::rule::NoRedundantReadonlyRule;
use crate::rule::SensitiveParameterRule;
use crate::settings::Settings;
use crate::test_lint_failure;
use crate::test_lint_success;

test_lint_success! {
    name = allow_covers_a_function_body,
    rule = NoDebugSymbolsRule,
    code = r"use Whim\Lint\Allow; #[Allow('no-debug-symbols')] function f(): void { debug!('value'); }",
}

test_lint_failure! {
    name = allow_ends_at_the_declaration_boundary,
    rule = NoDebugSymbolsRule,
    count = 1,
    code = r"use Whim\Lint\Allow; #[Allow('no-debug-symbols')] function f(): void { debug!(1); } function g(): void { debug!(2); }",
}

test_lint_failure! {
    name = warn_overrides_a_rule_error_level,
    rule = NoLiteralPasswordRule,
    count = 1,
    diagnostic = ("no-literal-password", Level::WARNING),
    code = r"#[Whim\Lint\Warn(reason: 'test', rule: 'no-literal-password')] const PASSWORD = 'test';",
}

test_lint_failure! {
    name = deny_overrides_a_rule_warning_level,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("no-debug-symbols", Level::ERROR),
    code = r"#[Whim\Lint\Deny(rule: 'no-debug-symbols', reason: null)] function f(): void { debug!(1); }",
}

test_lint_success! {
    name = nested_allow_overrides_deny,
    rule = NoDebugSymbolsRule,
    code = r"use Whim\Lint\{Allow, Deny}; #[Deny('no-debug-symbols')] class C { #[Allow('no-debug-symbols')] public function f(): void { debug!(1); } }",
}

test_lint_failure! {
    name = nested_deny_overrides_allow_and_restores_it_for_siblings,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("no-debug-symbols", Level::ERROR),
    code = r"use Whim\Lint\{Allow, Deny}; #[Allow('no-debug-symbols')] class C { #[Deny('no-debug-symbols')] public function f(): void { debug!(1); } public function g(): void { debug!(2); } }",
}

test_lint_failure! {
    name = repeated_controls_follow_source_order,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("no-debug-symbols", Level::WARNING),
    code = r"use Whim\Lint\{Allow, Deny, Warn}; #[Allow('no-debug-symbols'), Deny('no-debug-symbols')] #[Warn('no-debug-symbols')] function f(): void { debug!(1); }",
}

test_lint_failure! {
    name = forbid_reports_errors,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("no-debug-symbols", Level::ERROR),
    code = r"#[Whim\Lint\Forbid('no-debug-symbols', reason: 'test')] function f(): void { debug!(1); }",
}

test_lint_failure! {
    name = forbid_rejects_nested_allow_even_without_a_finding,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"use Whim\Lint\{Allow, Forbid}; #[Forbid('no-debug-symbols')] class C { #[Allow('no-debug-symbols')] public function f(): void {} }",
}

test_lint_failure! {
    name = forbid_rejects_warn_in_the_same_scope,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"use Whim\Lint\{Warn, Forbid}; #[Forbid('no-debug-symbols'), Warn('no-debug-symbols')] function f(): void {}",
}

test_lint_failure! {
    name = deny_cannot_remove_an_enclosing_forbid,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"use Whim\Lint\{Allow, Deny, Forbid}; #[Forbid('no-debug-symbols')] class C { #[Deny('no-debug-symbols')] public function f(): void { $f = #[Allow('no-debug-symbols')] fn(): void {}; } }",
}

test_lint_success! {
    name = repeated_forbid_and_deny_keep_the_error_level,
    rule = NoDebugSymbolsRule,
    code = r"use Whim\Lint\{Deny, Forbid}; #[Forbid('no-debug-symbols'), Deny('no-debug-symbols'), Forbid('no-debug-symbols')] function f(): void {}",
}

test_lint_success! {
    name = aliases_named_arguments_and_string_expressions_resolve,
    rule = NoDebugSymbolsRule,
    code = r"namespace App; use Whim\Lint as L; use Whim\Lint\Allow as Quiet; #[Quiet(rule: ('no-' . ('debug' . '-symbols')))] function f(): void { debug!(1); } #[L\Allow('no-debug-symbols')] function g(): void { debug!(2); }",
}

const TOKEN_METHOD: &str = r"
use Whim\HTTP\Message\FieldMap;
use Whim\Lint\Allow;
use Whim\Marker\MustUse;

class HeaderChecker {
    #[MustUse]
    #[Allow(rule: 'no-insecure-comparison', reason: 'The token is not security-sensitive')]
    private function hasToken(
        FieldMap $headers,
        string $name,
        #[Allow(rule: 'sensitive-parameter', reason: 'This is not a security-sensitive parameter')]
        string $token,
    ): bool {
        return $headers->get($name) == $token;
    }
}
";

test_lint_success! {
    name = named_rule_and_reason_apply_to_methods,
    rule = NoInsecureComparisonRule,
    code = TOKEN_METHOD,
}

test_lint_success! {
    name = named_rule_and_reason_apply_to_parameters,
    rule = SensitiveParameterRule,
    code = TOKEN_METHOD,
}

test_lint_success! {
    name = reasons_accept_positional_mixed_and_reordered_named_arguments,
    rule = NoDebugSymbolsRule,
    code = r"use Whim\Lint\Allow; #[Allow('no-debug-symbols', 'test')] function a(): void { debug!(1); } #[Allow('no-debug-symbols', reason: null)] function b(): void { debug!(2); } #[Allow(reason: 'test', rule: 'no-' . 'debug-symbols')] function c(): void { debug!(3); }",
}

test_lint_success! {
    name = reasons_do_not_need_static_evaluation_by_the_linter,
    rule = NoDebugSymbolsRule,
    code = r"const WHY = 'test'; #[Whim\Lint\Allow(rule: 'no-debug-symbols', reason: WHY)] function f(): void { debug!(1); }",
}

test_lint_failure! {
    name = named_reason_does_not_replace_a_missing_rule,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"#[Whim\Lint\Allow(reason: 'no-debug-symbols')] function f(): void {}",
}

test_lint_failure! {
    name = named_reason_does_not_hide_an_unresolved_rule,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"#[Whim\Lint\Allow(reason: 'test', rule: UNKNOWN)] function f(): void {}",
}

test_lint_failure! {
    name = duplicate_rule_or_reason_arguments_are_errors,
    rule = NoDebugSymbolsRule,
    count = 4,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"use Whim\Lint\Allow; #[Allow('no-debug-symbols', rule: 'no-debug-symbols')] function a() {} #[Allow(rule: 'no-debug-symbols', rule: 'no-debug-symbols')] function b() {} #[Allow('no-debug-symbols', 'first', reason: 'second')] function c() {} #[Allow(rule: 'no-debug-symbols', reason: 'first', reason: 'second')] function d() {}",
}

test_lint_success! {
    name = decoded_string_literals_supply_the_rule_name,
    rule = NoDebugSymbolsRule,
    code = r##"#[\Whim\Lint\Allow("no-debug\x2dsymbols")] function f(): void { debug!(1); }"##,
}

test_lint_failure! {
    name = unrelated_attributes_and_namespace_aliases_do_not_leak,
    rule = NoDebugSymbolsRule,
    count = 2,
    code = r"namespace A { use Whim\Lint\Allow; #[Allow('no-debug-symbols')] function f(): void { debug!(1); } } namespace B { #[Allow('no-debug-symbols')] function f(): void { debug!(2); } } namespace C { use Other\Allow; #[Allow('no-debug-symbols')] function f(): void { debug!(3); } }",
}

test_lint_failure! {
    name = a_later_import_does_not_change_an_earlier_attribute,
    rule = NoDebugSymbolsRule,
    count = 1,
    code = r"#[Allow('no-debug-symbols')] function f(): void { debug!(1); } use Whim\Lint\Allow; #[Allow('no-debug-symbols')] function g(): void { debug!(2); }",
}

test_lint_failure! {
    name = parameter_controls_apply_to_diagnostics_from_program_rules,
    rule = SensitiveParameterRule,
    count = 1,
    code = r"use Whim\Lint\Allow; function f(#[Allow('sensitive-parameter')] string $token, string $password): void {}",
}

test_lint_success! {
    name = property_and_promoted_parameter_controls_apply_to_class_rules,
    rule = NoRedundantReadonlyRule,
    code = r"use Whim\Lint\Allow; readonly class C { #[Allow('no-redundant-readonly')] public readonly string $name; public function __construct(#[Allow('no-redundant-readonly')] public readonly int $id) {} }",
}

test_lint_success! {
    name = constants_parameters_properties_and_enum_cases_have_scopes,
    rule = NoLiteralPasswordRule,
    code = r"use Whim\Lint\Allow; #[Allow('no-literal-password')] const PASSWORD = 'x'; class C { #[Allow('no-literal-password')] public const string SECRET = 'x'; #[Allow('no-literal-password')] public string $token = 'x'; public function f(#[Allow('no-literal-password')] string $password = 'x'): void {} } enum E: string { #[Allow('no-literal-password')] case PASSWORD = 'x'; }",
}

test_lint_success! {
    name = interfaces_types_and_newtypes_have_scopes,
    rule = DisallowedSymbolsRule,
    settings = |settings: &mut Settings| {
        settings.rules.disallowed_symbols.config.symbols = vec![DisallowedSymbol::Simple("Bad".into())];
    },
    code = r"use Whim\Lint\Allow; #[Allow('disallowed-symbols')] interface C extends Bad {} #[Allow('disallowed-symbols')] type T = Bad; #[Allow('disallowed-symbols')] newtype N = Bad;",
}

test_lint_failure! {
    name = closure_controls_do_not_suppress_the_enclosing_body,
    rule = NoDebugSymbolsRule,
    count = 1,
    code = r"use Whim\Lint\Allow; function f(): void { $f = #[Allow('no-debug-symbols')] fn(): void { debug!(1); }; debug!(2); }",
}

test_lint_failure! {
    name = unknown_rule_names_are_errors,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"#[Whim\Lint\Allow('missing-rule')] function f(): void {}",
}

test_lint_failure! {
    name = constants_are_not_evaluated_as_rule_names,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"const RULE = 'no-debug-symbols'; #[Whim\Lint\Allow(RULE)] function f(): void {}",
}

test_lint_failure! {
    name = calls_are_not_evaluated_as_rule_names,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"#[Whim\Lint\Allow(rule_name())] function f(): void {}",
}

test_lint_failure! {
    name = non_string_arguments_are_errors,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"#[Whim\Lint\Allow(12)] function f(): void {}",
}

test_lint_failure! {
    name = invalid_utf8_rule_names_are_errors,
    rule = NoDebugSymbolsRule,
    count = 1,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r##"#[Whim\Lint\Allow("\xff")] function f(): void {}"##,
}

test_lint_failure! {
    name = invalid_constructor_arguments_are_errors,
    rule = NoDebugSymbolsRule,
    count = 4,
    diagnostic = ("lint-attribute", Level::ERROR),
    code = r"use Whim\Lint\Allow; #[Allow] function a() {} #[Allow()] function b() {} #[Allow('no-debug-symbols', 'test', 'extra')] function c() {} #[Allow(name: 'no-debug-symbols')] function d() {}",
}

test_lint_success! {
    name = known_rules_outside_the_selected_set_are_valid,
    rule = NoDebugSymbolsRule,
    code = r"#[Whim\Lint\Allow('sensitive-parameter')] function f(string $token): void {}",
}

#[test]
fn controls_enable_disabled_rules_only_in_their_scope() {
    let arena = LocalArena::new();
    let source = r"use Whim\Lint\Deny; #[Deny('no-debug-symbols')] function f(): void { debug!(1); } function g(): void { debug!(2); }";
    let program = parse(&arena, source).unwrap();
    let mut settings = Settings::default();
    settings.rules.no_debug_symbols.enabled = false;
    let linter = Linter::new(&arena, &settings, None, false).unwrap();
    let diagnostics = linter.lint("test.whim", program);
    assert_eq!(diagnostics.len(), 1);
    assert!(
        Renderer::plain()
            .render(&diagnostics)
            .contains("error[no-debug-symbols]")
    );
}

#[test]
fn attribute_errors_survive_rule_selection_and_exclusions() {
    let arena = LocalArena::new();
    let source = r"#[Whim\Lint\Allow('no-debug-symbols')] function f(): void { $f = #[Whim\Lint\Warn(UNKNOWN)] fn(): void {}; }";
    let program = parse(&arena, source).unwrap();
    let mut settings = Settings::default();
    settings.rules.no_debug_symbols.exclude.push("**".into());
    for only in [vec![], vec!["no-debug-symbols".into()]] {
        let linter = Linter::new(&arena, &settings, Some(&only), false).unwrap();
        let mut reported = Vec::new();
        let mut report = |span, code: &str, level: &Level<'static>, _: &str| {
            reported.push((span, code.to_owned(), level.clone()));
        };
        let diagnostics = linter.lint_with_diagnostics(
            "test.whim",
            Path::new("test.whim"),
            program,
            Some(&mut report),
        );
        assert_eq!(diagnostics.len(), 1);
        let [(span, code, level)] = reported.as_slice() else {
            panic!("{reported:?}")
        };
        assert_eq!(
            &source[span.start.offset as usize..span.end.offset as usize],
            "UNKNOWN"
        );
        assert_eq!(code, "lint-attribute");
        assert_eq!(*level, Level::ERROR);
    }
}
