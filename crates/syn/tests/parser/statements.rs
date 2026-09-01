use whim_syn::arena::LocalArena;

use whim_syn::cst::control_flow::ForeachTarget;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::r#type::Type;
use whim_syn::error::ParseError;

use crate::error;
use crate::program;
use crate::statement;

#[test]
fn control_flow_statements() {
    let arena = LocalArena::new();

    assert!(matches!(
        &program(&arena, "if ($c) { return 1; } else { return 2; }").statements[0],
        Statement::If(_)
    ));
    assert!(matches!(
        &program(&arena, "if ($a) { } else if ($b) { } else { }").statements[0],
        Statement::If(_)
    ));
    assert!(matches!(
        &program(&arena, "while ($c) { $i = $i + 1; }").statements[0],
        Statement::While(_)
    ));
    assert!(matches!(
        &program(&arena, "do { $x; } while ($c);").statements[0],
        Statement::DoWhile(_)
    ));
    assert!(matches!(
        &program(&arena, "for ($i = 0; $i < 10; $i++) { }").statements[0],
        Statement::For(_)
    ));
    assert!(matches!(
        &program(&arena, "foreach ($items as $item) { }").statements[0],
        Statement::Foreach(_)
    ));
    assert!(matches!(
        &program(&arena, "foreach ($map as $k => $v) { }").statements[0],
        Statement::Foreach(_)
    ));
    assert!(matches!(
        &program(&arena, "foreach ($pairs as ($a, $b)) { }").statements[0],
        Statement::Foreach(_)
    ));
    assert!(matches!(
        &program(&arena, "try { $a; } catch (Error $e) { } finally { }").statements[0],
        Statement::Try(_)
    ));
    let Statement::Using(using) = &program(
        &arena,
        "using ($resource = open(), ($a, $b) = pair(),) { use($resource); }",
    )
    .statements[0] else {
        panic!("expected a using statement");
    };
    assert_eq!(using.bindings.len(), 2);
    let Statement::Expression(statement) = &program(&arena, "return;").statements[0] else {
        panic!("expected an expression statement");
    };

    assert!(matches!(statement.expression, Expression::Return(_)));
    let Statement::Expression(statement) = &program(&arena, "break 2;").statements[0] else {
        panic!("expected an expression statement");
    };
    assert!(matches!(statement.expression, Expression::Break(_)));
    assert!(matches!(
        &program(&arena, ";").statements[0],
        Statement::Noop(_)
    ));
}

#[test]
fn final_local_binding_has_a_required_initializer() {
    let arena = LocalArena::new();
    let Statement::FinalLocal(binding) = statement(&arena, "final $answer = 42;") else {
        panic!("expected a final local binding");
    };

    assert_eq!(binding.variable.name, "$answer");
    assert!(matches!(binding.value, Expression::Literal(_)));
    assert!(matches!(
        error("final $answer;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn using_requires_a_binding_and_a_bind_target() {
    assert!(matches!(
        error("using () {}"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("using ($object->resource = open()) {}"),
        ParseError::InvalidBindTarget(_)
    ));
}

#[test]
fn optional_parentheses_around_conditions() {
    let arena = LocalArena::new();
    assert!(matches!(
        statement(&arena, "if ($x > 0) { }"),
        Statement::If(_)
    ));
    assert!(matches!(
        statement(&arena, "while ($ok) { }"),
        Statement::While(_)
    ));
    assert!(matches!(
        statement(&arena, "do { } while ($ok);"),
        Statement::DoWhile(_)
    ));
}

#[test]
fn control_flow_headers_require_parentheses() {
    assert!(matches!(
        error("if $x { }"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("if ($a) { } else if $b { }"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("while $x { }"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("do { } while $x;"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$m = match $x { default => 1 };"),
        ParseError::UnexpectedToken(..)
    ));

    let arena = LocalArena::new();
    assert!(matches!(statement(&arena, "if ($x) { }"), Statement::If(_)));
    assert!(matches!(
        statement(&arena, "while ($x) { }"),
        Statement::While(_)
    ));
    assert!(matches!(
        statement(&arena, "do { } while ($x);"),
        Statement::DoWhile(_)
    ));
}

#[test]
fn for_header_requires_parentheses() {
    let arena = LocalArena::new();
    let Statement::For(header) = statement(&arena, "for ($i = 0; $i < 10; $i++) { }") else {
        panic!("expected a for loop");
    };
    assert_eq!(header.initializations.len(), 1);
    assert_eq!(header.conditions.len(), 1);
    assert_eq!(header.increments.len(), 1);

    assert!(matches!(
        error("for $i = 0; $i < 10; $i++ { }"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn foreach_header_requires_parentheses() {
    let arena = LocalArena::new();

    let Statement::Foreach(bare) = statement(&arena, "foreach ($items as $x) { }") else {
        panic!("expected a foreach loop");
    };
    assert!(matches!(bare.target, ForeachTarget::Value(_)));

    let Statement::Foreach(subject) = statement(&arena, "foreach (($items) as $x) { }") else {
        panic!("expected a foreach loop");
    };
    assert!(matches!(subject.expression, Expression::Parenthesized(_)));

    let Statement::Foreach(tuple) = statement(&arena, "foreach ((1, 2) as $x) { }") else {
        panic!("expected a foreach loop");
    };
    assert!(matches!(tuple.expression, Expression::Tuple(_)));

    let Statement::Foreach(pair) = statement(&arena, "foreach ($dict as $k => $v) { }") else {
        panic!("expected a foreach loop");
    };
    assert!(matches!(pair.target, ForeachTarget::KeyValue(_)));

    assert!(matches!(
        error("foreach $items as $x { }"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn foreach_subject_mode_does_not_escape_into_nested_expressions() {
    let arena = LocalArena::new();

    for source in [
        "foreach (match ($k) { $y @ int => 1, $_ => 2 } as $item) {}",
        "foreach (match ($x) { $i @ int => $i, $_ => 0 } as $item) {}",
        "foreach (($x as vec<int>) as $item) {}",
    ] {
        let Statement::Foreach(_) = statement(&arena, source) else {
            panic!("expected a foreach loop for {source}");
        };
    }
}

#[test]
fn catch_header_requires_parentheses() {
    let arena = LocalArena::new();

    for source in [
        "try { } catch (Foo $e) { }",
        "try { } catch (Foo) { }",
        "try { } catch (Foo | Bar $e) { }",
        "try { } catch ((Foo) $e) { }",
        "try { } catch ((Foo)) { }",
    ] {
        assert!(
            matches!(statement(&arena, source), Statement::Try(_)),
            "{source}"
        );
    }

    let Statement::Try(bare) = statement(&arena, "try { } catch (Foo $e) { }") else {
        panic!("expected a try statement");
    };
    assert!(bare.catch_clauses[0].variable.is_some());
    assert!(matches!(bare.catch_clauses[0].r#type, Type::Named(_)));

    let Statement::Try(paren_type) = statement(&arena, "try { } catch ((Foo) $e) { }") else {
        panic!("expected a try statement");
    };
    assert!(matches!(
        paren_type.catch_clauses[0].r#type,
        Type::Parenthesized(_)
    ));

    assert!(matches!(
        error("try { } catch Foo $e { }"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn try_else_has_its_own_ordered_clause() {
    let arena = LocalArena::new();

    for source in [
        "try { } else { }",
        "try { } else { } finally { }",
        "try { } catch (Foo $e) { } else { }",
        "try { } catch (Foo $e) { } else { } finally { }",
    ] {
        let Statement::Try(statement) = statement(&arena, source) else {
            panic!("expected a try statement for {source}");
        };
        assert!(statement.else_clause.is_some(), "{source}");
    }

    for source in [
        "try { } else { } else { }",
        "try { } finally { } else { }",
        "try { } else { } catch (Foo) { }",
    ] {
        assert!(matches!(error(source), ParseError::UnexpectedToken(..)));
    }
}

#[test]
fn shebang_and_multiple_statements() {
    let arena = LocalArena::new();
    let program = program(&arena, "#!/usr/bin/env whim\n$a = 1;\n$b = 2;\n");
    assert_eq!(program.statements.len(), 2);
    assert!(!program.trivia.is_empty());
}
