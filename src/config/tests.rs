use std::error::Error as _;
use std::path::Path;

use crate::config::Manifest;

#[test]
fn compact_and_detailed_dependencies_are_equivalent() {
    let compact = Manifest::parse(
        "manifest-version = 1\n[dependencies]\n\"git+https://github.com/acme/a\" = \"^1.2\"\n",
        true,
    )
    .expect("the compact manifest is valid");
    let detailed = Manifest::parse(
        "manifest-version = 1\n[dependencies]\n\"git+https://github.com/acme/a\" = { version = \"^1.2\" }\n",
        true,
    )
    .expect("the detailed manifest is valid");

    assert_eq!(
        compact.resolution_hash().expect("the hash is available"),
        detailed.resolution_hash().expect("the hash is available")
    );
}

#[test]
fn metadata_and_formatting_do_not_change_resolution() {
    let first =
        Manifest::parse("manifest-version = 1\n", true).expect("the first manifest is valid");
    let second = Manifest::parse(
        "manifest-version = 1\n[package]\nauthor = \"Whim\"\n[format]\nprint_width = 100\n",
        true,
    )
    .expect("the second manifest is valid");

    assert_eq!(
        first.resolution_hash().expect("the hash is available"),
        second.resolution_hash().expect("the hash is available")
    );
}

#[test]
fn runtime_settings_apply_without_changing_resolution() {
    let plain = Manifest::parse("manifest-version = 1\n", true).expect("the manifest is valid");
    let configured = Manifest::parse(
        "manifest-version = 1\n[runtime]\noptimizations = \"off\"\ncall-depth = 42\ncycle-threshold = 7\nfull-trace = true\n",
        true,
    )
    .expect("the runtime settings are valid");
    let runtime = configured.runtime.engine_configuration();

    assert!(!runtime.optimize);
    assert_eq!(runtime.call_depth_limit, 42);
    assert_eq!(runtime.cycle_threshold, Some(7));
    assert!(runtime.full_trace);
    assert_eq!(
        plain.resolution_hash().expect("the hash is available"),
        configured.resolution_hash().expect("the hash is available")
    );
}

#[test]
fn runtime_settings_reject_unknown_values_and_fields() {
    assert!(
        Manifest::parse(
            "manifest-version = 1\n[runtime]\noptimizations = \"maybe\"\n",
            true,
        )
        .is_err()
    );
    assert!(Manifest::parse("manifest-version = 1\n[runtime]\ncall_depth = 10\n", true,).is_err());
}

#[test]
fn format_patterns_are_project_relative_globs() {
    let manifest = Manifest::parse(
        "manifest-version = 1\n[format]\ninclude = [\"src\"]\nexclude = [\"src/generated/**\"]\n",
        true,
    )
    .expect("the format patterns are valid");
    let patterns = manifest
        .format
        .patterns()
        .expect("validated patterns compile again");

    assert!(patterns.includes(Path::new("src/App.whim")));
    assert!(!patterns.includes(Path::new("tests/App.whim")));
    assert!(patterns.excludes(Path::new("src/generated/App.whim")));
    assert!(patterns.excludes(Path::new("vendor/package/App.whim")));
    assert!(patterns.excludes(Path::new(".git/hooks/check.whim")));
}

#[test]
fn format_patterns_reject_unsafe_and_invalid_values() {
    for pattern in [
        "",
        "./",
        "/tmp/**",
        "../outside/**",
        "src\\**",
        "src/[broken",
    ] {
        let source = format!(
            "manifest-version = 1\n[format]\nexclude = [{}]\n",
            toml::Value::String(pattern.to_owned())
        );
        assert!(
            Manifest::parse(&source, true).is_err(),
            "the pattern {pattern:?} should be rejected"
        );
    }
}

#[test]
fn duplicate_normalized_sources_are_rejected() {
    let error = Manifest::parse(
        "manifest-version = 1\n[dependencies]\n\"https://github.com/acme/a.git\" = \"^1\"\n\"git+https://github.com/acme/a\" = \"^2\"\n",
        true,
    )
    .expect_err("the duplicate must be rejected");

    assert!(error.to_string().contains("duplicate normalized"));
}

#[test]
fn dependency_group_changes_make_the_root_lock_stale() {
    let runtime = Manifest::parse(
        "manifest-version = 1\n[dependencies]\n\"git+https://github.com/acme/a\" = \"^1\"\n",
        true,
    )
    .expect("the runtime manifest is valid");
    let development = Manifest::parse(
        "manifest-version = 1\n[dev-dependencies]\n\"git+https://github.com/acme/a\" = \"^1\"\n",
        true,
    )
    .expect("the development manifest is valid");
    assert_ne!(
        runtime.resolution_hash().expect("the hash is available"),
        development
            .resolution_hash()
            .expect("the hash is available")
    );
}

#[test]
fn consumed_hash_ignores_development_dependencies() {
    let plain =
        Manifest::parse("manifest-version = 1\n", false).expect("the plain manifest is valid");
    let development = Manifest::parse(
        "manifest-version = 1\n[dev-dependencies]\n\"git+https://github.com/acme/a\" = \"^1\"\n",
        false,
    )
    .expect("the development manifest is valid");
    assert_eq!(
        plain
            .consumed_resolution_hash()
            .expect("the hash is available"),
        development
            .consumed_resolution_hash()
            .expect("the hash is available")
    );
}

#[test]
fn consumed_overrides_and_reserved_prefixes_are_rejected() {
    assert!(
        Manifest::parse(
            "manifest-version = 1\n[overrides]\n\"git+https://github.com/acme/a\" = \"git+https://github.com/acme/b\"\n",
            false,
        )
        .is_err()
    );
    assert!(
        Manifest::parse(
            "manifest-version = 1\n[autoload.namespaces]\n\"Whim\\\\Foo\\\\\" = \"src/\"\n",
            true,
        )
        .is_err()
    );
}

#[test]
fn autoload_directories_reject_control_characters() {
    assert!(
        Manifest::parse(
            "manifest-version = 1\n[autoload.namespaces]\n\"App\\\\\" = \"src\\tgenerated/\"\n",
            true,
        )
        .is_err()
    );
}

#[test]
fn empty_autoload_directories_report_the_violated_requirement() {
    let error = Manifest::parse(
        "manifest-version = 1\n[autoload.namespaces]\n\"App\\\\\" = \"\"\n",
        true,
    )
    .expect_err("the empty directory must be rejected");

    assert_eq!(
        error.to_string(),
        "autoload directory for prefix `App\\` must not be empty"
    );
}

#[test]
fn cargo_requirement_forms_are_accepted_without_alternatives() {
    for requirement in ["=1.2.3", "^1.2", "~1.2", "1.*", ">=1.0, <2.0"] {
        let text = format!(
            "manifest-version = 1\n[dependencies]\n\"git+https://github.com/acme/a\" = \"{requirement}\"\n"
        );
        assert!(Manifest::parse(&text, true).is_ok(), "{requirement}");
    }
    assert!(
        Manifest::parse(
            "manifest-version = 1\n[dependencies]\n\"git+https://github.com/acme/a\" = \"^1 || ^2\"\n",
            true,
        )
        .is_err()
    );
}

#[test]
fn package_versions_and_dependency_options_are_not_part_of_the_schema() {
    assert!(
        Manifest::parse(
            "manifest-version = 1\n[package]\nversion = \"1.0.0\"\n",
            true,
        )
        .is_err()
    );
    assert!(
        Manifest::parse(
            "manifest-version = 1\n[dependencies]\n\"git+https://github.com/acme/a\" = { version = \"^1\", branch = \"main\" }\n",
            true,
        )
        .is_err()
    );
}

#[test]
fn conflicts_affect_resolution_hash_but_package_metadata_and_suggestions_do_not() {
    let plain = Manifest::parse("manifest-version = 1\n", true).expect("the manifest is valid");
    let conflict = Manifest::parse(
        "manifest-version = 1\n[conflicts]\n\"git+https://github.com/acme/a\" = \"^1\"\n",
        true,
    )
    .expect("the conflict is valid");
    let metadata = Manifest::parse(
        "manifest-version = 1\n[package]\nlicense = \"MIT OR Apache-2.0\"\nsponsor = \"https://github.com/sponsors/acme\"\n[suggests]\n\"git+https://github.com/acme/a\" = \"^1\"\n",
        true,
    )
    .expect("the metadata is valid");

    assert_ne!(
        plain.resolution_hash().expect("the hash is available"),
        conflict.resolution_hash().expect("the hash is available")
    );
    assert_eq!(
        plain.resolution_hash().expect("the hash is available"),
        metadata.resolution_hash().expect("the hash is available")
    );
}

#[test]
fn licenses_are_spdx_expressions_and_sponsors_are_web_urls() {
    let license = Manifest::parse(
        "manifest-version = 1\n[package]\nlicense = \"MIT AND NOPE\"\n",
        true,
    )
    .expect_err("the unknown SPDX identifier must fail");
    assert!(license.source().is_some());

    let sponsor = Manifest::parse(
        "manifest-version = 1\n[package]\nsponsor = \"not a URL\"\n",
        true,
    )
    .expect_err("the invalid URL must fail");
    assert!(sponsor.source().is_some());

    assert!(
        Manifest::parse(
            "manifest-version = 1\n[package]\nsponsor = \"file:///tmp/money\"\n",
            true,
        )
        .is_err()
    );
    let credentials = Manifest::parse(
        "manifest-version = 1\n[package]\nsponsor = \"https://user:secret@example.com\"\n",
        true,
    )
    .expect_err("sponsor credentials must fail");
    assert!(!credentials.to_string().contains("secret"));
}
