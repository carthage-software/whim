use whim_syn::arena::LocalArena;

use whim_syn::cst::call::Call;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::r#type::IntegerRangeBound;
use whim_syn::cst::r#type::IntegerRangeOperator;
use whim_syn::cst::r#type::NegativeLiteralType;
use whim_syn::cst::r#type::Type;
use whim_syn::error::ParseError;
use whim_syn::token::kind::TokenKind;

use crate::aliased_type;
use crate::error;
use crate::expression;
use crate::statement;

#[test]
fn function_type() {
    let arena = LocalArena::new();
    let Type::Function(function) = aliased_type(&arena, "fn(string, int, =float): string") else {
        panic!("expected a function type");
    };

    let Some(signature) = &function.signature else {
        panic!("expected a parametric function type");
    };
    assert_eq!(signature.parameters.len(), 3);
    let optionals: Vec<_> = signature
        .parameters
        .iter()
        .map(|parameter| parameter.equals.is_some())
        .collect();
    assert_eq!(optionals, vec![false, false, true]);
    assert!(matches!(signature.return_type, Type::String(_)));
}

#[test]
fn function_type_with_no_parameters() {
    let arena = LocalArena::new();
    let Type::Function(function) = aliased_type(&arena, "fn(): void") else {
        panic!("expected a function type");
    };

    let Some(signature) = &function.signature else {
        panic!("expected a parametric function type");
    };
    assert!(signature.parameters.is_empty());
}

#[test]
fn bare_function_type() {
    let arena = LocalArena::new();
    let Type::Function(function) = aliased_type(&arena, "fn") else {
        panic!("expected a function type");
    };

    assert!(function.signature.is_none());
}

#[test]
fn bare_function_type_as_a_parameter_type() {
    let arena = LocalArena::new();
    let Statement::Function(function) = statement(&arena, "function f(fn $callback): void {}")
    else {
        panic!("expected a function declaration");
    };

    let Some(parameter) = function.parameter_list.parameters.first() else {
        panic!("expected one parameter");
    };
    let Some(Type::Function(declared)) = parameter.r#type else {
        panic!("expected a function type");
    };
    assert!(declared.signature.is_none());
    assert_eq!(parameter.variable.name, "$callback");
}

#[test]
fn an_interpolated_string_is_not_a_literal_type() {
    assert!(matches!(
        error(r#"type Greeting = "hello, $name";"#),
        ParseError::UnexpectedToken(_, TokenKind::StringPart, _)
    ));
}

#[test]
fn array_vec_and_dict_types() {
    let arena = LocalArena::new();
    let Type::Array(array) = aliased_type(&arena, "array<int, string>") else {
        panic!("expected an array type");
    };
    let arguments = array.type_arguments.expect("arguments");
    assert!(matches!(
        arguments.arguments.as_slice()[0].r#type,
        Type::Int(_)
    ));
    assert!(matches!(
        arguments.arguments.as_slice()[1].r#type,
        Type::String(_)
    ));

    let Type::Vec(vec) = aliased_type(&arena, "vec<int>") else {
        panic!("expected a vec type");
    };
    assert!(matches!(
        vec.type_arguments.expect("arguments").arguments.as_slice()[0].r#type,
        Type::Int(_)
    ));

    let Type::Dict(dict) = aliased_type(&arena, "dict<string, int>") else {
        panic!("expected a dict type");
    };
    let arguments = dict.type_arguments.expect("arguments");
    assert!(matches!(
        arguments.arguments.as_slice()[0].r#type,
        Type::String(_)
    ));
    assert!(matches!(
        arguments.arguments.as_slice()[1].r#type,
        Type::Int(_)
    ));
}

#[test]
fn classname_types() {
    let arena = LocalArena::new();
    let Type::Classname(classname) = aliased_type(&arena, "classname<User>") else {
        panic!("expected a classname type");
    };
    assert!(matches!(classname.inner, Type::Named(_)));

    assert!(matches!(
        error("type Alias = classname;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn member_types() {
    let arena = LocalArena::new();
    let Type::Self_(self_type) = aliased_type(&arena, "self::find<int>") else {
        panic!("expected a self type");
    };

    let member = self_type.member.expect("member");
    assert_eq!(member.name.value, "find");
    assert_eq!(
        member
            .type_arguments
            .expect("member type arguments")
            .arguments
            .len(),
        1
    );

    let Type::Named(named) = aliased_type(&arena, "Repository<User>::find<int>") else {
        panic!("expected a named type");
    };
    assert_eq!(
        named
            .type_arguments
            .expect("owner arguments")
            .arguments
            .len(),
        1
    );
    assert_eq!(
        named
            .member
            .expect("member")
            .type_arguments
            .expect("member arguments")
            .arguments
            .len(),
        1
    );
}

#[test]
fn nested_classname_types_split_the_shift_token() {
    let arena = LocalArena::new();
    let Type::Vec(vec) = aliased_type(&arena, "vec<classname<User>>") else {
        panic!("expected a vec type");
    };
    assert!(matches!(
        vec.type_arguments.expect("arguments").arguments.as_slice()[0].r#type,
        Type::Classname(_)
    ));

    let Type::Dict(dict) = aliased_type(&arena, "dict<string, classname<User>>") else {
        panic!("expected a dict type");
    };
    let arguments = dict.type_arguments.expect("arguments");
    assert!(matches!(
        arguments.arguments.as_slice()[0].r#type,
        Type::String(_)
    ));
    assert!(matches!(
        arguments.arguments.as_slice()[1].r#type,
        Type::Classname(_)
    ));
}

#[test]
fn classname_and_typename_are_usable_as_names() {
    let arena = LocalArena::new();

    let Statement::Function(_) = statement(&arena, "function classname(): int { return 1; }")
    else {
        panic!("expected a function declaration named `classname`");
    };

    let Statement::Constant(_) = statement(&arena, "const typename = 1;") else {
        panic!("expected a constant declaration named `typename`");
    };

    let Type::Named(typename) = aliased_type(&arena, "typename<int>") else {
        panic!("expected `typename` to be an ordinary generic named type");
    };
    assert!(typename.type_arguments.is_some());

    let Expression::Call(Call::Method(_)) = expression(&arena, "$obj->classname();") else {
        panic!("expected a method call named `classname`");
    };
}

#[test]
fn tuple_types() {
    let arena = LocalArena::new();
    assert!(matches!(
        aliased_type(&arena, "(int)"),
        Type::Parenthesized(_)
    ));

    let Type::Tuple(one) = aliased_type(&arena, "(int,)") else {
        panic!("expected a single-element tuple");
    };
    assert_eq!(one.elements.len(), 1);

    let Type::Tuple(many) = aliased_type(&arena, "(int, string, bool)") else {
        panic!("expected a tuple");
    };
    assert_eq!(many.elements.len(), 3);

    assert!(matches!(
        error("type Alias = ();"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn callable_is_no_longer_a_keyword() {
    let arena = LocalArena::new();
    let Type::Named(named) = aliased_type(&arena, "callable") else {
        panic!("expected a named type");
    };
    assert_eq!(named.identifier.value(), "callable");
    assert!(named.type_arguments.is_none());
}

#[test]
fn question_mark_nullable_type_is_removed() {
    assert!(matches!(
        error("type Alias = ?int;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn builtin_types_are_their_own_variants() {
    let arena = LocalArena::new();
    assert!(matches!(aliased_type(&arena, "string"), Type::String(_)));
    assert!(matches!(aliased_type(&arena, "int"), Type::Int(_)));
    assert!(matches!(aliased_type(&arena, "float"), Type::Float(_)));
    assert!(matches!(aliased_type(&arena, "bool"), Type::Bool(_)));
    assert!(matches!(aliased_type(&arena, "void"), Type::Void(_)));
    assert!(matches!(aliased_type(&arena, "mixed"), Type::Mixed(_)));
    assert!(matches!(aliased_type(&arena, "never"), Type::Never(_)));
    assert!(matches!(aliased_type(&arena, "object"), Type::Object(_)));
    assert!(matches!(aliased_type(&arena, "array"), Type::Array(_)));
    assert!(matches!(aliased_type(&arena, "Int"), Type::Named(_)));
}

#[test]
fn negative_numeric_literal_types_have_their_own_node() {
    let arena = LocalArena::new();

    let Type::NegativeLiteral(NegativeLiteralType::Integer { literal, .. }) =
        aliased_type(&arena, "-1")
    else {
        panic!("expected a negative integer literal type");
    };
    assert_eq!(literal.value, 1);

    let Type::NegativeLiteral(NegativeLiteralType::Float { literal, .. }) =
        aliased_type(&arena, "-1.5")
    else {
        panic!("expected a negative float literal type");
    };
    assert_eq!(literal.value, 1.5);

    assert!(matches!(
        error("type Alias = -string;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn integer_range_types_preserve_their_bounds_and_operator() {
    let arena = LocalArena::new();

    let Type::IntegerRange(exclusive) = aliased_type(&arena, "1..10") else {
        panic!("expected an exclusive integer range");
    };
    assert!(matches!(
        exclusive.lower,
        Some(IntegerRangeBound::Positive(literal)) if literal.value == 1
    ));
    assert!(matches!(
        exclusive.operator,
        IntegerRangeOperator::Exclusive(_)
    ));
    assert!(matches!(
        exclusive.upper,
        Some(IntegerRangeBound::Positive(literal)) if literal.value == 10
    ));

    let Type::IntegerRange(inclusive) = aliased_type(&arena, "-10..=-1") else {
        panic!("expected an inclusive signed integer range");
    };
    assert!(matches!(
        inclusive.lower,
        Some(IntegerRangeBound::Negative { literal, .. }) if literal.value == 10
    ));
    assert!(matches!(
        inclusive.operator,
        IntegerRangeOperator::Inclusive(_)
    ));
    assert!(matches!(
        inclusive.upper,
        Some(IntegerRangeBound::Negative { literal, .. }) if literal.value == 1
    ));

    let Type::IntegerRange(open_upper) = aliased_type(&arena, "0..") else {
        panic!("expected an open-ended upper range");
    };
    assert!(open_upper.upper.is_none());

    let Type::IntegerRange(open_lower) = aliased_type(&arena, "..=100") else {
        panic!("expected an open-ended lower range");
    };
    assert!(open_lower.lower.is_none());

    assert!(matches!(
        error("type Alias = ..;"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("type Alias = 0..1.5;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn named_type_with_type_arguments() {
    let arena = LocalArena::new();
    let Type::Named(named) = aliased_type(&arena, "Vector<int, string>") else {
        panic!("expected a named type");
    };
    let arguments = named.type_arguments.expect("type arguments");
    assert_eq!(arguments.arguments.len(), 2);
}

#[test]
fn nested_type_arguments_split_the_shift_token() {
    let arena = LocalArena::new();
    let Type::Named(named) = aliased_type(&arena, "Map<string, Vector<int>>") else {
        panic!("expected a named type");
    };
    assert_eq!(named.type_arguments.expect("arguments").arguments.len(), 2);
}

#[test]
fn bare_and_parametric_array_vec_and_dict() {
    let arena = LocalArena::new();

    let Type::Array(bare_array) = aliased_type(&arena, "array") else {
        panic!("expected an array type");
    };
    assert!(bare_array.type_arguments.is_none());

    let Type::Array(array) = aliased_type(&arena, "array<int, string>") else {
        panic!("expected an array type");
    };
    assert!(array.type_arguments.is_some());

    let Type::Vec(bare_vec) = aliased_type(&arena, "vec") else {
        panic!("expected a vec type");
    };
    assert!(bare_vec.type_arguments.is_none());

    let Type::Vec(vec) = aliased_type(&arena, "vec<int>") else {
        panic!("expected a vec type");
    };
    assert!(vec.type_arguments.is_some());

    let Type::Dict(bare_dict) = aliased_type(&arena, "dict") else {
        panic!("expected a dict type");
    };
    assert!(bare_dict.type_arguments.is_none());

    let Type::Dict(dict) = aliased_type(&arena, "dict<string, int>") else {
        panic!("expected a dict type");
    };
    assert!(dict.type_arguments.is_some());
}

#[test]
fn type_composition_uses_union_intersection_and_negation_precedence() {
    let arena = LocalArena::new();

    let Type::Union(union) = aliased_type(&arena, "A | B & !C") else {
        panic!("expected a union");
    };
    let Type::Intersection(intersection) = union.right else {
        panic!("expected intersection to bind inside union");
    };
    assert!(matches!(intersection.right, Type::Negated(_)));

    let Type::Intersection(intersection) = aliased_type(&arena, "!A & B") else {
        panic!("expected an intersection");
    };
    assert!(matches!(intersection.left, Type::Negated(_)));

    assert!(matches!(
        aliased_type(&arena, "(A & B) | C"),
        Type::Union(_)
    ));
    assert!(matches!(
        aliased_type(&arena, "(A | B) & C"),
        Type::Intersection(_)
    ));
    assert!(matches!(aliased_type(&arena, "A | B | C"), Type::Union(_)));
    assert!(matches!(
        aliased_type(&arena, "A & B & C"),
        Type::Intersection(_)
    ));
    assert!(matches!(aliased_type(&arena, "!(A | B)"), Type::Negated(_)));
}
