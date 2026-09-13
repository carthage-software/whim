use std::path::Path;
use std::slice::from_ref;

use annotate_snippets::Level;
use annotate_snippets::Renderer;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::Linter;
use crate::rule::LintRule;
use crate::settings::Settings;

pub fn run_lint_test<R, F>(
    source: &str,
    expected: Option<usize>,
    settings_fn: Option<F>,
    diagnostic: Option<(&str, Level<'static>)>,
) where
    R: LintRule,
    F: FnOnce(&mut Settings),
{
    let arena = LocalArena::new();
    let program = parse(&arena, source)
        .unwrap_or_else(|error| panic!("failed to parse test source: {error:?}"));
    let mut settings = Settings::default();
    if let Some(settings_fn) = settings_fn {
        settings_fn(&mut settings);
    }

    let code = R::meta().code;
    let linter = Linter::new(&arena, &settings, Some(&[code.to_owned()]), false)
        .unwrap_or_else(|error| panic!("failed to build rule `{code}`: {error}"));
    assert_eq!(linter.rules().len(), 1, "rule `{code}` is not registered");

    let mut reported = Vec::new();
    let mut report = |_, code: &str, level: &Level<'static>, _: &str| {
        reported.push((code.to_owned(), level.clone()));
    };
    let diagnostics = linter.lint_with_diagnostics(
        "test.whim",
        Path::new("test.whim"),
        program,
        Some(&mut report),
    );
    assert_eq!(reported.len(), diagnostics.len());
    if let Some(expected) = expected {
        assert_eq!(
            diagnostics.len(),
            expected,
            "rule `{code}` produced the wrong number of diagnostics for:\n{source}"
        );
    } else {
        assert!(
            !diagnostics.is_empty(),
            "rule `{code}` did not report this source:\n{source}"
        );
    }

    for (group, (reported_code, level)) in diagnostics.iter().zip(reported) {
        let rendered = Renderer::plain().render(from_ref(group));
        assert_eq!(
            reported_code,
            diagnostic.as_ref().map_or(code, |(code, _)| code),
            "{rendered}"
        );
        if let Some((_, expected_level)) = &diagnostic {
            assert_eq!(level, *expected_level, "{rendered}");
        }
        assert!(rendered.contains(" = help:"), "{rendered}");
        Renderer::styled().render(from_ref(group));
    }
}
