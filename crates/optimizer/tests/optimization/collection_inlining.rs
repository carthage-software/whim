use super::compile;
use whim_bytecode::chunk::descriptors::IcDescriptor;
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

#[test]
fn nested_collection_arguments_include_every_written_value() {
    for (checked, body) in [
        (
            false,
            "$rows = vec[]; for ($i = 0; $i < 4; $i++) { $row = dict['n' => $i]; if ($flag) { $row['extra'] = null; } $rows[] = $row; } foreach ($rows as $row) { $sum += read($row); }",
        ),
        (
            false,
            "$rows = dict[1u => dict['n' => 1]]; $rows[2u] = dict['n' => 2]; foreach ($rows as $row) { $sum += read($row); }",
        ),
        (
            false,
            "$rows = vec[dict['n' => 1]]; $row = $rows[0]; $sum += read($row);",
        ),
        (
            false,
            "$row = dict['n' => 1]; $rows = vec[$row]; escape($row); foreach ($rows as $row) { $sum += read($row); }",
        ),
        (
            true,
            "$rows = vec[dict['n' => 1]]; $rows[] = dict['n' => 'wrong']; foreach ($rows as $row) { $sum += read($row); }",
        ),
        (
            true,
            "$rows = vec[dict['n' => 1], ...$unknown]; foreach ($rows as $row) { $sum += read($row); }",
        ),
        (
            true,
            "$rows = vec[dict['n' => 1]]; for ($i = 0; $i < 2; $i++) { foreach ($rows as $row) { $sum += read($row); } $rows[0] = dict['n' => 'wrong']; }",
        ),
        (
            true,
            "$rows = vec[dict['n' => 1]]; $copy = $rows; $copy[0] = dict['n' => 'wrong']; foreach ($copy as $row) { $sum += read($row); }",
        ),
        (
            true,
            "$rows = vec[dict['n' => 1]]; $row = $rows[0]; $row['n'] = 'wrong'; $sum += read($row);",
        ),
        (
            true,
            "$rows = vec[dict['n' => 1]]; if ($flag) { $rows = $unknown; } foreach ($rows as $row) { $sum += read($row); }",
        ),
        (
            true,
            "$row = dict['n' => 1]; if ($flag) { $row = dict['n' => 'wrong']; } $rows = vec[$row]; foreach ($rows as $row) { $sum += read($row); }",
        ),
    ] {
        let source = format!(
            r"
            #[Whim\Marker\NeverInline]
            function read(dict<string, int|null> $row): int {{ return $row['n'] ?? 0; }}
            #[Whim\Marker\NeverInline]
            function escape(dict<string, int> $row): void {{ $row['n'] = 'changed'; }}
            function run(bool $flag, vec<dict> $unknown): int {{ $sum = 0; {body} return $sum; }}
            "
        );

        let unit = compile(&source, OptimizationConfiguration::default());
        let function = unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == b"run")
            .unwrap();

        let calls = function
            .chunk
            .code
            .iter()
            .filter_map(|instruction| {
                let (cache, checked) = match instruction {
                    Instruction::CallNamed { cache, .. } => (cache, true),
                    Instruction::CallNamedUnchecked { cache, .. }
                    | Instruction::CallNamedDirect { cache, .. } => (cache, false),
                    _ => return None,
                };
                let IcDescriptor::Member { name, .. } =
                    &function.chunk.ic_descriptors[usize::from(cache.index())]
                else {
                    return None;
                };
                (name.as_bytes() == b"read").then_some(checked)
            })
            .collect::<Vec<_>>();

        assert_eq!(calls, [checked], "{body}: {:?}", function.chunk.code);
        verify_unit(&unit).unwrap();
    }
}

#[test]
fn coalescing_getters_inline_for_branch_built_nested_collections() {
    let unit = compile(
        r"
        function score(dict<string, int|null> $row, dict<string, int> $defaults): int {
            return ($row['primary'] ?? $row['secondary'] ?? $defaults['fallback']) + ($row['bonus'] ?? 0);
        }
        function run(bool $flag): int {
            $rows = vec[];
            for ($i = 0; $i < 4; $i++) {
                $row = dict['primary' => $i];
                if ($flag) { $row['bonus'] = null; }
                $rows[] = $row;
            }
            $defaults = dict['fallback' => 23];
            $sum = 0;
            foreach ($rows as $row) { $sum += score($row, $defaults); }
            return $sum;
        }
        ",
        OptimizationConfiguration::default(),
    );

    let function = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"run")
        .unwrap();
    assert!(
        !function.chunk.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::CallNamed { .. }
                | Instruction::CallNamedUnchecked { .. }
                | Instruction::CallNamedDirect { .. }
        )),
        "{:?}",
        function.chunk.code
    );

    verify_unit(&unit).unwrap();
}
