use whim_syn::arena::LocalArena;

use whim_syn::cst::call::Call;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::statement::TopLevelStatement;
use whim_syn::error::ParseError;

use crate::error;
use crate::expression;
use crate::program;
use crate::top_level_statement;

#[test]
fn contextual_keywords_are_usable_as_names() {
    let arena = LocalArena::new();

    let TopLevelStatement::Constant(_) = top_level_statement(&arena, "const enum = 'x';") else {
        panic!("expected a constant declaration named `enum`");
    };
    assert!(matches!(
        expression(&arena, "write_line!(enum);"),
        Expression::Construct(_)
    ));
    assert!(matches!(
        expression(&arena, "$x = out;"),
        Expression::Assignment(_)
    ));

    let TopLevelStatement::Function(_) = top_level_statement(&arena, "function vec() {}") else {
        panic!("expected a function declaration named `vec`");
    };
    assert!(matches!(
        expression(&arena, "int($x);"),
        Expression::Call(_)
    ));

    let TopLevelStatement::Function(_) = top_level_statement(&arena, "function f(): int {}") else {
        panic!("expected a function returning `int`");
    };

    program(
        &arena,
        "const where = 1; function where() {} where(); $value = where;",
    );
    program(
        &arena,
        "class Query<T> { public function where(): void where T: int {} }",
    );
    assert!(matches!(
        expression(&arena, "$query->where();"),
        Expression::Call(Call::Method(_))
    ));
}

#[test]
fn soft_reserved_keywords_are_names_only_in_call_position() {
    let arena = LocalArena::new();

    assert!(matches!(expression(&arena, "as($x);"), Expression::Call(_)));
    let Expression::Call(Call::Method(_)) = expression(&arena, "$o->is();") else {
        panic!("expected a method call named `is`");
    };

    assert!(matches!(error("$x = as;"), ParseError::UnexpectedToken(..)));
}

#[test]
fn reserved_keywords_are_usable_only_as_member_names() {
    let arena = LocalArena::new();

    let Expression::Call(Call::Method(_)) = expression(&arena, "$o->match();") else {
        panic!("expected a method call named `match`");
    };

    assert!(matches!(
        error("function if() {}"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = while;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn keyword_named_class_members() {
    let arena = LocalArena::new();

    program(
        &arena,
        "class C { public function match(): int {} const enum = 1; function int(): int {} }",
    );

    assert!(matches!(
        expression(&arena, "C::enum;"),
        Expression::Access(_)
    ));
    let Expression::Call(Call::StaticMethod(_)) = expression(&arena, "C::match();") else {
        panic!("expected a static method call named `match`");
    };
}

#[test]
fn constant_named_after_a_keyword_round_trips() {
    let arena = LocalArena::new();
    let program = program(&arena, "const enum = 'hello'; write_line!(enum);");
    assert_eq!(program.statements.len(), 2);
}
