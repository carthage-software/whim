use std::path::Path;
use std::rc::Rc;

use whim_bytecode::instruction::Instruction;
use whim_compiler::CompileConfiguration;
use whim_compiler::compile_with_configuration;
use whim_optimizer::OptimizationConfiguration;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::engine::Engine;
use crate::engine::EngineConfiguration;
use crate::symbols::CallableOptimization;

#[test]
fn array_type_checks_ignore_rolled_back_aliases() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(
            r"
use Whim\Marker\NeverInline;
final class Stored { public static mixed $value = vec[1, 2]; }
#[NeverInline]
function matches(mixed $value): bool { return $value is vec<Future>; }
#[NeverInline]
function stage(): string { assert!(matches(Stored::$value)); return 'bad'; }
",
            Path::new("/array-cache-before-rollback.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        let result = engine.run_source(
            r"
type Future = int;
final class Broken { public static int $value = stage(); }
",
            Path::new("/array-cache-failed-declaration.whim"),
        );
        assert_ne!(result.exit_code(), 0);
        let result = engine.run_source(
            r"
type Future = string;
assert!(!matches(Stored::$value));
assert!(matches(vec['first', 'second']));
assert!(!matches(Stored::$value));
",
            Path::new("/array-cache-after-rollback.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn pending_optimization_ignores_rolled_back_aliases() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        assert_eq!(
            engine
                .run_source("", Path::new("/pending-bootstrap.whim"))
                .exit_code(),
            0
        );
        let arena = LocalArena::new();
        let program = parse(
            &arena,
            r"
use Whim\Marker\NeverInline;
final class Effects { public static int $calls = 0; }
#[NeverInline]
function take(Transient<uint> $value): uint { Effects::$calls++; return $value; }
#[NeverInline]
function caller(): uint { return take(2u); }
#[NeverInline]
function stage(): string { assert!(caller() == 2u); return 'bad'; }
",
        )
        .unwrap();
        let unit = compile_with_configuration(
            program,
            "/pending-before-rollback.whim",
            &engine.heap,
            CompileConfiguration {
                optimization: OptimizationConfiguration {
                    enabled: false,
                    ..OptimizationConfiguration::default()
                },
                trusted_return_types: false,
            },
        )
        .unwrap();
        let (unit, lazy) = engine.optimize_required_unit_against_world(unit);
        assert_eq!(lazy, optimize);
        assert!(
            engine
                .declare_compiled(&Rc::new(unit), Vec::new(), None, lazy)
                .is_ok()
        );
        let caller = engine
            .tables
            .functions
            .iter()
            .position(|function| function.name.as_bytes() == b"caller")
            .unwrap();
        let result = engine.run_source(
            r"
type Transient<T: uint> = T&!0u;
final class Broken { public static int $value = stage(); }
",
            Path::new("/pending-failed-declaration.whim"),
        );
        assert_ne!(result.exit_code(), 0);
        if optimize {
            assert!(engine.tables.functions[caller].optimization == CallableOptimization::Pending);
        }
        let result = engine.run_source(
            r"
$caught = false;
try { caller(); } catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
assert!(Effects::$calls == 1);
",
            Path::new("/pending-after-rollback.whim"),
        );
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
        if optimize {
            assert!(engine.tables.functions[caller].optimization == CallableOptimization::Complete);
            // SAFETY: the idle engine owns the callable and its current chunk.
            let chunk = unsafe { engine.tables.functions[caller].chunk.as_ref() };
            assert!(chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallNamed { .. } | Instruction::CallNamedDirect { .. }
            )));
        }
    }
}
