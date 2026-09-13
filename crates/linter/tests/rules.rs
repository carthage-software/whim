use std::path::Path;
use std::slice::from_ref;

use annotate_snippets::Renderer;
use whim_linter::Linter;
use whim_linter::rule::best_practices::yoda_conditions::YodaConditionsMode;
use whim_linter::settings::Settings;
use whim_span::Span;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

fn spans(source: &str, code: &str, settings: &Settings) -> Vec<Span> {
    let arena = LocalArena::new();
    let program = parse(&arena, source).expect(source);
    let linter = Linter::new(&arena, settings, Some(&[code.to_owned()]), false).unwrap();
    let mut spans = Vec::new();
    let mut callback = |span, reported: &str, _: &_, _: &str| {
        assert_eq!(reported, code);
        spans.push(span);
    };
    let diagnostics = linter.lint_with_diagnostics(
        "test.whim",
        Path::new("test.whim"),
        program,
        Some(&mut callback),
    );
    assert_eq!(diagnostics.len(), spans.len());
    for diagnostic in &diagnostics {
        let rendered = Renderer::plain().render(from_ref(diagnostic));
        assert!(rendered.contains(" = help:"), "{rendered}");
        Renderer::styled().render(from_ref(diagnostic));
    }
    for span in &spans {
        assert!(
            source
                .get(span.start.offset as usize..span.end.offset as usize)
                .is_some()
        );
    }
    spans
}

#[test]
fn every_rule_reports_its_bad_example_and_accepts_its_good_example() {
    let settings = Settings::default();
    for (code, bad, good) in [
        (
            "tagged-todo",
            "// TODO: clean up\n",
            "// TODO(#123) clean up\n",
        ),
        (
            "tagged-fixme",
            "/** FIXME: broken */",
            "/** FIXME(@name) broken */",
        ),
        (
            "loop-does-not-iterate",
            "while ($ready) { break; }",
            "while ($ready) { if ($skip) { continue; } break; }",
        ),
        (
            "yoda-conditions",
            "if ($count == 5) {}",
            "if (5 == $count) {}",
        ),
        (
            "use-compound-assignment",
            "$count = $count + 1;",
            "$count += 1;",
        ),
        (
            "no-assign-in-argument",
            "call(($value = 1));",
            "call(fn() => $value = 1);",
        ),
        (
            "no-assign-in-condition",
            "if (($value = true)) {}",
            "$value = true; if ($value) {}",
        ),
        (
            "no-dead-store",
            "function f() { $x = 1; $x = 2; return $x; }",
            "function f() { $x = 1; debug!($x); $x = 2; return $x; }",
        ),
        (
            "excessive-nesting",
            "if ($a) { if ($a) { if ($a) { if ($a) { if ($a) { if ($a) { if ($a) { if ($a) {} } } } } } } }",
            "if ($a) { if ($a) {} }",
        ),
        (
            "no-redundant-static",
            "final class C { public static function make(): static { return new self(); } }",
            "class C { public static function make(): static { return new static(); } }",
        ),
        (
            "no-redundant-final",
            "final class C { final public function f() {} }",
            "class C { final public function f() {} }",
        ),
        (
            "no-redundant-else",
            "if ($a) { return 1; } else { return 2; }",
            "if ($a) { f(); } else { g(); }",
        ),
        (
            "no-literal-password",
            "$password = 'secret';",
            "$password = load_password();",
        ),
        (
            "no-insecure-comparison",
            "$password == $input;",
            "$password == '';",
        ),
        (
            "no-redundant-continue",
            "while ($ready) { run(); continue; }",
            "while ($ready) { if ($skip) { continue; } run(); }",
        ),
    ] {
        assert_eq!(spans(bad, code, &settings).len(), 1, "{code}: {bad}");
        assert!(spans(good, code, &settings).is_empty(), "{code}: {good}");
    }
}

#[test]
fn rules_keep_mago_options_and_all_start_enabled() {
    let arena = LocalArena::new();
    let mut settings = Settings::default();
    let linter = Linter::new(&arena, &settings, None, false).unwrap();
    assert_eq!(linter.rules().len(), 15);
    assert!(linter.rules().iter().all(|rule| rule.default_enabled()));
    settings.rules.yoda_conditions.config.mode = YodaConditionsMode::Deny;
    assert_eq!(
        spans("if (5 == $count) {}", "yoda-conditions", &settings).len(),
        1
    );
    settings
        .rules
        .no_assign_in_condition
        .config
        .ignore_while_statements = true;
    assert!(
        spans(
            "while ($x = next()) {}",
            "no-assign-in-condition",
            &settings
        )
        .is_empty()
    );
    settings.rules.excessive_nesting.config.threshold = 1;
    assert_eq!(
        spans("if ($a) { if ($b) {} }", "excessive-nesting", &settings).len(),
        1
    );
}

#[test]
fn dead_stores_respect_branches_loops_captures_and_terminators() {
    let settings = Settings::default();
    for (body, expected) in [
        ("$x = 1; $x = 2; $x = 3; return $x;", 2),
        ("if ($a) { $x = 1; } else { $x = 2; } return $x;", 0),
        ("if ($a) { $x = 1; $x = 2; return $x; }", 1),
        ("$x = 1; $f = fn() => $x; $x = 2; return $f;", 0),
        ("$x = 1; $x += 2; return $x;", 0),
        (
            "$x = 1; foreach ($items as $x) { debug!($x); } debug!($x);",
            0,
        ),
        ("$x = 1; while ($ready) { $x = 2; } return $x;", 0),
        ("$x = 1; while ($x != 0) { $x = next(); }", 0),
        ("$x = 1; return; $x = 2;", 0),
        ("$x = 1; $obj?->f($x = 2); return $x;", 0),
        ("$x = 'a'; assert!($ok, $x = 'b'); return $x;", 1),
        ("$x = 1; $y ??= ($x = 2); return $x;", 0),
        ("$x = 1; $y &&= ($x = 2); return $x;", 0),
        ("$x = 1; $items[$x] = ($x = 2); return $items;", 0),
        ("$x = 1; $items[$x = 2] = read($x); return $items;", 1),
        (
            "$x = 1; foreach ($items as ($y = ($x = 2), $z)) {} return $x;",
            0,
        ),
        ("$_ignored = 1; $_ignored = 2;", 0),
    ] {
        assert_eq!(
            spans(
                &format!("function f() {{ {body} }}"),
                "no-dead-store",
                &settings
            )
            .len(),
            expected,
            "{body}"
        );
    }
}

#[test]
fn security_rules_cover_declarations_keys_members_and_arguments() {
    let source = "const PASSWORD = 'x'; class C { public string $apiKey = 'x'; public function f(string $secret = 'x') {} } $data = dict['token' => 'x']; f(password: 'x'); $c->password = 'x';";
    assert_eq!(
        spans(source, "no-literal-password", &Settings::default()).len(),
        6
    );
    assert_eq!(
        spans(
            "$request->token == $given; $data['api_key'] != $input;",
            "no-insecure-comparison",
            &Settings::default()
        )
        .len(),
        2
    );
}

#[test]
fn diagnostics_render_directly_with_annotate_snippets_and_obey_exclusions() {
    let arena = LocalArena::new();
    let source = "// TODO: work\n";
    let program = parse(&arena, source).unwrap();
    let mut settings = Settings::default();
    settings
        .rules
        .tagged_todo
        .exclude
        .push("generated/**".to_owned());
    let linter = Linter::new(&arena, &settings, None, false).unwrap();
    assert!(linter.lint("generated/file.whim", program).is_empty());
    let diagnostics = linter.lint("src/file.whim", program);
    let rendered = Renderer::plain().render(&diagnostics);
    assert!(rendered.contains("warning[tagged-todo]"), "{rendered}");
    assert!(rendered.contains("src/file.whim:1:4"), "{rendered}");
    assert!(
        rendered.contains("^^^^ missing an owner or issue reference"),
        "{rendered}"
    );
    assert!(
        rendered.contains("help: Add a tag such as `TODO(@name)`"),
        "{rendered}"
    );
}

#[test]
fn comment_highlights_follow_the_marker_in_multiline_comments() {
    for (source, code, marker) in [
        (
            "'🙂'; /** a comment\r\n * TODO: work\r\n */",
            "tagged-todo",
            "TODO",
        ),
        (
            "/*\n * café\n *   FIXME: work\n */",
            "tagged-fixme",
            "FIXME",
        ),
    ] {
        let reported = spans(source, code, &Settings::default());
        assert_eq!(reported.len(), 1);
        let start = source.find(marker).unwrap();
        assert_eq!(reported[0].start.offset as usize, start);
        assert_eq!(reported[0].end.offset as usize, start + marker.len());
    }
}

#[test]
fn dead_store_reports_point_to_the_assignment_that_overwrites_each_value() {
    let arena = LocalArena::new();
    let source = "function f() {\n    $x = 1;\n    $x = 2;\n    $x = 3;\n    return $x;\n}";
    let program = parse(&arena, source).unwrap();
    let linter = Linter::new(
        &arena,
        &Settings::default(),
        Some(&["no-dead-store".to_owned()]),
        false,
    )
    .unwrap();
    let reports = linter.lint("test.whim", program);
    assert_eq!(reports.len(), 2);
    for (report, (earlier, later)) in reports.iter().zip([(2, 3), (3, 4)]) {
        let rendered = Renderer::plain().render(from_ref(report));
        assert!(
            rendered.contains(&format!("test.whim:{earlier}:5")),
            "{rendered}"
        );
        assert!(
            rendered.contains(&format!("{later} |     $x =")),
            "{rendered}"
        );
        assert!(
            rendered.contains("^^ this value is never read"),
            "{rendered}"
        );
        assert!(
            rendered.contains("-- this assignment overwrites it"),
            "{rendered}"
        );
    }
}

#[test]
fn metadata_examples_are_valid_whim_and_match_the_rule() {
    let arena = LocalArena::new();
    let settings = Settings::default();
    let linter = Linter::new(&arena, &settings, None, false).unwrap();
    for rule in linter.rules() {
        let meta = rule.meta();
        assert!(
            !spans(meta.bad_example, meta.code, &settings).is_empty(),
            "{}",
            meta.code
        );
        assert!(
            spans(meta.good_example, meta.code, &settings).is_empty(),
            "{}",
            meta.code
        );
    }
}

#[test]
fn dead_stores_handle_destructuring_and_reads_during_unwinding() {
    for (body, expected) in [
        ("$x = 1; ($x, $y) = values(); return $x;", 1),
        ("$x = 1; dict['x' => $x] = values(); return $x;", 1),
        ("$x = 1; ($y = $x, $z) = values(); return $y;", 0),
        (
            "try { $x = 1; may_throw(); $x = 2; } catch (Error $e) { debug!($x); }",
            0,
        ),
        (
            "try { $x = 1; may_throw(); $x = 2; } finally { debug!($x); }",
            0,
        ),
        (
            "$x = 1; match ($obj) { #{ $x } => $x, _ => null }; debug!($x);",
            0,
        ),
        ("$x = 1; using ($x = open()) { debug!($x); } debug!($x);", 0),
    ] {
        assert_eq!(
            spans(
                &format!("function f() {{ {body} }}"),
                "no-dead-store",
                &Settings::default()
            )
            .len(),
            expected,
            "{body}"
        );
    }
}
