use whim_span::HasSpan;
use whim_span::Position;
use whim_syn::arena::LocalArena;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::pattern::Pattern;
use whim_syn::error::ParseError;
use whim_syn::parser::parse_fragment;

use crate::error;
use crate::expression;
use crate::program;

#[test]
fn fragments_keep_spans_in_their_full_source() {
    let arena = LocalArena::new();
    let source = "first();\n\nsecond();";
    let fragment = &source[10..];
    let parsed = parse_fragment(&arena, source, fragment, Position::new(10)).expect("parse");

    assert_eq!(parsed.source_text, source);
    assert_eq!(parsed.span().start.offset, 10);
    assert_eq!(parsed.span().end.offset, source.len() as u32);
}

#[test]
fn recursive_patterns_share_one_match_cst() {
    let arena = LocalArena::new();

    let Expression::Match(matching) = expression(
        &arena,
        "match ($value) { ($first @ int, $second @ string) => $first, vec[int, ...string] => 1, $_ => 0 };",
    ) else {
        panic!("expected a match");
    };
    assert_eq!(matching.arms.len(), 3);
    assert!(matches!(
        matching.arms.as_slice()[0].pattern,
        Pattern::Tuple(_)
    ));
    assert!(matches!(
        matching.arms.as_slice()[1].pattern,
        Pattern::Vec(_)
    ));
    assert!(matches!(
        matching.arms.as_slice()[2].pattern,
        Pattern::Variable(_)
    ));

    assert!(matches!(
        error("match ($value) { 1, 2 => 'number' };"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("match ($value) { default => 'other' };"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn documented_argument_list_families_accept_only_their_own_forms() {
    let arena = LocalArena::new();

    for source in [
        "f(1, name: 2,);",
        "f(1, ?);",
        "f(1, ...);",
        "$object->method::<int>(?, ...);",
        "$object?->method::<int>(1, name: 2);",
        "Static::method::<int>(1, ...);",
        "new Box::<int>(1, name: 2,);",
        "#[Example(1, name: 2,)] function f(): void {}",
    ] {
        program(&arena, source);
    }

    for source in [
        "f(...,);",
        "new Box(?);",
        "#[Example(?)] function f(): void {}",
        "$object?->method(?);",
        "$object->method::<int>;",
        "Static::method::<int>;",
        "callable::<int>;",
    ] {
        assert!(
            matches!(error(source), ParseError::UnexpectedToken(..)),
            "{source}"
        );
    }
}

#[test]
fn numeric_ranges_and_nested_generic_closers_are_lexically_unambiguous() {
    let arena = LocalArena::new();

    for source in [
        "type R = -10..=10;",
        "type Lower = ..0;",
        "type Upper = 0..;",
        "type Nested = vec<classname<Box<0..=255>>>;",
    ] {
        program(&arena, source);
    }

    for source in ["type Empty = ..;", "type FloatBound = 0..1.5;"] {
        assert!(
            matches!(error(source), ParseError::UnexpectedToken(..)),
            "{source}"
        );
    }
}
