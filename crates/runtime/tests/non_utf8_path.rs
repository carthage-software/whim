use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::process;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn source_paths_preserve_non_utf8_bytes_for_embedding() {
    let mut name = format!("whim-non-utf8-path-{}-", process::id()).into_bytes();
    name.push(0xff);
    let directory = env::temp_dir().join(OsString::from_vec(name));
    let source_path = directory.join("main.whim");
    let embedded_path = directory.join("payload.txt");

    if directory.exists() {
        fs::remove_dir_all(&directory).expect("the old test directory is removable");
    }
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == Some(libc::EILSEQ) => return,
        Err(error) => panic!("the test directory is creatable: {error}"),
    }
    fs::write(&embedded_path, "embedded").expect("the embedded file is writable");

    let mut engine = Engine::new(EngineConfiguration::default());
    let outcome = engine.run_source(
        "assert!(embed!('payload.txt') == 'embedded');",
        &source_path,
    );

    fs::remove_dir_all(directory).expect("the test directory is removable");
    assert_eq!(outcome.exit_code(), 0);
}
