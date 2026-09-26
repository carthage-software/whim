use super::compile;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;

#[test]
fn specialized_collection_reads_remain_inlinable() {
    let unit = compile(
        r"
        function vector(vec<int> $values, int $index): int { return $values[$index]; }
        function strings(vec<string> $values, int $index): string { return $values[$index]; }
        function integers(dict<int, int> $values, int $key): int { return $values[$key]; }
        function unsigned(dict<uint, int> $values, uint $key): int { return $values[$key]; }
        function keys(dict<string, int> $values, string $key): int { return $values[$key]; }
        function first((int, int) $pair): int { return $pair[0]; }
        function fallback(dict<string, int> $values, string $key): int { return $values[$key] ?? 0; }
        function call_vector(vec<int> $values, int $index): int { return vector($values, $index); }
        function call_strings(vec<string> $values, int $index): string { return strings($values, $index); }
        function call_integers(dict<int, int> $values, int $key): int { return integers($values, $key); }
        function call_unsigned(dict<uint, int> $values, uint $key): int { return unsigned($values, $key); }
        function call_keys(dict<string, int> $values, string $key): int { return keys($values, $key); }
        function call_first(): int { return first((10, 20)); }
        function call_fallback(dict<string, int> $values, string $key): int { return fallback($values, $key); }
        function call_closure(vec<int> $values, int $index): int {
            return (fn(vec<int> $items, int $offset): int => $items[$offset])($values, $index);
        }
        final class Reader {
            public function at(vec<int> $values, int $index): int { return $values[$index]; }
        }
        function call_method(Reader $reader, vec<int> $values, int $index): int {
            return $reader->at($values, $index);
        }
        ",
        OptimizationConfiguration::default(),
    );
    for function in &unit.functions {
        if !function.name.as_bytes().starts_with(b"call_") {
            continue;
        }
        assert!(
            !function.chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallNamed { .. }
                    | Instruction::CallNamedUnchecked { .. }
                    | Instruction::CallNamedDirect { .. }
                    | Instruction::CallValue { .. }
                    | Instruction::CallValueUnchecked { .. }
                    | Instruction::CallMethod { .. }
                    | Instruction::CallMethodUnchecked { .. }
                    | Instruction::CallMethodDirect { .. }
            )),
            "{:?}: {:?}",
            function.name,
            function.chunk.code,
        );
    }
    verify_unit(&unit).unwrap();
}
