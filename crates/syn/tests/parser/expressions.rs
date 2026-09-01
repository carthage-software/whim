use whim_syn::arena::LocalArena;

use whim_syn::cst::construct::Construct;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::expression::InterpolatedStringPart;
use whim_syn::cst::operation::AssignmentOperator;
use whim_syn::cst::operation::AssignmentTarget;
use whim_syn::cst::operation::BinaryOperator;
use whim_syn::cst::operation::DestructureTarget;
use whim_syn::cst::operation::TypeOperator;
use whim_syn::cst::pattern::Pattern;
use whim_syn::error::Expected;
use whim_syn::error::ParseError;
use whim_syn::token::kind::TokenKind;

use crate::error;
use crate::expression;

#[test]
fn literals_and_variables() {
    let arena = LocalArena::new();
    assert!(matches!(expression(&arena, "42;"), Expression::Literal(_)));
    assert!(matches!(
        expression(&arena, "$name;"),
        Expression::Variable(_)
    ));
    assert!(matches!(
        expression(&arena, "'hello';"),
        Expression::Literal(_)
    ));
    assert!(matches!(
        expression(&arena, r#""\u{41}";"#),
        Expression::Literal(_)
    ));
    assert!(matches!(
        expression(&arena, "true;"),
        Expression::Literal(_)
    ));
}

#[test]
fn return_is_an_expression_with_an_optional_value() {
    let arena = LocalArena::new();
    let Expression::Return(with_value) = expression(&arena, "return 42;") else {
        panic!("expected a return expression");
    };
    assert!(matches!(with_value.value, Some(Expression::Literal(_))));

    let Expression::Return(without_value) = expression(&arena, "return;") else {
        panic!("expected a return expression");
    };
    assert!(without_value.value.is_none());

    let Expression::Return(qualified_value) = expression(&arena, "return Foo\\Bar::value();")
    else {
        panic!("expected a return expression");
    };
    assert!(qualified_value.value.is_some());

    let Expression::Match(matching) = expression(
        &arena,
        "match ($value) { true => return 1, false => return, };",
    ) else {
        panic!("expected a match expression");
    };
    let arms = matching.arms.as_slice();
    assert!(matches!(arms[0].expression, Expression::Return(_)));
    assert!(matches!(arms[1].expression, Expression::Return(_)));
}

#[test]
fn filled_vec_and_membership_constructs_have_dedicated_nodes() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "vec[$value; $size];"),
        Expression::VecFill(_)
    ));
    assert!(matches!(
        expression(&arena, "vec[$value, $size];"),
        Expression::Vec(_)
    ));
    assert!(matches!(
        expression(&arena, "contains!($values, $needle);"),
        Expression::Construct(Construct::Contains(_))
    ));
    assert!(matches!(
        expression(&arena, "contains_key!($values, $key);"),
        Expression::Construct(Construct::ContainsKey(_))
    ));
}

#[test]
fn double_quoted_strings_support_the_two_interpolation_forms() {
    let arena = LocalArena::new();
    let Expression::InterpolatedString(string) =
        expression(&arena, r#""Hello, $name: {$order->total + 1}!";"#)
    else {
        panic!("expected an interpolated string");
    };

    assert_eq!(string.parts.len(), 5);
    assert!(matches!(
        string.parts[0],
        InterpolatedStringPart::Literal(_)
    ));
    assert!(matches!(
        string.parts[1],
        InterpolatedStringPart::Variable(_)
    ));
    assert!(matches!(
        string.parts[2],
        InterpolatedStringPart::Literal(_)
    ));
    assert!(matches!(
        string.parts[3],
        InterpolatedStringPart::Expression(_)
    ));
    assert!(matches!(
        string.parts[4],
        InterpolatedStringPart::Literal(_)
    ));
}

#[test]
fn interpolation_may_begin_immediately_after_the_opening_quote() {
    let arena = LocalArena::new();
    let Expression::InterpolatedString(string) = expression(&arena, r#""{$id}";"#) else {
        panic!("expected an interpolated string");
    };

    assert_eq!(string.parts.len(), 3);
    assert!(matches!(
        string.parts[0],
        InterpolatedStringPart::Literal(_)
    ));
    assert!(matches!(
        string.parts[1],
        InterpolatedStringPart::Expression(_)
    ));
    assert!(matches!(
        string.parts[2],
        InterpolatedStringPart::Literal(_)
    ));
}

#[test]
fn interpolation_is_recursive_and_escaped_markers_stay_literal() {
    let arena = LocalArena::new();
    let Expression::InterpolatedString(string) =
        expression(&arena, r#""outer {"inner $name"} \{literal\} \$raw";"#)
    else {
        panic!("expected an interpolated string");
    };
    let InterpolatedStringPart::Expression(interpolation) = &string.parts[1] else {
        panic!("expected a braced expression");
    };
    assert!(matches!(
        interpolation.expression,
        Expression::InterpolatedString(_)
    ));

    assert!(matches!(
        expression(&arena, r"'Hello, $name and {literal}';"),
        Expression::Literal(_)
    ));
}

#[test]
fn interpolation_rejects_unclosed_and_unescaped_braces() {
    assert!(matches!(
        error(r#""value: {$value";"#),
        ParseError::SyntaxError(..)
    ));
    assert!(matches!(
        error(r#""value: }";"#),
        ParseError::InvalidStringLiteral(..)
    ));
}

#[test]
fn multiplicative_binds_tighter_than_additive() {
    let arena = LocalArena::new();
    let Expression::Binary(add) = expression(&arena, "1 + 2 * 3;") else {
        panic!("expected a binary expression");
    };

    assert!(matches!(add.operator, BinaryOperator::Addition(_)));
    assert!(matches!(add.lhs, Expression::Literal(_)));
    assert!(matches!(add.rhs, Expression::Binary(_)));
}

#[test]
fn bitwise_binds_tighter_than_comparison() {
    let arena = LocalArena::new();
    let Expression::Binary(equal) = expression(&arena, "$flags & MASK == MASK;") else {
        panic!("expected a binary expression");
    };

    assert!(matches!(equal.operator, BinaryOperator::Equal(_)));
    assert!(matches!(equal.lhs, Expression::Binary(_)));
}

#[test]
fn exponent_is_right_associative_and_binds_tighter_than_unary() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "-2 ** 2;"),
        Expression::UnaryPrefix(_)
    ));

    let Expression::Binary(outer) = expression(&arena, "2 ** 3 ** 2;") else {
        panic!("expected a binary expression");
    };
    assert!(matches!(outer.rhs, Expression::Binary(_)));
}

#[test]
fn chained_comparison_is_an_error() {
    assert!(matches!(
        error("$a < $b < $c;"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$a == $b < $c;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn chained_type_operation_has_specific_error() {
    let ParseError::UnexpectedToken(expected, found, _) = error("$x as int as float;") else {
        panic!("expected an unexpected-token error");
    };

    assert_eq!(
        expected,
        Expected::Description("parentheses around the chained type operation")
    );
    assert_eq!(found, TokenKind::As);
}

#[test]
fn parentheses_disambiguate_non_associative_operators() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "($a < $b) < $c;"),
        Expression::Binary(_)
    ));
    assert!(matches!(
        expression(&arena, "$a < ($b < $c);"),
        Expression::Binary(_)
    ));
    assert!(matches!(
        expression(&arena, "($a == $b) == $c;"),
        Expression::Binary(_)
    ));
    assert!(matches!(
        expression(&arena, "($a is int) is float;"),
        Expression::TypeOperation(_)
    ));
}

#[test]
fn assignment_is_right_associative() {
    let arena = LocalArena::new();
    let Expression::Assignment(assignment) = expression(&arena, "$a = $b = 1;") else {
        panic!("expected an assignment");
    };

    assert!(matches!(assignment.operator, AssignmentOperator::Assign(_)));
    assert!(matches!(assignment.value, Expression::Assignment(_)));
}

#[test]
fn invalid_assignment_target_is_an_error() {
    assert!(matches!(
        error("1 = 2;"),
        ParseError::InvalidAssignmentTarget(_)
    ));
    assert!(matches!(
        error("$a + $b = 2;"),
        ParseError::InvalidAssignmentTarget(_)
    ));
}

#[test]
fn array_append_target() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "$items[] = 1;"),
        Expression::Assignment(_)
    ));
}

#[test]
fn destructuring_assignment() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "($a, $b) = $pair;"),
        Expression::Assignment(_)
    ));
    assert!(matches!(
        expression(&arena, "($a, ($b, $c)) = $nested;"),
        Expression::Assignment(_)
    ));
}

#[test]
fn type_operations() {
    let arena = LocalArena::new();

    let Expression::TypeOperation(check) = expression(&arena, "$x is int;") else {
        panic!("expected a type operation");
    };
    assert!(matches!(check.operator, TypeOperator::Check(_)));

    let Expression::TypeOperation(assert) = expression(&arena, "$x as string;") else {
        panic!("expected a type operation");
    };
    assert!(matches!(assert.operator, TypeOperator::Assert(_)));

    let Expression::TypeOperation(assert_or_null) = expression(&arena, "$x ?as float;") else {
        panic!("expected a type operation");
    };
    assert!(matches!(
        assert_or_null.operator,
        TypeOperator::AssertOrNull(..)
    ));
}

#[test]
fn type_operation_binds_looser_than_arithmetic() {
    let arena = LocalArena::new();
    let Expression::TypeOperation(operation) = expression(&arena, "$a + $b as float;") else {
        panic!("expected a type operation");
    };
    assert!(matches!(operation.operand, Expression::Binary(_)));
}

#[test]
fn coalesce_defaulting() {
    let arena = LocalArena::new();
    let Expression::Binary(coalesce) = expression(&arena, "$x ?as int ?? 0;") else {
        panic!("expected a binary expression");
    };
    assert!(matches!(coalesce.operator, BinaryOperator::NullCoalesce(_)));
    assert!(matches!(coalesce.lhs, Expression::TypeOperation(_)));
}

#[test]
fn calls_and_member_access() {
    let arena = LocalArena::new();
    assert!(matches!(expression(&arena, "foo();"), Expression::Call(_)));
    assert!(matches!(
        expression(&arena, "foo(1, 2);"),
        Expression::Call(_)
    ));
    assert!(matches!(
        expression(&arena, "foo(name: 1);"),
        Expression::Call(_)
    ));
    assert!(matches!(
        expression(&arena, "$obj->method();"),
        Expression::Call(_)
    ));
    assert!(matches!(
        expression(&arena, "$obj?->prop;"),
        Expression::Access(_)
    ));
    assert!(matches!(
        expression(&arena, "Foo::BAR;"),
        Expression::Access(_)
    ));
    assert!(matches!(
        expression(&arena, "Foo::method();"),
        Expression::Call(_)
    ));
    assert!(matches!(
        expression(&arena, "self::$prop;"),
        Expression::Access(_)
    ));
    assert!(matches!(
        expression(&arena, "$array[0];"),
        Expression::ArrayAccess(_)
    ));
}

#[test]
fn chained_postfix() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "$repo->find($id)?->profile->name;"),
        Expression::Access(_)
    ));
}

#[test]
fn partial_application_and_first_class_callable() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "strlen(...);"),
        Expression::PartialApplication(_)
    ));
    assert!(matches!(
        expression(&arena, "add(?, 1);"),
        Expression::PartialApplication(_)
    ));
}

#[test]
fn instantiation_and_throw() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "new Foo(1);"),
        Expression::Instantiation(_)
    ));
    assert!(matches!(
        expression(&arena, "new self();"),
        Expression::Instantiation(_)
    ));
    assert!(matches!(
        expression(&arena, "throw $e;"),
        Expression::Throw(_)
    ));
}

#[test]
fn loop_jumps_are_expressions() {
    let arena = LocalArena::new();
    assert!(matches!(expression(&arena, "break;"), Expression::Break(_)));
    assert!(matches!(
        expression(&arena, "continue 2;"),
        Expression::Continue(_)
    ));
    assert!(matches!(
        expression(
            &arena,
            "match ($value) { true => break, false => continue };"
        ),
        Expression::Match(_)
    ));
}

#[test]
fn match_expression() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "match ($x) { 1 => 'a', $_ => 'b' };"),
        Expression::Match(_)
    ));
}

#[test]
fn match_has_recursive_checking_and_binding_patterns() {
    let arena = LocalArena::new();
    let Expression::Match(matching) = expression(
        &arena,
        "match ($value) { $circle @ Circle => $circle, ($n @ int, $_ @ string) => $n, vec[$first @ int, ...string] => $first, $_ => null };",
    ) else {
        panic!("expected a match");
    };

    assert!(matches!(
        matching.arms.as_slice()[0].pattern,
        Pattern::As(_)
    ));
    assert!(matches!(
        matching.arms.as_slice()[1].pattern,
        Pattern::Tuple(tuple)
            if tuple.elements.iter().all(|pattern| matches!(pattern, Pattern::As(_)))
    ));
    assert!(matches!(
        matching.arms.as_slice()[2].pattern,
        Pattern::Vec(vector)
            if matches!(vector.elements.as_slice()[0], Pattern::As(_))
                && vector.trailing.as_ref().is_some_and(|trailing| trailing.pattern.is_some())
    ));
    assert!(matches!(
        matching.arms.as_slice()[3].pattern,
        Pattern::Variable(_)
    ));
    assert!(matches!(
        error("match ($value) { $object->field @ Circle => 1 };"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("match ($value) { $items[0] @ Circle => 1 };"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn match_as_pattern_includes_the_union_on_its_right() {
    let arena = LocalArena::new();
    let Expression::Match(matching) = expression(
        &arena,
        "match ($value) { $bound @ 1 | 2 => $bound, $_ => null };",
    ) else {
        panic!("expected a match");
    };

    assert!(matches!(
        matching.arms.as_slice()[0].pattern,
        Pattern::As(pattern)
            if matches!(pattern.left, Pattern::Variable(_))
                && matches!(pattern.right, Pattern::Union(_))
    ));
}

#[test]
fn an_argument_cannot_be_spread() {
    assert!(matches!(
        error("f(...$values);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("f(...$a, ...$b);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("f(1, ...$rest);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$o->m(...$values);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("C::m(...$values);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("new C(...$values);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("f(name: ...$values);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$f = find(...$given, ?);"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn vec_and_dict_literals_take_a_spread() {
    let arena = LocalArena::new();
    let Expression::Vec(vector) = expression(&arena, "vec[1, ...$o, 4];") else {
        panic!("expected a vec literal");
    };
    let elements: Vec<_> = vector.elements.iter().collect();
    assert_eq!(elements.len(), 3);
    assert!(!elements[0].is_spread());
    assert!(elements[1].is_spread());
    assert!(!elements[2].is_spread());

    let Expression::Dict(dictionary) = expression(&arena, "dict['a' => 1, ...$o];") else {
        panic!("expected a dict literal");
    };
    let entries: Vec<_> = dictionary.entries.iter().collect();
    assert_eq!(entries.len(), 2);
    assert!(!entries[0].is_spread());
    assert!(entries[1].is_spread());
}

#[test]
fn a_rest_inside_parentheses_parses() {
    let arena = LocalArena::new();
    let Expression::Assignment(assignment) = expression(&arena, "($a, $b, ...$r) = $v;") else {
        panic!("expected an assignment");
    };
    let AssignmentTarget::Tuple(pattern) = &assignment.target else {
        panic!("expected a destructuring pattern");
    };
    let targets: Vec<_> = pattern.targets.iter().collect();
    assert_eq!(targets.len(), 3);
    assert!(!targets[0].is_rest());
    assert!(targets[2].is_rest());

    let Expression::Assignment(bare) = expression(&arena, "($a, ...) = $v;") else {
        panic!("expected an assignment");
    };
    let AssignmentTarget::Tuple(pattern) = &bare.target else {
        panic!("expected a destructuring pattern");
    };
    let Some(DestructureTarget::Rest(rest)) = pattern.targets.iter().last() else {
        panic!("expected a trailing rest");
    };
    assert!(rest.target.is_none());

    assert!(matches!(
        error("$t = (...$rest,);"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn a_destructuring_default_is_not_an_inner_assignment() {
    let arena = LocalArena::new();
    let Expression::Assignment(assignment) =
        expression(&arena, "($given, $family = '(none)') = $parts;")
    else {
        panic!("expected an assignment");
    };
    let AssignmentTarget::Tuple(pattern) = &assignment.target else {
        panic!("expected a destructuring pattern");
    };
    assert!(matches!(
        pattern.targets.as_slice()[0],
        DestructureTarget::Target(_)
    ));
    assert!(matches!(
        pattern.targets.as_slice()[1],
        DestructureTarget::Default(_)
    ));
}

#[test]
fn a_dict_spread_carries_no_key() {
    assert!(matches!(
        error("$d = dict[...$o => 1];"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn a_first_class_callable_is_not_a_spread() {
    let arena = LocalArena::new();
    let Expression::PartialApplication(application) = expression(&arena, "f(...);") else {
        panic!("expected a partial application");
    };
    assert!(application.is_first_class_callable());

    let Expression::PartialApplication(bound) = expression(&arena, "f(1, 2, ...);") else {
        panic!("expected a partial application");
    };
    assert!(!bound.is_first_class_callable());
}

#[test]
fn vec_and_dict_literals() {
    let arena = LocalArena::new();
    let Expression::Vec(vec) = expression(&arena, "vec[1, 2, 3];") else {
        panic!("expected a vec literal");
    };
    assert_eq!(vec.elements.len(), 3);

    let Expression::Dict(dict) = expression(&arena, "dict[1 => 'a', $k => 'b'];") else {
        panic!("expected a dict literal");
    };
    assert_eq!(dict.entries.len(), 2);

    assert!(matches!(
        error("$x = vec[1 => 2];"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = dict[1, 2];"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = vec[...$values; 2];"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = vec[1; 2,];"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn tuple_expressions() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "(1);"),
        Expression::Parenthesized(_)
    ));

    let Expression::Tuple(one) = expression(&arena, "(1,);") else {
        panic!("expected a single-element tuple");
    };
    assert_eq!(one.elements.len(), 1);

    let Expression::Tuple(many) = expression(&arena, "(1, 2, 3);") else {
        panic!("expected a tuple");
    };
    assert_eq!(many.elements.len(), 3);

    assert!(matches!(error("$x = ();"), ParseError::UnexpectedToken(..)));
}

#[test]
fn vec_and_dict_are_collection_literals_before_a_bracket() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "vec[1, 2];"),
        Expression::Vec(_)
    ));
    assert!(matches!(
        expression(&arena, "$x = vec;"),
        Expression::Assignment(_)
    ));
    assert!(matches!(
        expression(&arena, "vec($x);"),
        Expression::Call(_)
    ));
}

#[test]
fn language_constructs() {
    let arena = LocalArena::new();

    let Expression::Construct(Construct::Length(_)) = expression(&arena, "length!($v);") else {
        panic!("expected a length construct");
    };
    let Expression::Construct(Construct::Remove(remove)) = expression(&arena, "remove!($v, 0);")
    else {
        panic!("expected a remove construct");
    };
    assert!(matches!(remove.key, Expression::Literal(_)));
    let Expression::Construct(Construct::SwapRemove(remove)) =
        expression(&arena, "swap_remove!($v, 0);")
    else {
        panic!("expected a swap_remove construct");
    };
    assert!(matches!(remove.index, Expression::Literal(_)));

    let Expression::Construct(Construct::Clone(clone)) =
        expression(&arena, "clone!($o, name: 'x', age: 1);")
    else {
        panic!("expected a clone construct");
    };
    assert_eq!(clone.fields.len(), 2);
    assert_eq!(clone.fields[0].name.value, "name");

    let Expression::Construct(Construct::Assert(assert)) =
        expression(&arena, "assert!($cond, 'boom');")
    else {
        panic!("expected an assert construct");
    };
    assert!(assert.message.is_some());
    let Expression::Construct(Construct::Exit(exit)) = expression(&arena, "exit!();") else {
        panic!("expected an exit construct");
    };
    assert!(exit.code.is_none());

    let Expression::Construct(Construct::Panic(panic)) =
        expression(&arena, "panic!(\"impossible\",);")
    else {
        panic!("expected a panic construct");
    };
    assert_eq!(panic.message.raw, "\"impossible\"");
    assert_eq!(panic.message.value, b"impossible");
    assert!(panic.trailing_comma.is_some());

    let Expression::Construct(Construct::WriteLine(write)) =
        expression(&arena, "write_line!('a', $b, 'c');")
    else {
        panic!("expected a write_line construct");
    };
    assert_eq!(write.arguments.len(), 3);

    let Expression::Construct(Construct::Drop(drop)) =
        expression(&arena, "drop!($first, $second,);")
    else {
        panic!("expected a drop construct");
    };
    assert_eq!(drop.variables.len(), 2);

    let Expression::Construct(Construct::Discard(discard)) =
        expression(&arena, "discard!(compute());")
    else {
        panic!("expected a discard construct");
    };
    assert!(matches!(discard.value, Expression::Call(_)));
}

#[test]
fn constructs_fail_fast_in_the_parser() {
    assert!(matches!(
        error("nope!($x);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = remove!($v);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = swap_remove!($v);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = length!($v, $w);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = contains!($v);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$x = contains_key!($v, 0, 1);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(error("drop!();"), ParseError::UnexpectedToken(..)));
    assert!(matches!(
        error("discard!();"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("discard!($first, $second);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("drop!($object->property);"),
        ParseError::UnexpectedToken(..)
    ));
    for invalid in [
        "panic!();",
        "panic!($message);",
        "panic!('one', 'two');",
        "panic!(\"{$message}\");",
    ] {
        assert!(matches!(error(invalid), ParseError::UnexpectedToken(..)));
    }
}

#[test]
fn file_and_directory_constructs() {
    let arena = LocalArena::new();

    let Expression::Assignment(assignment) = expression(&arena, "$p = file!();") else {
        panic!("expected an assignment");
    };
    assert!(matches!(
        assignment.value,
        Expression::Construct(Construct::File(_))
    ));

    let Expression::Assignment(assignment) = expression(&arena, "$d = directory!();") else {
        panic!("expected an assignment");
    };
    assert!(matches!(
        assignment.value,
        Expression::Construct(Construct::Directory(_))
    ));

    assert!(matches!(
        error("file!(1);"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("directory!(,);"),
        ParseError::UnexpectedToken(..)
    ));

    assert!(matches!(
        expression(&arena, "$x = file;"),
        Expression::Assignment(_)
    ));
    assert!(matches!(
        expression(&arena, "file(1);"),
        Expression::Call(_)
    ));
}

#[test]
fn embed_construct() {
    let arena = LocalArena::new();

    let Expression::Assignment(assignment) =
        expression(&arena, "$template = embed!(\"./template.tpl\",);")
    else {
        panic!("expected an assignment");
    };
    let Expression::Construct(Construct::Embed(embed)) = assignment.value else {
        panic!("expected an embed construct");
    };
    assert_eq!(embed.path.raw, "\"./template.tpl\"");
    assert_eq!(embed.path.value, b"./template.tpl");
    assert!(embed.trailing_comma.is_some());

    for invalid in [
        "embed!();",
        "embed!($path);",
        "embed!('./one', './two');",
        "embed!(\"{$path}\");",
    ] {
        assert!(matches!(error(invalid), ParseError::UnexpectedToken(..)));
    }

    assert!(matches!(
        expression(&arena, "embed('x');"),
        Expression::Call(_)
    ));
    assert!(matches!(
        expression(&arena, "panic('x');"),
        Expression::Call(_)
    ));
}

#[test]
fn construct_names_are_not_reserved() {
    let arena = LocalArena::new();
    assert!(matches!(
        expression(&arena, "length($v);"),
        Expression::Call(_)
    ));
    assert!(matches!(
        expression(&arena, "length!($v);"),
        Expression::Construct(_)
    ));
    assert!(matches!(
        error("type_of!($v);"),
        ParseError::UnexpectedToken(
            Expected::Description("a known language construct"),
            TokenKind::Identifier,
            _
        )
    ));
}
