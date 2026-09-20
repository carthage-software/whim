use std::env;
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::process;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

#[test]
fn source_paths_preserve_non_utf8_bytes_for_embedding() {
    let prefix = format!("whim-non-utf8-path-{}-", process::id());
    #[cfg(unix)]
    let name = OsString::from_vec([prefix.as_bytes(), &[0xff]].concat());
    #[cfg(windows)]
    let name = OsString::from_wide(&prefix.encode_utf16().chain([0xd800]).collect::<Vec<_>>());
    let directory = env::temp_dir().join(name);
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
    fs::create_dir(directory.join("unused")).expect("the child directory is creatable");
    fs::write(&embedded_path, "embedded").expect("the embedded file is writable");

    let mut engine = Engine::new(EngineConfiguration::default());
    let outcome = engine.run_source(
        r"
assert!(embed!('payload.txt') == 'embedded');
assert!(Whim\_Private\read_file(directory!() . '/./unused/../payload.txt', 0, null) == 'embedded');
$path = Whim\Reflection\get_loaded_files()[0]->getPath();
assert!(Whim\Reflection\reflect_file($path)->getPath() == $path);
",
        &source_path,
    );

    fs::remove_dir_all(directory).expect("the test directory is removable");
    assert_eq!(outcome.exit_code(), 0);
}
