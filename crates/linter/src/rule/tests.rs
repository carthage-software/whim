use std::slice::from_ref;

use annotate_snippets::Renderer;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::Linter;
use crate::rule::LintRule;
use crate::settings::Settings;

pub fn run_lint_test<R, F>(source: &str, expected: Option<usize>, settings_fn: Option<F>)
where
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

    let diagnostics = linter.lint("test.whim", program);
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

    for diagnostic in &diagnostics {
        let rendered = Renderer::plain().render(from_ref(diagnostic));
        assert!(rendered.contains(code), "{rendered}");
        assert!(rendered.contains(" = help:"), "{rendered}");
        Renderer::styled().render(from_ref(diagnostic));
    }
}
