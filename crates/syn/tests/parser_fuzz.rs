//! Fuzzes parser input and nesting. Parsing must return a result without a
//! panic or stack overflow.

use std::thread::Builder;

use proptest::collection::vec as vec_strategy;
use proptest::prelude::*;
use whim_syn::arena::LocalArena;
use whim_syn::parser;

const FUZZ_STACK_BYTES: usize = 32 * 1024 * 1024;

fn parse(source: &str) {
    let source = source.to_owned();
    let handle = Builder::new()
        .stack_size(FUZZ_STACK_BYTES)
        .spawn(move || {
            let arena = LocalArena::new();
            let _ = parser::parse(&arena, &source);
        })
        .expect("the fuzz parse thread spawns");

    handle.join().expect("the parser must not panic");
}

const SEEDS: &[&str] = &[
    "function add(int $x, int $y): int { return $x + $y; }",
    "$a = 1; $b = 2; write_line!($a + $b * 3);",
    "class Counter { public int $n = 0; function step(): int { return $this->n; } }",
    "if ($x > 0) { write_line!('positive'); } else { write_line!('other'); }",
    "$total = 0; while ($total < 10) { $total = $total + 1; }",
    "$label = match ($code) { 1 => 'one', 2 => 'two', $_ => 'many' };",
    "try { throw new Failure('x', 0); } catch (Failure $e) { } finally { write!('done'); }",
    "enum Suit { case Hearts; case Spades; }",
    "$pair = (1, 'two'); $vector = vec[1, 2, 3]; $map = dict['k' => 1];",
    "$double = function (int $n): int { return $n * 2; };",
];

const KEYWORDS: &[&str] = &[
    "public",
    "private",
    "protected",
    "static",
    "final",
    "abstract",
    "readonly",
    "class",
    "interface",
    "enum",
    "function",
    "const",
    "use",
    "namespace",
    "type",
    "if",
    "else",
    "while",
    "for",
    "foreach",
    "return",
    "match",
    "try",
    "catch",
    "new",
    "extends",
    "implements",
];

const LITERAL_FRAGMENTS: &[&str] = &[
    "0x", "0X", "0b", "0B", "0o", "0O", "0xG", "0b2", "0o8", "0x_", "1e", "1.", ".5e", "0755",
    "0xff", "0b101", "0o17", "1_000", "1.5e-3", "; ", "= ", "$a ", "+ ", "( ", ") ",
];

fn mutated_seed() -> impl Strategy<Value = String> {
    (
        0..SEEDS.len(),
        vec_strategy((any::<usize>(), any::<u8>(), 0u8..3), 0..10),
    )
        .prop_map(|(seed_index, edits)| {
            let mut bytes = SEEDS[seed_index].as_bytes().to_vec();
            for (position, byte, operation) in edits {
                if bytes.is_empty() {
                    break;
                }
                let position = position % bytes.len();
                match operation {
                    0 => bytes[position] = byte,
                    1 => bytes.insert(position, byte),
                    _ => {
                        bytes.remove(position);
                    }
                }
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
}

fn long_modifier_run() -> impl Strategy<Value = String> {
    (vec_strategy(0usize..7, 0..40), any::<bool>(), 0usize..3).prop_map(
        |(modifiers, with_body, tail)| {
            let mut source = String::new();
            for index in modifiers {
                source.push_str(KEYWORDS[index]);
                source.push(' ');
            }
            source.push_str(["class", "interface", "enum"][tail]);
            if with_body {
                source.push_str(" Name {}");
            }
            source
        },
    )
}

fn keyword_storm() -> impl Strategy<Value = String> {
    vec_strategy(0..KEYWORDS.len(), 0..80).prop_map(|indices| {
        indices
            .into_iter()
            .map(|index| KEYWORDS[index])
            .collect::<Vec<_>>()
            .join(" ")
    })
}

fn deeply_nested_type() -> impl Strategy<Value = String> {
    (0usize..1600, any::<bool>()).prop_map(|(depth, dict)| {
        let mut source = String::from("function deep(): ");
        for _ in 0..depth {
            source.push_str(if dict { "dict<int, " } else { "vec<" });
        }
        source.push_str("int");
        for _ in 0..depth {
            source.push('>');
        }
        source.push_str(" { return $x; }");
        source
    })
}

fn deeply_nested_expression() -> impl Strategy<Value = String> {
    (0usize..1600, 0usize..3).prop_map(|(depth, shape)| {
        let (open, close): (&str, &str) = match shape {
            0 => ("(", ")"),
            1 => ("vec[", "]"),
            _ => ("-", ""),
        };
        let mut source = String::from("$x = ");
        for _ in 0..depth {
            source.push_str(open);
        }
        source.push('1');
        for _ in 0..depth {
            source.push_str(close);
        }
        source.push(';');
        source
    })
}

fn malformed_literal_soup() -> impl Strategy<Value = String> {
    vec_strategy(0..LITERAL_FRAGMENTS.len(), 0..40).prop_map(|indices| {
        indices
            .into_iter()
            .map(|index| LITERAL_FRAGMENTS[index])
            .collect::<String>()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_input_never_panics(source in ".*") {
        parse(&source);
    }

    #[test]
    fn token_soup_never_panics(
        source in "[A-Za-z0-9_$ \t\r\n(){}\\[\\]<>;:=+\\-*/%!&|^~.,?'\"\\\\@#]{0,160}"
    ) {
        parse(&source);
    }

    #[test]
    fn mutated_valid_programs_never_panic(source in mutated_seed()) {
        parse(&source);
    }

    #[test]
    fn long_modifier_runs_never_panic(source in long_modifier_run()) {
        parse(&source);
    }

    #[test]
    fn keyword_storms_never_panic(source in keyword_storm()) {
        parse(&source);
    }

    #[test]
    fn malformed_literal_soup_never_panics(source in malformed_literal_soup()) {
        parse(&source);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn deeply_nested_types_never_overflow(source in deeply_nested_type()) {
        parse(&source);
    }

    #[test]
    fn deeply_nested_expressions_never_overflow(source in deeply_nested_expression()) {
        parse(&source);
    }
}
