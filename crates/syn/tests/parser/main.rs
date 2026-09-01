//! Integration tests for the Whim parser spine (expressions, control
//! flow, and simple statements). Declaration parsing is covered separately.

mod declarations;
mod expressions;
mod generics;
mod grammar;
mod keywords;
mod statements;
mod traversal;
mod types;

use whim_syn::arena::LocalArena;

use whim_syn::cst::Program;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::r#type::Type;
use whim_syn::error::ParseError;
use whim_syn::parser::parse;

fn aliased_type<'a>(arena: &'a LocalArena, source: &str) -> &'a Type<'a> {
    let program = program(arena, &format!("type Alias = {source};"));
    let Statement::TypeAlias(alias) = &program.statements[0] else {
        panic!("expected a type alias");
    };

    alias.aliased
}

fn program<'a>(arena: &'a LocalArena, source: &str) -> &'a Program<'a> {
    parse(arena, source).expect("expected parsing to succeed")
}

fn statement<'a>(arena: &'a LocalArena, source: &str) -> &'a Statement<'a> {
    let program = program(arena, source);
    assert_eq!(
        program.statements.len(),
        1,
        "expected exactly one statement"
    );

    &program.statements[0]
}

fn expression<'a>(arena: &'a LocalArena, source: &str) -> &'a Expression<'a> {
    let program = program(arena, source);
    assert_eq!(
        program.statements.len(),
        1,
        "expected exactly one statement"
    );

    match &program.statements[0] {
        Statement::Expression(statement) => statement.expression,
        other => panic!("expected an expression statement, got {other:?}"),
    }
}

fn error(source: &str) -> ParseError {
    let arena = LocalArena::new();

    parse(&arena, source).expect_err("expected a parse error")
}
