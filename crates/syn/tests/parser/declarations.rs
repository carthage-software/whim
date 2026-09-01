use whim_syn::arena::LocalArena;

use whim_syn::cst::atom::Identifier;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::declaration::NamespaceBody;
use whim_syn::cst::declaration::UseItems;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::function::ShortClosureBody;
use whim_syn::cst::statement::Statement;
use whim_syn::error::ParseError;

use crate::error;
use crate::expression;
use crate::program;
use crate::statement;

#[test]
fn function_declaration_with_typed_params_and_return_type() {
    let arena = LocalArena::new();
    let Statement::Function(function) = statement(
        &arena,
        "function add(int $a, int $b): int { return $a + $b; }",
    ) else {
        panic!("expected a function declaration");
    };

    assert_eq!(function.name.value, "add");
    assert_eq!(function.parameter_list.parameters.len(), 2);
    assert!(function.return_type.is_some());
    let first = function
        .parameter_list
        .parameters
        .first()
        .expect("a parameter");
    assert!(first.r#type.is_some());
}

#[test]
fn underscore_is_reserved_for_every_declaration_name() {
    for source in [
        "function _(): void {}",
        "function f<_>(): void {}",
        "class _ {}",
        "interface _ {}",
        "enum _ {}",
        "type _ = int;",
        "newtype _ = int;",
        "const _ = 1;",
        "namespace _;",
        "use Foo as _;",
        "class C { public const int _ = 1; public function _(): void {} }",
    ] {
        assert!(
            matches!(error(source), ParseError::ReservedIdentifier(_)),
            "expected `_` to be rejected in {source}"
        );
    }
}

#[test]
fn function_with_default_parameters() {
    let arena = LocalArena::new();
    let Statement::Function(function) =
        statement(&arena, "function f(string $name, int $count = 0) { }")
    else {
        panic!("expected a function declaration");
    };

    let parameters: Vec<_> = function.parameter_list.parameters.iter().collect();
    assert_eq!(parameters.len(), 2);
    assert!(parameters[1].default.is_some());
}

#[test]
fn a_parameter_cannot_be_variadic() {
    assert!(matches!(
        error("function f(mixed ...$rest) { }"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("function f(...$rest) { }"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("class C { public function m(int $a, string ...$rest): void {} }"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$f = function (mixed ...$rest) { };"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("$f = fn(mixed ...$rest) => 1;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn a_function_type_parameter_cannot_be_variadic() {
    assert!(matches!(
        error("type Handler = fn(int, ...bool): void;"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("type Handler = fn(...mixed): mixed;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn class_with_method_typed_property_and_constant() {
    let arena = LocalArena::new();
    let source = "final class Point extends Base implements Comparable {
        public const int DIMENSIONS = 2;
        private int $x = 0;
        public function moveBy(int $dx): void { }
    }";
    let Statement::Class(class) = statement(&arena, source) else {
        panic!("expected a class declaration");
    };

    assert_eq!(class.name.value, "Point");
    assert!(class.is_final());
    assert!(class.extends.is_some());
    assert!(class.implements.is_some());
    assert_eq!(class.members.len(), 3);
    assert!(class.members[0].is_constant());
    assert!(class.members[1].is_property());
    assert!(matches!(class.members[2], ClassLikeMember::Method(_)));

    let ClassLikeMember::Property(property) = &class.members[1] else {
        panic!("expected a property");
    };
    assert!(property.is_typed());
    assert!(property.has_default());
}

#[test]
fn constructor_property_promotion() {
    let arena = LocalArena::new();
    let source = "class Point {
        public function __construct(private int $x, private int $y) { }
    }";
    let Statement::Class(class) = statement(&arena, source) else {
        panic!("expected a class declaration");
    };

    let ClassLikeMember::Method(constructor) = &class.members[0] else {
        panic!("expected a method");
    };
    let promoted: Vec<_> = constructor.parameter_list.parameters.iter().collect();
    assert!(promoted[0].is_promoted_property());
    assert!(promoted[1].is_promoted_property());
}

#[test]
fn interface_with_abstract_method() {
    let arena = LocalArena::new();
    let source = "interface Comparable extends Stringable {
        public function compareTo(self $other): int;
    }";
    let Statement::Interface(interface) = statement(&arena, source) else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.name.value, "Comparable");
    assert!(interface.extends.is_some());
    let ClassLikeMember::Method(method) = &interface.members[0] else {
        panic!("expected a method");
    };
    assert!(matches!(method.body, MethodBody::Abstract(_)));
}

#[test]
fn sealed_interface_permissions_follow_extends() {
    let arena = LocalArena::new();
    let source = "interface Narrow extends Result for Success, App\\Failure {}";
    let Statement::Interface(interface) = statement(&arena, source) else {
        panic!("expected an interface declaration");
    };

    let permissions = interface.permissions.as_ref().expect("a for clause");
    let names: Vec<_> = permissions.types.iter().map(Identifier::value).collect();
    assert_eq!(names, ["Success", "App\\Failure"]);
}

#[test]
fn backed_enum_with_cases() {
    let arena = LocalArena::new();
    let source = "enum Suit: string {
        case Hearts = 'H';
        case Spades = 'S';
    }";
    let Statement::Enum(r#enum) = statement(&arena, source) else {
        panic!("expected an enum declaration");
    };

    assert_eq!(r#enum.name.value, "Suit");
    assert!(r#enum.is_backed());
    assert_eq!(r#enum.members.len(), 2);
    let ClassLikeMember::EnumCase(case) = &r#enum.members[0] else {
        panic!("expected an enum case");
    };
    assert!(case.is_backed());
}

#[test]
fn unbacked_enum_with_cases() {
    let arena = LocalArena::new();
    let source = "enum Direction { case Up; case Down; }";
    let Statement::Enum(r#enum) = statement(&arena, source) else {
        panic!("expected an enum declaration");
    };

    assert!(!r#enum.is_backed());
    assert_eq!(r#enum.members.len(), 2);
}

#[test]
fn closure_with_use_clause() {
    let arena = LocalArena::new();
    let Expression::Closure(closure) = expression(
        &arena,
        "function ($x) use ($y, $z): int { return $x + $y; };",
    ) else {
        panic!("expected a closure");
    };

    assert_eq!(closure.parameter_list.parameters.len(), 1);
    let use_clause = closure.use_clause.as_ref().expect("a use clause");
    assert_eq!(use_clause.variables.len(), 2);
    assert!(closure.return_type.is_some());
}

#[test]
fn expression_bodied_short_closure() {
    let arena = LocalArena::new();
    let Expression::ShortClosure(closure) = expression(&arena, "fn ($x): int => $x * 2;") else {
        panic!("expected a short closure");
    };

    assert_eq!(closure.parameter_list.parameters.len(), 1);
    assert!(closure.return_type.is_some());
    assert!(matches!(
        closure.body,
        ShortClosureBody::Expression {
            expression: Expression::Binary(_),
            ..
        }
    ));
}

#[test]
fn block_bodied_short_closure() {
    let arena = LocalArena::new();
    let Expression::ShortClosure(closure) = expression(
        &arena,
        "fn(int $x): int { $result = $x * 2; return $result; };",
    ) else {
        panic!("expected a short closure");
    };

    let ShortClosureBody::Block(block) = &closure.body else {
        panic!("expected a block body");
    };
    assert_eq!(block.statements.len(), 2);
}

#[test]
fn attribute_on_a_class() {
    let arena = LocalArena::new();
    let source = "#[Entity, Table('users')] final class User { }";
    let Statement::Class(class) = statement(&arena, source) else {
        panic!("expected a class declaration");
    };

    assert_eq!(class.attribute_lists.len(), 1);
    assert!(class.is_final());
    let attributes = &class.attribute_lists[0].attributes;
    assert_eq!(attributes.len(), 2);
    let with_args = attributes.get(1).expect("a second attribute");
    assert!(with_args.argument_list.is_some());
}

#[test]
fn attributed_function_and_short_closure() {
    let arena = LocalArena::new();
    assert!(matches!(
        statement(&arena, "#[Pure] function f(): int { return 1; }"),
        Statement::Function(_)
    ));
    let Statement::Expression(statement) = statement(&arena, "#[A] fn () => 1;") else {
        panic!("expected an expression statement");
    };
    assert!(matches!(statement.expression, Expression::ShortClosure(_)));
}

#[test]
fn namespace_both_forms() {
    let arena = LocalArena::new();

    let Statement::Namespace(implicit) = statement(
        &arena,
        "namespace App\\Service;\nconst VERSION = 1;\nfn () => 1;",
    ) else {
        panic!("expected a namespace declaration");
    };
    assert_eq!(implicit.name.value(), "App\\Service");
    let NamespaceBody::Implicit(body) = &implicit.body else {
        panic!("expected an implicit namespace body");
    };
    assert_eq!(body.statements.len(), 2);

    let Statement::Namespace(braced) = statement(&arena, "namespace App { const X = 1; }") else {
        panic!("expected a namespace declaration");
    };
    assert!(matches!(braced.body, NamespaceBody::BraceDelimited(_)));
}

#[test]
fn a_namespace_needs_a_name_and_a_body_or_it_is_not_a_declaration() {
    let arena = LocalArena::new();

    assert!(matches!(
        statement(&arena, "namespace;"),
        Statement::Expression(_)
    ));
    assert!(matches!(
        error("namespace { const X = 1; }"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("namespace App + 1;"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn namespace_is_usable_as_an_ordinary_identifier() {
    let arena = LocalArena::new();

    assert!(matches!(
        statement(&arena, "function namespace(): int { return 1; }"),
        Statement::Function(_)
    ));
    assert!(matches!(
        statement(&arena, "namespace();"),
        Statement::Expression(_)
    ));

    let program = program(
        &arena,
        "namespace App;\nfunction namespace(): int { return 1; }\nnamespace();\n$x = 1;",
    );
    let Statement::Namespace(namespace) = &program.statements[0] else {
        panic!("expected a namespace declaration");
    };
    let NamespaceBody::Implicit(body) = &namespace.body else {
        panic!("expected an implicit namespace body");
    };
    assert_eq!(body.statements.len(), 3);
}

#[test]
fn use_statements() {
    let arena = LocalArena::new();

    let Statement::Use(simple) = statement(&arena, "use App\\Service\\Mailer;") else {
        panic!("expected a use statement");
    };
    assert!(matches!(simple.items, UseItems::Sequence(_)));

    let Statement::Use(aliased) = statement(&arena, "use App\\Service\\Mailer as Postman;") else {
        panic!("expected a use statement");
    };
    let UseItems::Sequence(sequence) = &aliased.items else {
        panic!("expected a use sequence");
    };
    assert!(sequence.items.first().expect("an item").alias.is_some());

    let Statement::Use(grouped) = statement(&arena, "use App\\Service\\{Mailer, Logger as Log};")
    else {
        panic!("expected a use statement");
    };
    let UseItems::List(list) = &grouped.items else {
        panic!("expected a grouped use list");
    };
    assert_eq!(list.items.len(), 2);

    let Statement::Use(sequence) = statement(&arena, "use App\\Mailer, App\\Logger as Log;") else {
        panic!("expected a use statement");
    };
    assert!(matches!(sequence.items, UseItems::Sequence(_)));
}

#[test]
fn use_is_kind_agnostic() {
    assert!(matches!(
        error("use function App\\Helpers\\format;"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("use const App\\VERSION;"),
        ParseError::UnexpectedToken(..)
    ));
    assert!(matches!(
        error("use App\\{Service\\Mailer, function Helpers\\format};"),
        ParseError::UnexpectedToken(..)
    ));
}

#[test]
fn top_level_constant() {
    let arena = LocalArena::new();
    let Statement::Constant(constant) = statement(&arena, "const MAX_RETRIES = 3;") else {
        panic!("expected a constant declaration");
    };
    assert_eq!(constant.name.value, "MAX_RETRIES");
}

#[test]
fn type_alias() {
    let arena = LocalArena::new();
    let Statement::TypeAlias(alias) = statement(&arena, "type Scalar = int|float|string|bool;")
    else {
        panic!("expected a type alias");
    };
    assert_eq!(alias.name.value, "Scalar");
    assert!(alias.aliased.is_union());
}

#[test]
fn generic_newtype() {
    let arena = LocalArena::new();
    let Statement::Newtype(newtype) =
        statement(&arena, "newtype Identifier<out T: int|string = int> = T;")
    else {
        panic!("expected a newtype declaration");
    };
    assert_eq!(newtype.name.value, "Identifier");
    assert_eq!(
        newtype
            .type_parameters
            .as_ref()
            .map(|parameters| parameters.parameters.len()),
        Some(1)
    );
}

#[test]
fn class_constant_with_and_without_type() {
    let arena = LocalArena::new();
    let source = "class C {
        const int TYPED = 1;
        const UNTYPED = 2;
    }";
    let Statement::Class(class) = statement(&arena, source) else {
        panic!("expected a class declaration");
    };

    let ClassLikeMember::Constant(typed) = &class.members[0] else {
        panic!("expected a constant");
    };
    assert!(typed.is_typed());

    let ClassLikeMember::Constant(untyped) = &class.members[1] else {
        panic!("expected a constant");
    };
    assert!(!untyped.is_typed());
}

#[test]
fn kitchen_sink_program() {
    let source = r"#!/usr/bin/env whim
namespace App\Service;

use App\Contract\Mailer;
use App\Helpers\format;

const VERSION = 2;

type Id = int|string;

#[Injectable]
final class Service extends Base implements Mailer {
    public const int RETRIES = 3;

    private readonly Logger $logger;
    public int $count = 0;

    public function __construct(private Config $config) {}

    public function send(Message $message, vec<string> $tags): bool {
        $this->count++;
        $result = $this->transport?->deliver($message) ?? false;

        return match ($result) {
            true => $result is bool && $this->count > 0,
            $_ => throw new SendFailure('unreachable'),
        };
    }

    abstract public function describe(): string;
}

interface Contract {
    public function run(): void;
}

enum Suit: string {
    case Hearts = 'H';
    case Spades = 'S';
}

function main(): int {
    $service = new Service(loadConfig());
    foreach (vec[1, 2, 3] as $index => $value) {
        $handler = fn($x) => $x * 2;
        $service->send($value as Message);
    }

    return 0;
}
";

    let arena = LocalArena::new();
    let program = program(&arena, source);

    assert_eq!(program.statements.len(), 1);
    let Statement::Namespace(namespace) = &program.statements[0] else {
        panic!("expected a namespace");
    };

    let body = namespace.statements();
    assert_eq!(body.len(), 8);
    assert!(matches!(body[4], Statement::Class(_)));
    assert!(matches!(body[5], Statement::Interface(_)));
    assert!(matches!(body[6], Statement::Enum(_)));
    assert!(matches!(body[7], Statement::Function(_)));
}
