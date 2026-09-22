use whim_span::HasSpan;
use whim_syn::arena::LocalArena;

use whim_syn::cst::access::ClassReference;
use whim_syn::cst::call::Call;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::statement::TopLevelStatement;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeVariance;
use whim_syn::error::ParseError;

use crate::aliased_type;
use crate::error;
use crate::expression;
use crate::program;
use crate::top_level_statement;

#[test]
fn method_where_clauses_keep_constraints_and_spans() {
    let arena = LocalArena::new();
    let source = "abstract class Vector<T> {
        public function sum(): int where T: int { return 0; }
        public abstract function combine<U>(U $value): U where T: U, U: A&B;
        public function visit($value) where T: A, T: B, {}
        public function clear(): void {}
    }";
    let TopLevelStatement::Class(class) = top_level_statement(&arena, source) else {
        panic!("expected a class");
    };
    let methods = class
        .members
        .iter()
        .map(|member| match member {
            ClassLikeMember::Method(method) => method,
            other => panic!("expected a method, got {other:?}"),
        })
        .collect::<Vec<_>>();
    let clause = methods[0].where_clause.as_ref().expect("a where clause");
    let constraint = clause.constraints.first().unwrap();
    let span = clause.span();
    assert_eq!(
        &source[span.start.offset as usize..span.end.offset as usize],
        "where T: int"
    );
    assert_eq!(clause.r#where.value, "where");
    assert_eq!(constraint.parameter.value, "T");
    let span = constraint.span();
    assert_eq!(
        &source[span.start.offset as usize..span.end.offset as usize],
        "T: int"
    );
    let span = constraint.colon;
    assert_eq!(
        &source[span.start.offset as usize..span.end.offset as usize],
        ":"
    );
    assert!(matches!(constraint.bound, Type::Int(_)));
    assert!(matches!(methods[0].body, MethodBody::Concrete(_)));

    let clause = methods[1].where_clause.as_ref().unwrap();
    assert!(methods[1].type_parameters.is_some());
    assert!(matches!(methods[1].body, MethodBody::Abstract(_)));
    assert_eq!(clause.constraints.len(), 2);
    assert_eq!(clause.constraints.tokens.len(), 1);
    assert_eq!(clause.constraints.tokens[0].value, ",");
    assert!(matches!(clause.constraints.nodes[0].bound, Type::Named(_)));
    assert!(matches!(
        clause.constraints.nodes[1].bound,
        Type::Intersection(_)
    ));

    let clause = methods[2].where_clause.as_ref().unwrap();
    assert!(methods[2].return_type.is_none());
    assert_eq!(clause.constraints.len(), 2);
    assert!(clause.constraints.get_trailing_token().is_some());
    let span = clause.span();
    assert_eq!(
        &source[span.start.offset as usize..span.end.offset as usize],
        "where T: A, T: B,"
    );
    assert!(
        clause
            .constraints
            .iter()
            .all(|constraint| constraint.parameter.value == "T")
    );
    assert!(methods[3].where_clause.is_none());
}

#[test]
fn where_constraints_accept_composite_types_and_method_forms() {
    let arena = LocalArena::new();
    for bound in [
        "int|float",
        "Named&Stored",
        "vec<dict<string, vec<U>>>",
        "(int, U)",
        "fn(T, =U): vec<T|U>",
        "dict['value' => U, ...<string, T>]",
        "Box<U> #{ value: T, ... }",
        "string & !''",
        "0..=100",
        "U",
    ] {
        for declaration in [
            format!("class C<T> {{ public static function f<U>(): void where T: {bound} {{}} }}"),
            format!("interface C<T> {{ public function f<U>() where T: {bound},; }}"),
            format!("enum C {{ case A; public function f<T, U>() where T: {bound} {{}} }}"),
        ] {
            program(&arena, &declaration);
        }
    }
}

#[test]
fn malformed_where_constraints_report_parse_errors() {
    for clause in [
        "where",
        "where T",
        "where T:",
        "where , T: int",
        "where T: int,, U: string",
        "where T: int U: string",
    ] {
        for ending in ["{}", ";"] {
            let source = format!("class C<T> {{ public function f() {clause} {ending} }}");
            assert!(
                matches!(error(&source), ParseError::UnexpectedToken(..)),
                "{source}"
            );
        }
    }
}

#[test]
fn class_with_type_parameters() {
    let arena = LocalArena::new();
    let TopLevelStatement::Class(class) = top_level_statement(
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
    let TopLevelStatement::Function(function) =
        top_level_statement(&arena, "function f<T: A + B + C>(): void { }")
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
    let TopLevelStatement::Enum(declaration) = top_level_statement(&arena, "enum E<T> { case A; }")
    else {
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
    let TopLevelStatement::TypeAlias(alias) =
        top_level_statement(&arena, "type Pair<A, B> = (A, B);")
    else {
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
    let TopLevelStatement::Function(function) =
        top_level_statement(&arena, "function map<T, U>(fn(T): U $f): U { return $f; }")
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
