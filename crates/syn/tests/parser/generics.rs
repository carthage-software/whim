use whim_syn::arena::LocalArena;

use whim_syn::cst::access::ClassReference;
use whim_syn::cst::call::Call;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeVariance;
use whim_syn::error::ParseError;

use crate::aliased_type;
use crate::error;
use crate::expression;
use crate::statement;

#[test]
fn class_with_type_parameters() {
    let arena = LocalArena::new();
    let Statement::Class(class) = statement(
        &arena,
        "class Box<in T: object = mixed, out U> extends Base<T> {}",
    ) else {
        panic!("expected a class");
    };
    let parameters = class.type_parameters.expect("type parameters");
    assert_eq!(parameters.parameters.len(), 2);

    let first = parameters.parameters.get(0).expect("first parameter");
    assert!(matches!(first.variance, Some(TypeVariance::In(_))));
    assert_eq!(first.name.value, "T");
    assert!(first.bound.is_some());
    assert!(first.default.is_some());

    let second = parameters.parameters.get(1).expect("second parameter");
    assert!(matches!(second.variance, Some(TypeVariance::Out(_))));

    let extends = class.extends.as_ref().expect("extends");
    let base = extends.types.first().expect("a base type");
    assert!(base.type_arguments.is_some());
}

#[test]
fn a_type_parameter_may_have_multiple_bounds() {
    let arena = LocalArena::new();
    let Statement::Function(function) = statement(&arena, "function f<T: A + B + C>(): void { }")
    else {
        panic!("expected a function");
    };
    let parameters = function.type_parameters.expect("type parameters");
    let bound = parameters
        .parameters
        .get(0)
        .expect("a parameter")
        .bound
        .as_ref()
        .expect("a bound");
    assert_eq!(bound.types.len(), 3);
}

#[test]
fn an_enum_parses_type_parameters_for_a_precise_diagnostic() {
    // The grammar admits type parameters on an enum so the compiler can reject
    // a generic enum with a precise message and span, rather than a bare
    // syntax error.
    let arena = LocalArena::new();
    let Statement::Enum(declaration) = statement(&arena, "enum E<T> { case A; }") else {
        panic!("expected an enum");
    };
    assert_eq!(
        declaration
            .type_parameters
            .expect("type parameters")
            .parameters
            .len(),
        1
    );
}

#[test]
fn generic_type_alias() {
    let arena = LocalArena::new();
    let Statement::TypeAlias(alias) = statement(&arena, "type Pair<A, B> = (A, B);") else {
        panic!("expected a type alias");
    };
    assert_eq!(alias.name.value, "Pair");
    assert_eq!(
        alias
            .type_parameters
            .expect("type parameters")
            .parameters
            .len(),
        2
    );
}

#[test]
fn function_with_type_parameters() {
    let arena = LocalArena::new();
    let Statement::Function(function) =
        statement(&arena, "function map<T, U>(fn(T): U $f): U { return $f; }")
    else {
        panic!("expected a function");
    };
    assert_eq!(
        function
            .type_parameters
            .expect("type params")
            .parameters
            .len(),
        2
    );
}

#[test]
fn turbofish_on_a_function_call() {
    let arena = LocalArena::new();
    let Expression::Call(Call::Function(call)) = expression(&arena, "make::<int, string>();")
    else {
        panic!("expected a function call");
    };
    assert_eq!(call.type_arguments.expect("turbofish").arguments.len(), 2);
}

#[test]
fn turbofish_on_a_class_reference_and_static_call() {
    let arena = LocalArena::new();
    let Expression::Call(Call::StaticMethod(call)) = expression(&arena, "Vector::<int>::new();")
    else {
        panic!("expected a static method call");
    };
    let ClassReference::Named(named) = call.class else {
        panic!("expected a named class reference");
    };
    assert!(named.type_arguments.is_some());
    assert!(call.type_arguments.is_none());
}

#[test]
fn turbofish_requires_a_call() {
    assert!(matches!(
        error("$x = foo::<int>;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn empty_generic_lists_are_rejected() {
    assert!(matches!(
        error("type T = Box<>;"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$y = compute::<>();"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("class C<> {}"),
        ParseError::UnexpectedToken(..)
    ));

    let arena = LocalArena::new();
    assert!(matches!(aliased_type(&arena, "Box<int>"), Type::Named(_)));
}
