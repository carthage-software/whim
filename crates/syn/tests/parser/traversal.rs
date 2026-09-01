use whim_syn::arena::LocalArena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;
use whim_syn::parser::parse;

struct Collector {
    kinds: Vec<NodeKind>,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for Collector {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        self.kinds.push(node.kind());
        Flow::Descend
    }
}

fn kinds(source: &str) -> Vec<NodeKind> {
    let arena = LocalArena::new();
    let program = parse(&arena, source).expect("parse");
    let mut collector = Collector { kinds: Vec::new() };
    walk(Node::Program(program), &mut collector);

    collector.kinds
}

#[test]
fn walks_a_whole_program() {
    let ks = kinds("function f(int $n): int {\n  $sum = $n + 1;\n  return $sum;\n}");

    assert_eq!(ks[0], NodeKind::Program);
    assert!(ks.contains(&NodeKind::Function));
    assert!(ks.contains(&NodeKind::Parameter));
    assert!(ks.contains(&NodeKind::Type));
    assert!(ks.contains(&NodeKind::ReturnType));
    assert!(ks.contains(&NodeKind::Block));
    assert!(ks.contains(&NodeKind::Assignment));
    assert!(ks.contains(&NodeKind::Binary));
    assert!(ks.contains(&NodeKind::Return));
    assert!(ks.contains(&NodeKind::Variable));
}

#[test]
fn descends_into_nested_expressions() {
    let ks = kinds("$x = foo($a->b, dict[1 => $c], vec[2, 3]);");
    assert!(ks.contains(&NodeKind::Call));
    assert!(ks.contains(&NodeKind::PropertyAccess));
    assert!(ks.contains(&NodeKind::DictExpression));
    assert!(ks.contains(&NodeKind::DictEntry));
    assert!(ks.contains(&NodeKind::VecExpression));
    assert!(ks.contains(&NodeKind::NamedArgument) || ks.contains(&NodeKind::PositionalArgument));
}

#[test]
fn short_closure_bodies_are_nodes() {
    let expression = kinds("$f = fn(int $value): int => $value * 2;");
    assert!(expression.contains(&NodeKind::ShortClosure));
    assert!(expression.contains(&NodeKind::ShortClosureBody));
    assert!(expression.contains(&NodeKind::Binary));

    let block = kinds("$f = fn(int $value): int { return $value * 2; };");
    assert!(block.contains(&NodeKind::ShortClosure));
    assert!(block.contains(&NodeKind::ShortClosureBody));
    assert!(block.contains(&NodeKind::Block));
    assert!(block.contains(&NodeKind::Return));
}

#[test]
fn descends_into_dictionary_pattern_keys() {
    let ks = kinds("$x = match ($v) { dict[-0x10 => $_, 'name' => $_] => 1, $_ => 0 };");
    assert_eq!(
        ks.iter()
            .filter(|kind| **kind == NodeKind::DictPatternKey)
            .count(),
        2
    );
    assert!(ks.contains(&NodeKind::LiteralInteger));
    assert!(ks.contains(&NodeKind::LiteralString));
}

#[test]
fn skip_prunes_children() {
    struct SkipClasses {
        saw_class: bool,
        saw_method: bool,
    }
    impl<'ast, 'arena> Visitor<'ast, 'arena> for SkipClasses {
        fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
            match node {
                Node::Class(_) => {
                    self.saw_class = true;
                    Flow::Skip
                }
                Node::Method(_) => {
                    self.saw_method = true;
                    Flow::Descend
                }
                _ => Flow::Descend,
            }
        }
    }

    let arena = LocalArena::new();
    let program = parse(&arena, "class C { public function m() {} }").expect("parse");
    let mut visitor = SkipClasses {
        saw_class: false,
        saw_method: false,
    };
    walk(Node::Program(program), &mut visitor);

    assert!(visitor.saw_class, "the class itself is still entered");
    assert!(!visitor.saw_method, "skipping the class prunes its members");
}

#[test]
fn children_matches_visit_children() {
    let arena = LocalArena::new();
    let program = parse(&arena, "$a = 1 + 2;").expect("parse");
    let root = Node::Program(program);
    let children = root.children();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].kind(), NodeKind::Statement);
}

#[test]
fn walks_into_operators_modifiers_and_leaves() {
    let ks = kinds("class C { public static Num $n = 1 <=> 2; }");
    assert!(ks.contains(&NodeKind::Modifier), "modifiers are nodes");
    assert!(
        ks.contains(&NodeKind::BinaryOperator),
        "operators are nodes"
    );
    assert!(
        ks.contains(&NodeKind::LiteralInteger),
        "literal kinds are nodes"
    );
    assert!(ks.contains(&NodeKind::Identifier));
    assert!(ks.contains(&NodeKind::NamedType), "named types are nodes");

    let modifiers = kinds("final class C { protected static function m() {} }")
        .iter()
        .filter(|k| **k == NodeKind::Modifier)
        .count();
    assert_eq!(modifiers, 3, "final, protected, static");
}

#[test]
fn walks_into_every_keyword() {
    struct KeywordCollector<'arena> {
        values: Vec<&'arena str>,
    }
    impl<'ast, 'arena> Visitor<'ast, 'arena> for KeywordCollector<'arena> {
        fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
            if let Node::Keyword(keyword) = node {
                self.values.push(keyword.value);
            }
            Flow::Descend
        }
    }

    let source = "\
namespace App;
use Foo\\Bar as Baz;
const C = 1;
type T = int;
type U = classname<D>;
function f(): int { return 1; }
final class D extends E implements F {
const int K = 1;
public function m(): void {
    $g = function () use ($x) { return $x; };
    $h = fn () => 1;
    if ($x) { } else { }
    while ($x) { }
    do { } while ($x);
    for (;;) { break; continue; }
    foreach ($x as $y) { }
    try { } catch (Ex $e) { } else { } finally { }
    $m = match ($x) { $_ => 1 };
    $n = new D();
    $v = vec[1];
    $d = dict['a' => 1];
    throw $e;
}
}
enum G { case A; }";

    let arena = LocalArena::new();
    let program = parse(&arena, source).expect("parse");
    let mut collector = KeywordCollector { values: Vec::new() };
    walk(Node::Program(program), &mut collector);
    let seen = collector.values;

    for keyword in [
        "namespace",
        "use",
        "as",
        "const",
        "type",
        "classname",
        "function",
        "return",
        "class",
        "extends",
        "implements",
        "case",
        "fn",
        "if",
        "else",
        "while",
        "do",
        "for",
        "break",
        "continue",
        "foreach",
        "try",
        "catch",
        "finally",
        "match",
        "new",
        "vec",
        "dict",
        "throw",
        "enum",
    ] {
        assert!(
            seen.contains(&keyword),
            "the walk reaches the `{keyword}` keyword"
        );
    }
}

#[test]
fn walks_into_every_construct_node() {
    let ks = kinds(
        "$a = remove!($v, clone!($o, k: 1)); $b = swap_remove!($v, 0); assert!($c, 'm'); debug!($x); discard!(work()); embed!('data.txt'); panic!('stop');",
    );
    assert!(
        ks.contains(&NodeKind::Construct),
        "the Construct dispatch node"
    );
    assert!(ks.contains(&NodeKind::RemoveConstruct));
    assert!(ks.contains(&NodeKind::SwapRemoveConstruct));
    assert!(ks.contains(&NodeKind::CloneConstruct));
    assert!(
        ks.contains(&NodeKind::CloneField),
        "a clone field is a node"
    );
    assert!(ks.contains(&NodeKind::AssertConstruct));
    assert!(
        ks.contains(&NodeKind::AssertMessage),
        "an assert message is a node"
    );
    assert!(ks.contains(&NodeKind::DebugConstruct));
    assert!(ks.contains(&NodeKind::DiscardConstruct));
    assert!(ks.contains(&NodeKind::EmbedConstruct));
    assert!(ks.contains(&NodeKind::PanicConstruct));
    assert!(
        ks.contains(&NodeKind::ConstructArgument),
        "a variadic construct argument is a node"
    );
}
