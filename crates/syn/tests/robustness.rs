use std::thread::Builder;

use whim_span::HasSpan;
use whim_syn::arena::LocalArena;
use whim_syn::cst::node::Node;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::walker::deepest_path;
use whim_syn::error::ParseError;
use whim_syn::error::SyntaxError;
use whim_syn::parser::MAX_STRUCTURAL_DEPTH;
use whim_syn::parser::parse;

fn errors(source: &str) -> Vec<ParseError> {
    let arena = LocalArena::new();
    vec![parse(&arena, source).expect_err("expected parse error")]
}

#[test]
fn parsing_stops_at_the_first_error() {
    let source = "$a = ;\n\
                  $b = 1 + ;\n\
                  $c = )( ;\n\
                  $d = ;\n";
    let reported = errors(source);
    assert_eq!(reported.len(), 1, "parser must stop at the first error");

    for error in &reported {
        let span = HasSpan::span(error);
        assert!(
            span.start.offset as usize <= source.len(),
            "error span escapes the source: {error:?}"
        );
    }
}

#[test]
fn a_well_formed_statement_after_a_broken_one_is_not_parsed() {
    let reported = errors("$a = ;\n$b = 2;\n");
    assert_eq!(
        reported.len(),
        1,
        "parser must stop after the broken statement"
    );
}

#[test]
fn diagnostics_render_a_source_snippet_and_caret_not_debug() {
    let source = "$a = 1 +\n";
    let rendered = parse(&LocalArena::new(), source)
        .expect_err("expected a parse error")
        .render(source, "example.whim");

    assert!(rendered.contains("error:"), "no error header: {rendered}");
    assert!(rendered.contains("example.whim"), "no origin: {rendered}");
    assert!(rendered.contains("$a = 1 +"), "no source line: {rendered}");
    assert!(rendered.contains('^'), "no caret: {rendered}");
    assert!(
        !rendered.contains("UnexpectedToken") && !rendered.contains("UnexpectedEndOfFile"),
        "diagnostic leaks the Debug struct: {rendered}"
    );
}

#[test]
fn malformed_base_prefixed_literals_are_rejected_at_the_lexer() {
    for malformed in ["0x", "0X", "0b", "0o", "0xG", "0b2", "0o8", "$a = 0x;"] {
        let reported = errors(malformed);
        assert!(
            reported.iter().any(|error| matches!(
                error,
                ParseError::SyntaxError(SyntaxError::MalformedNumericLiteral(_))
            )),
            "`{malformed}` was not rejected as a malformed literal: {reported:?}"
        );
    }

    for good in ["$a = 0xff;", "$a = 0b101;", "$a = 0o17;"] {
        let arena = LocalArena::new();
        assert!(parse(&arena, good).is_ok(), "`{good}` should parse");
    }
}

#[test]
fn a_long_run_of_modifiers_does_not_overflow_the_lookahead_ring() {
    let source = "public private protected static final abstract readonly public private static \
         final abstract readonly public private class Wide {}";
    let arena = LocalArena::new();
    let _ = parse(&arena, source);
}

#[test]
fn the_parser_records_the_one_source_it_lexed() {
    let source = "$greeting = 'hello';\n";
    let arena = LocalArena::new();
    let program = parse(&arena, source).expect("valid program");
    assert_eq!(program.source_text, source);
}

#[test]
fn deeply_nested_types_hit_the_recursion_guard_not_the_stack() {
    let rendered = Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let mut source = String::from("function deep(): ");
            for _ in 0..4000 {
                source.push_str("vec<");
            }
            source.push_str("int");
            for _ in 0..4000 {
                source.push('>');
            }
            source.push_str(" { return $x; }");

            let arena = LocalArena::new();
            matches!(
                parse(&arena, &source).expect_err("deep types must be rejected"),
                ParseError::RecursionLimitExceeded(_)
            )
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    assert!(rendered, "expected the recursion guard to fire");
}

fn chain_depth(head: &str, link: &str, links: usize) -> usize {
    let mut source = String::from(head);
    for _ in 0..links {
        source.push_str(link);
    }
    source.push(';');

    let arena = LocalArena::new();
    let program = parse(&arena, &source).expect("a measuring chain parses");

    deepest_path(Node::Program(program)).levels
}

fn links_within_limit(head: &str, link: &str) -> usize {
    let one = chain_depth(head, link, 1);
    let step = chain_depth(head, link, 2) - one;

    1 + (MAX_STRUCTURAL_DEPTH - one) / step
}

#[test]
fn a_chain_far_past_the_structural_limit_is_refused_on_a_small_stack() {
    let (reported, length) = Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let source = plus_chain(200_000);
            let arena = LocalArena::new();
            let reported = [parse(&arena, &source).expect_err("the chain must be refused")];

            (reported, source.len())
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    let [ParseError::StructuralDepthExceeded(span)] = reported[..] else {
        panic!("expected one structural-depth error, got {reported:?}");
    };
    assert!(
        (span.end.offset as usize) <= length,
        "the span escapes the source: {span:?}"
    );
}

#[test]
fn the_deepest_admissible_operator_chain_is_spanned_iteratively() {
    let span = Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut source = String::from("$x = 0");
            for _ in 0..links_within_limit("$x = 0", " + 1") {
                source.push_str(" + 1");
            }
            source.push(';');

            let arena = LocalArena::new();
            let program = parse(&arena, &source).expect("a flat chain is valid Whim");
            let statement = program.statements.first().expect("one statement");

            (
                HasSpan::span(statement),
                source.len(),
                program.statements.len(),
            )
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    let (span, length, statements) = span;
    assert_eq!(statements, 1, "the chain is one statement");
    assert_eq!(span.start.offset, 0, "the span starts at the leftmost term");
    assert_eq!(
        span.end.offset as usize, length,
        "the span reaches the end of the chain"
    );
}

#[test]
fn the_deepest_admissible_composition_chain_is_spanned_iteratively() {
    let (span, start, end) = Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut source = String::from("type T = A");
            for _ in 0..links_within_limit("type T = A", "|A") {
                source.push_str("|A");
            }
            source.push(';');

            let arena = LocalArena::new();
            let program = parse(&arena, &source).expect("a composition chain parses");
            let statement = program.statements.first().expect("one statement");
            let Statement::TypeAlias(alias) = statement else {
                panic!("the statement is a type alias");
            };

            (
                HasSpan::span(alias.aliased),
                source.find('A').expect("the first member"),
                source.len() - 1,
            )
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    assert_eq!(
        span.start.offset as usize, start,
        "the span starts at the leftmost member"
    );
    assert_eq!(
        span.end.offset as usize, end,
        "the span reaches the last member"
    );
}

#[test]
fn the_deepest_admissible_postfix_chain_is_spanned_iteratively() {
    let span = Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut source = String::from("$x = $y");
            for _ in 0..links_within_limit("$x = $y", "?->a") {
                source.push_str("?->a");
            }
            source.push(';');

            let arena = LocalArena::new();
            let program = parse(&arena, &source).expect("a postfix chain is valid Whim");
            let statement = program.statements.first().expect("one statement");

            (HasSpan::span(statement), source.len())
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    let (span, length) = span;
    assert_eq!(span.start.offset, 0, "the span starts at the leftmost term");
    assert_eq!(
        span.end.offset as usize, length,
        "the span reaches the end of the chain"
    );
}

#[test]
fn deeply_nested_expressions_hit_the_recursion_guard_not_the_stack() {
    let fired = Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let mut source = String::from("$x = ");
            for _ in 0..4000 {
                source.push('(');
            }
            source.push('1');
            for _ in 0..4000 {
                source.push(')');
            }
            source.push(';');

            let arena = LocalArena::new();
            matches!(
                parse(&arena, &source).expect_err("deep expressions must be rejected"),
                ParseError::RecursionLimitExceeded(_)
            )
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    assert!(fired, "expected the recursion guard to fire");
}

fn plus_chain(links: usize) -> String {
    let mut source = String::from("$x = 0");
    for _ in 0..links {
        source.push_str(" + 1");
    }
    source.push(';');

    source
}

fn plus_chain_depth(links: usize) -> usize {
    let arena = LocalArena::new();
    let program = parse(&arena, &plus_chain(links)).expect("a flat chain is valid Whim");

    deepest_path(Node::Program(program)).levels
}

fn plus_chain_links_for_depth(levels: usize) -> usize {
    let base = plus_chain_depth(0);
    let step = plus_chain_depth(1) - base;
    assert!(
        levels >= base && (levels - base).is_multiple_of(step),
        "a {levels}-level chain is not reachable in steps of {step} from {base}"
    );

    (levels - base) / step
}

#[test]
fn a_tree_at_the_structural_limit_is_returned() {
    let levels = Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let links = plus_chain_links_for_depth(MAX_STRUCTURAL_DEPTH);

            plus_chain_depth(links)
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    assert_eq!(
        levels, MAX_STRUCTURAL_DEPTH,
        "a tree at the limit must be returned, and must be exactly that deep"
    );
}

#[test]
fn a_tree_past_the_structural_limit_is_a_spanned_diagnostic() {
    let (reported, length) = Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let links = plus_chain_links_for_depth(MAX_STRUCTURAL_DEPTH) + 1;
            let source = plus_chain(links);

            let arena = LocalArena::new();
            let reported =
                [parse(&arena, &source).expect_err("a tree past the limit must be refused")];

            (reported, source.len())
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    let [ParseError::StructuralDepthExceeded(span)] = reported[..] else {
        panic!("expected one structural-depth error, got {reported:?}");
    };
    assert!(
        (span.end.offset as usize) <= length,
        "the span escapes the source: {span:?}"
    );
    assert!(
        span.start.offset < span.end.offset,
        "the diagnostic must carry a non-empty span: {span:?}"
    );
}

#[test]
fn every_spine_shape_is_cut_at_the_structural_limit() {
    let measured = Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let shapes: [(&str, &str, &str); 5] = [
                ("a sum", "$x = 0", " + 1"),
                ("a property chain", "$x = $y", "->a"),
                ("a nullsafe chain", "$x = $y", "?->a"),
                ("an index chain", "$x = $y", "[0]"),
                ("a call chain", "$x = $y", "()"),
            ];

            shapes
                .iter()
                .map(|(name, head, link)| {
                    let links = links_within_limit(head, link);
                    let mut source = String::from(*head);
                    for _ in 0..links {
                        source.push_str(link);
                    }
                    source.push(';');

                    let arena = LocalArena::new();
                    let program = parse(&arena, &source)
                        .unwrap_or_else(|errors| panic!("{name} must parse: {errors:?}"));
                    let levels = deepest_path(Node::Program(program)).levels;

                    let mut source = String::from(*head);
                    for _ in 0..=links {
                        source.push_str(link);
                    }
                    source.push(';');
                    let arena = LocalArena::new();
                    let refused = matches!(
                        parse(&arena, &source),
                        Err(ParseError::StructuralDepthExceeded(_))
                    );

                    (*name, levels, refused)
                })
                .collect::<Vec<_>>()
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    for (name, levels, refused) in measured {
        assert!(
            levels <= MAX_STRUCTURAL_DEPTH,
            "{name} at its admissible length is {levels} levels deep, past the \
             {MAX_STRUCTURAL_DEPTH} the parser allows"
        );
        assert!(
            refused,
            "{name} one link past its admissible length must be refused as too deep"
        );
    }
}
