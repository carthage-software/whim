use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn file_attributes_keep_scope_order_and_arguments_in_both_modes() {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });

        let result = engine.run_source(
            include_str!("../../../tests/language/file-attributes.whim"),
            Path::new("/file-attributes.whim"),
        );

        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn file_attributes_enforce_targets_repetition_and_arguments_before_execution() {
    for (declaration, application, expected) in [
        (
            "#[Whim\\Attribute\\Attribute(1)] class Tag {}",
            "if (false) { #![Tag] }",
            "does not target files",
        ),
        ("class Tag {}", "#![Tag]", "does not carry"),
        ("", "function uncalled() { #![Missing] }", "is not defined"),
        (
            "#[Whim\\Attribute\\Attribute(1024)] class Tag {}",
            "#![Tag] namespace Another { #![\\Tag] }",
            "not repeatable",
        ),
        (
            "#[Whim\\Attribute\\Attribute(1024)] class Tag {}",
            "#[Tag] function f() {}",
            "does not target functions",
        ),
        (
            "#[Whim\\Attribute\\Attribute(1024)] class Tag {}",
            "#![Tag(1)]",
            "declares no constructor",
        ),
        (
            "#[Whim\\Attribute\\Attribute(1024)] class Tag { public function __construct(string $name) {} }",
            "#![Tag]",
            "takes exactly 1",
        ),
        (
            "#[Whim\\Attribute\\Attribute(1024)] class Tag { public function __construct(string $name) {} }",
            "function uncalled(string $name) { #![Tag($name)] }",
            "an attribute argument is a constant expression",
        ),
        (
            "#[Whim\\Attribute\\Attribute(1024)] class Tag { public function __construct(fn(): int $value) {} }",
            "if (false) { #![Tag(fn(): int => $outside)] }",
            "a closure with captures",
        ),
    ] {
        let source = format!("{declaration}\nassert!(false, 'top-level code ran');\n{application}");
        let mut engine = Engine::new(EngineConfiguration::default());
        let result = engine.run_source(&source, Path::new("/invalid-file-attribute.whim"));
        assert_ne!(result.exit_code(), 0, "{source}");
        assert!(
            format!("{result:?}").contains(expected),
            "{source}: {result:?}"
        );
    }
}

#[test]
fn anonymous_and_reloaded_files_keep_their_own_attributes() {
    let mut engine = Engine::new(EngineConfiguration::default());
    for (path, source) in [
        (
            "/tag.whim",
            r"#[Whim\Attribute\Attribute(1024)] class Tag { public function __construct(public string $name) {} }",
        ),
        ("/reloaded.whim", "#![Tag('first')] class First {}"),
        ("/reloaded.whim", "#![Tag('second')]"),
        ("-", "#![Tag('anonymous')] class Anonymous {}"),
    ] {
        let result = engine.run_source(source, Path::new(path));
        assert_eq!(result.exit_code(), 0, "{result:?}");
    }

    let result = engine.run_source(r"
use Whim\Reflection;
$first = Reflection\reflect_class('First')->getFile();
$latest = Reflection\reflect_file('/reloaded.whim');
$anonymous = Reflection\reflect_class('Anonymous')->getFile();
assert!($first->getAttributes()[0]->newInstance()->name == 'first');
assert!($first->getAttributes()[0]->getTarget()->getAttributes()[0]->newInstance()->name == 'first');
assert!($latest->getAttributes()[0]->newInstance()->name == 'second');
assert!(!$latest->hasTopLevelCode());
assert!($anonymous->getPath() == null);
assert!($anonymous->getLocation() == null);
assert!($anonymous->getAttributes()[0]->newInstance()->name == 'anonymous');
assert!($anonymous->getAttributes()[0]->getTarget()->getPath() == null);
", Path::new("/check.whim"));
    assert_eq!(result.exit_code(), 0, "{result:?}");
}
