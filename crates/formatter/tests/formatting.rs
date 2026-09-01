//! Snapshot-style formatter tests.

use std::fs;

use whim_formatter::format;
use whim_formatter::settings::EndOfLine;
use whim_formatter::settings::FormatSettings;
use whim_syn::arena::LocalArena;
use whim_syn::parser;

macro_rules! case {
    ($name:ident) => {
        #[test]
        fn $name() {
            run_case(
                stringify!($name),
                include_str!(concat!("cases/", stringify!($name), "/before.whim")),
                include_str!(concat!("cases/", stringify!($name), "/after.whim")),
                include!(concat!("cases/", stringify!($name), "/settings.inc")),
            );
        }
    };
}

#[track_caller]
fn run_case(name: &str, before: &str, after: &str, settings: FormatSettings) {
    let arena = LocalArena::new();
    let formatted = format(&arena, before, settings)
        .unwrap_or_else(|error| panic!("case `{name}`: `before.whim` did not parse: {error:?}"));
    assert_same(
        name,
        "formatting `before.whim` did not produce `after.whim`",
        after,
        formatted,
    );

    let arena = LocalArena::new();
    let reformatted = format(&arena, after, settings)
        .unwrap_or_else(|error| panic!("case `{name}`: `after.whim` did not parse: {error:?}"));
    assert_same(
        name,
        "`after.whim` is not stable under reformatting",
        after,
        reformatted,
    );

    assert_comments_survive(name, before, after);
}

#[track_caller]
fn assert_comments_survive(name: &str, before: &str, after: &str) {
    let written = comments_of(name, before);
    let kept = comments_of(name, after);

    assert!(
        written == kept,
        "case `{name}`: the comments did not survive formatting\n\
         ----- written ------\n{written:#?}\n\
         ----- kept ---------\n{kept:#?}\n\
         --------------------",
    );
}

/// Every comment in `source`, in order, with its whitespace removed.
fn comments_of(name: &str, source: &str) -> Vec<String> {
    let arena = LocalArena::new();
    let program = parser::parse(&arena, source)
        .unwrap_or_else(|error| panic!("case `{name}`: the source did not parse: {error:?}"));

    program
        .trivia
        .iter()
        .filter(|trivia| trivia.kind.is_comment())
        .map(|trivia| {
            trivia
                .value
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect()
        })
        .collect()
}

#[track_caller]
fn assert_same(name: &str, what: &str, expected: &str, actual: &str) {
    assert!(
        expected == actual,
        "case `{name}`: {what}\n\
         ----- expected -----\n{expected}\n\
         ----- actual -------\n{actual}\n\
         --------------------",
    );
}

case!(comment_positions_are_preserved);
case!(kitchen_sink);
case!(collections_and_generics);
case!(narrow_print_width);
case!(tabs_and_crlf);
case!(comment_leading_and_trailing);
case!(comment_dangling_in_empty_bodies);
case!(comment_between_arguments);
case!(comment_between_collection_elements);
case!(comment_between_type_members);
case!(comment_in_parameter_and_match);
case!(integer_range_types);
case!(negative_literal_types);
case!(interpolated_strings);
case!(union_type_breaks);
case!(final_locals);
case!(discard_construct);
case!(hack_layout);

#[test]
fn every_case_directory_is_registered() {
    let harness = include_str!("formatting.rs");
    let cases = fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/cases"))
        .expect("the cases directory should exist");

    for entry in cases {
        let path = entry.expect("a readable directory entry").path();
        if !path.is_dir() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a valid directory name");
        if name.starts_with('-') {
            continue;
        }

        assert!(
            harness.contains(&format!("case!({name})")),
            "case directory `{name}` is not registered with `case!({name})` in formatting.rs",
        );
    }
}
