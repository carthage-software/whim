use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::num::NonZeroU64;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Child;
use std::process::ChildStderr;
use std::process::ChildStdout;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;
use std::str;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use rustix::process::Pid;
#[cfg(target_os = "linux")]
use rustix::process::Resource;
#[cfg(target_os = "linux")]
use rustix::process::Rlimit;
use rustix::process::Signal;
#[cfg(target_os = "linux")]
use rustix::process::getrlimit;
use rustix::process::kill_process_group;
#[cfg(target_os = "linux")]
use rustix::process::prlimit;
use semver::Version;

use super::Error;
use super::Operation;
use super::failed;
use super::git_command;

const MAXIMUM_CACHE_BYTES: u64 = 1024 * 1024 * 1024;
const MAXIMUM_ERROR_BYTES: usize = 64 * 1024;
const MAXIMUM_TAG_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MAXIMUM_TAG_METADATA_LINE_BYTES: usize = 4 * 1024;
const DEFAULT_NETWORK_TIMEOUT: Duration = Duration::from_secs(300);
const NETWORK_TIMEOUT_VARIABLE: &str = "WHIM_PACKAGE_NETWORK_TIMEOUT";
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) fn run<I, A>(
    operation: Operation,
    directory: &Path,
    arguments: I,
    input: Option<&[u8]>,
) -> Result<(), Error>
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    run_with_limit(
        operation,
        directory,
        arguments,
        input,
        MAXIMUM_CACHE_BYTES,
        network_timeout()?,
    )
}

pub(super) fn remote_version_tags(
    remote: &str,
    maximum_tags: usize,
) -> Result<BTreeSet<String>, Error> {
    let operation = Operation::ListRemoteTags;
    let timeout = network_timeout()?;
    let started = Instant::now();
    let mut command = git_command(None);
    command
        .args(["ls-remote", "--tags", "--refs", remote])
        .stdout(Stdio::piped());
    tracing::trace!(%operation, "running Git");
    let mut process = GitProcess::spawn(command, operation)?;
    let output = match process.take_output() {
        Ok(output) => output,
        Err(error) => return process.reject(error),
    };

    let reader = thread::spawn(move || read_remote_tags(BufReader::new(output), maximum_tags));
    let (output, tags) = finish_remote_tags(process, reader, started, timeout)?;
    if !output.status.success() {
        return Err(failed(operation, &output));
    }

    Ok(tags)
}

fn finish_remote_tags(
    mut process: GitProcess,
    reader: thread::JoinHandle<Result<BTreeSet<String>, Error>>,
    started: Instant,
    timeout: Duration,
) -> Result<(Output, BTreeSet<String>), Error> {
    let operation = process.operation;
    let mut reader = Some(reader);
    let mut tags = None;
    loop {
        if tags.is_none() && reader.as_ref().is_some_and(thread::JoinHandle::is_finished) {
            match finish_tag_reader(&mut reader, operation) {
                Ok(result) => tags = Some(result),
                Err(error) => return reject_remote_tags(process, reader, error),
            }
        }

        let status = match process.try_wait() {
            Ok(status) => status,
            Err(error) => return reject_remote_tags(process, reader, error),
        };

        if let Some(status) = status {
            let tags = match tags {
                Some(tags) => tags,
                None => match finish_tag_reader(&mut reader, operation) {
                    Ok(tags) => tags,
                    Err(error) => return reject_remote_tags(process, reader, error),
                },
            };

            return Ok((process.finish(Some(status))?, tags));
        }

        if started.elapsed() >= timeout {
            return reject_remote_tags(process, reader, timeout_error(operation, timeout));
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn finish_tag_reader(
    reader: &mut Option<thread::JoinHandle<Result<BTreeSet<String>, Error>>>,
    operation: Operation,
) -> Result<BTreeSet<String>, Error> {
    reader
        .take()
        .ok_or(Error::OutputReaderPanicked(operation))?
        .join()
        .map_err(|_| Error::OutputReaderPanicked(operation))?
}

fn reject_remote_tags<T>(
    mut process: GitProcess,
    reader: Option<thread::JoinHandle<Result<BTreeSet<String>, Error>>>,
    error: Error,
) -> Result<T, Error> {
    process.discard();
    if let Some(reader) = reader {
        let _ = reader.join();
    }

    Err(error)
}

fn read_remote_tags<R: BufRead>(
    mut output: R,
    maximum_tags: usize,
) -> Result<BTreeSet<String>, Error> {
    let mut line = Vec::new();
    let mut total = 0_u64;
    let mut count = 0_usize;
    let mut tags = BTreeSet::new();
    loop {
        line.clear();
        let read = output
            .read_until(b'\n', &mut line)
            .map_err(|source| Error::ReadOutput {
                operation: Operation::ListRemoteTags,
                source,
            })?;
        if read == 0 {
            return Ok(tags);
        }

        add_tag_metadata_size(&mut total, read)?;
        if line.len() > MAXIMUM_TAG_METADATA_LINE_BYTES {
            return Err(Error::TagMetadataLineTooLong {
                limit: MAXIMUM_TAG_METADATA_LINE_BYTES,
            });
        }
        if count >= maximum_tags {
            return Err(Error::TooManyTags {
                limit: maximum_tags,
            });
        }
        count += 1;

        if let Some(tag) = parse_remote_tag(&line)? {
            tags.insert(tag);
        }
    }
}

fn add_tag_metadata_size(total: &mut u64, read: usize) -> Result<(), Error> {
    let read = u64::try_from(read).map_err(|_| Error::TagMetadataSizeOverflow)?;
    *total = total
        .checked_add(read)
        .ok_or(Error::TagMetadataSizeOverflow)?;
    if *total > MAXIMUM_TAG_METADATA_BYTES {
        return Err(Error::TagMetadataTooLarge {
            limit: MAXIMUM_TAG_METADATA_BYTES,
        });
    }

    Ok(())
}

fn parse_remote_tag(line: &[u8]) -> Result<Option<String>, Error> {
    let line = str::from_utf8(line).map_err(Error::NonUtf8TagMetadata)?;
    let line = line.trim_end_matches(['\r', '\n']);
    let Some((_, reference)) = line.split_once('\t') else {
        return Err(Error::MalformedTagMetadata);
    };
    let Some(tag) = reference.strip_prefix("refs/tags/") else {
        return Err(Error::MalformedTagMetadata);
    };
    let version = tag.strip_prefix('v').unwrap_or(tag);
    if Version::parse(version).is_err() {
        return Ok(None);
    }

    Ok(Some(tag.to_owned()))
}

fn run_with_limit<I, A>(
    operation: Operation,
    directory: &Path,
    arguments: I,
    input: Option<&[u8]>,
    limit: u64,
    timeout: Duration,
) -> Result<(), Error>
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    let initial_size = cache_size(directory)?;
    if initial_size > limit {
        return reject_cache(directory, Error::CacheTooLarge { limit });
    }
    if initial_size == limit {
        return Err(Error::CacheTooLarge { limit });
    }

    let mut command = git_command(Some(directory));
    command.args(arguments).stdout(Stdio::null());
    if input.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }

    tracing::trace!(%operation, ?directory, limit, "running bounded Git");
    let started = Instant::now();
    let mut process = GitProcess::spawn(command, operation)?;
    #[cfg(target_os = "linux")]
    if let Err(error) = limit_child_file_size(process.child(), limit - initial_size)
        && !matches!(process.try_wait(), Ok(Some(_)))
    {
        return reject_running_cache(process, directory, error);
    }
    if let Some(input) = input
        && let Err(error) = process.write_input(input)
    {
        return reject_running_cache(process, directory, error);
    }

    let status = match wait_for_cache(&mut process, directory, limit, started, timeout) {
        Ok(status) => status,
        Err(error) => return reject_running_cache(process, directory, error),
    };
    let output = process.finish(Some(status))?;
    let final_size = cache_size(directory)?;
    if final_size > limit || output.status.signal() == Some(libc::SIGXFSZ) {
        return reject_cache(directory, Error::CacheTooLarge { limit });
    }
    if !output.status.success() {
        return Err(failed(operation, &output));
    }

    Ok(())
}

fn wait_for_cache(
    process: &mut GitProcess,
    directory: &Path,
    limit: u64,
    started: Instant,
    timeout: Duration,
) -> Result<ExitStatus, Error> {
    loop {
        if let Some(status) = process.try_wait()? {
            return Ok(status);
        }
        if cache_size(directory)? > limit {
            return Err(Error::CacheTooLarge { limit });
        }

        if started.elapsed() >= timeout {
            return Err(timeout_error(process.operation, timeout));
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn network_timeout() -> Result<Duration, Error> {
    match env::var(NETWORK_TIMEOUT_VARIABLE) {
        Ok(value) => parse_network_timeout(value),
        Err(env::VarError::NotPresent) => Ok(DEFAULT_NETWORK_TIMEOUT),
        Err(source) => Err(Error::InvalidNetworkTimeoutEnvironment(source)),
    }
}

fn parse_network_timeout(value: String) -> Result<Duration, Error> {
    value
        .parse::<NonZeroU64>()
        .map(|seconds| Duration::from_secs(seconds.get()))
        .map_err(|source| Error::InvalidNetworkTimeout { value, source })
}

const fn timeout_error(operation: Operation, timeout: Duration) -> Error {
    Error::NetworkTimeout {
        operation,
        seconds: timeout.as_secs(),
    }
}

#[cfg(target_os = "linux")]
fn limit_child_file_size(child: &Child, maximum: u64) -> Result<(), Error> {
    let inherited = getrlimit(Resource::Fsize);
    prlimit(
        Some(Pid::from_child(child)),
        Resource::Fsize,
        Rlimit {
            current: Some(
                inherited
                    .current
                    .map_or(maximum, |value| value.min(maximum)),
            ),
            maximum: inherited.maximum,
        },
    )
    .map_err(Error::LimitChildFileSize)?;
    Ok(())
}

struct GitProcess {
    operation: Operation,
    child: Child,
    diagnostic_reader: Option<thread::JoinHandle<Result<Vec<u8>, IoError>>>,
}

impl GitProcess {
    fn spawn(mut command: Command, operation: Operation) -> Result<Self, Error> {
        command.process_group(0);
        let mut child = command
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| Error::Execute { operation, source })?;
        let Some(diagnostics) = child.stderr.take() else {
            stop_process_group(&mut child);
            return Err(Error::MissingDiagnostics(operation));
        };

        Ok(Self {
            operation,
            child,
            diagnostic_reader: Some(thread::spawn(move || read_diagnostics(diagnostics))),
        })
    }

    #[cfg(target_os = "linux")]
    const fn child(&self) -> &Child {
        &self.child
    }

    fn take_output(&mut self) -> Result<ChildStdout, Error> {
        self.child
            .stdout
            .take()
            .ok_or(Error::MissingOutput(self.operation))
    }

    fn write_input(&mut self, input: &[u8]) -> Result<(), Error> {
        let mut stdin = self
            .child
            .stdin
            .take()
            .ok_or(Error::MissingInput(self.operation))?;
        stdin.write_all(input).map_err(|source| Error::WriteInput {
            operation: self.operation,
            source,
        })
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, Error> {
        self.child.try_wait().map_err(|source| Error::Execute {
            operation: self.operation,
            source,
        })
    }

    fn finish(mut self, status: Option<ExitStatus>) -> Result<Output, Error> {
        let status = match status {
            Some(status) => status,
            None => match self.child.wait() {
                Ok(status) => status,
                Err(source) => {
                    stop(&mut self.child);
                    let _ = self.finish_diagnostics();
                    return Err(Error::Execute {
                        operation: self.operation,
                        source,
                    });
                }
            },
        };
        let stderr = self.finish_diagnostics()?;
        Ok(Output {
            status,
            stdout: Vec::new(),
            stderr,
        })
    }

    fn reject<T>(mut self, error: Error) -> Result<T, Error> {
        self.discard();
        Err(error)
    }

    fn discard(&mut self) {
        stop_process_group(&mut self.child);
        let _ = self.finish_diagnostics();
    }

    fn finish_diagnostics(&mut self) -> Result<Vec<u8>, Error> {
        self.diagnostic_reader
            .take()
            .ok_or(Error::MissingDiagnostics(self.operation))?
            .join()
            .map_err(|_| Error::DiagnosticReaderPanicked(self.operation))?
            .map_err(|source| Error::ReadDiagnostics {
                operation: self.operation,
                source,
            })
    }
}

fn stop_process_group(child: &mut Child) {
    if let Err(error) = kill_process_group(Pid::from_child(child), Signal::KILL) {
        tracing::debug!(%error, "could not stop rejected Git process group");
    }

    stop(child);
}

fn read_diagnostics(mut diagnostics: ChildStderr) -> Result<Vec<u8>, IoError> {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = diagnostics.read(&mut buffer)?;
        if count == 0 {
            return Ok(captured);
        }

        let remaining = MAXIMUM_ERROR_BYTES.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..count.min(remaining)]);
    }
}

pub(super) fn stop(child: &mut Child) {
    if let Err(error) = child.kill()
        && error.kind() != ErrorKind::InvalidInput
    {
        tracing::debug!(%error, "could not stop rejected Git process");
    }
    if let Err(error) = child.wait() {
        tracing::debug!(%error, "could not reap rejected Git process");
    }
}

fn cache_size(path: &Path) -> Result<u64, Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(source) => {
            return Err(Error::InspectCache {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    if metadata.file_type().is_file() {
        return Ok(metadata.len());
    }
    if !metadata.file_type().is_dir() {
        return Err(Error::InvalidCacheEntry(path.to_path_buf()));
    }

    let entries = fs::read_dir(path).map_err(|source| Error::InspectCache {
        path: path.to_path_buf(),
        source,
    })?;
    let mut size = 0_u64;
    for entry in entries {
        let entry = entry.map_err(|source| Error::InspectCache {
            path: path.to_path_buf(),
            source,
        })?;
        size = size
            .checked_add(cache_size(&entry.path())?)
            .ok_or(Error::CacheSizeOverflow)?;
    }

    Ok(size)
}

fn reject_running_cache<T>(
    process: GitProcess,
    directory: &Path,
    rejection: Error,
) -> Result<T, Error> {
    let mut process = process;
    process.discard();
    if matches!(rejection, Error::CacheTooLarge { .. }) {
        reject_cache(directory, rejection)
    } else {
        Err(rejection)
    }
}

fn reject_cache<T>(directory: &Path, rejection: Error) -> Result<T, Error> {
    match fs::remove_dir_all(directory) {
        Ok(()) => Err(rejection),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(rejection),
        Err(source) => {
            tracing::debug!(%rejection, "could not preserve the Git rejection after cleanup failed");
            Err(Error::RemoveRejectedCache {
                path: directory.to_path_buf(),
                source,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsStr;
    use std::fs;
    use std::io::BufReader;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process;
    use std::process::Command;
    use std::process::Stdio;
    use std::sync::atomic::AtomicU32;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;

    use super::Error;
    use super::GitProcess;
    use super::Operation;
    use super::cache_size;
    use super::finish_remote_tags;
    use super::parse_network_timeout;
    use super::read_remote_tags;
    use super::run_with_limit;
    use crate::package::git::run_git;

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn create() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);

            let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!("whim-git-test-{}-{ordinal}", process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).expect("the temporary directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("the temporary directory should be removed");
        }
    }

    #[test]
    fn bounded_git_removes_a_cache_that_crosses_its_quota() {
        let temporary = TemporaryDirectory::create();
        let cache = temporary.path().join("cache.git");
        run_git(
            Operation::Initialize,
            None,
            [
                OsStr::new("init"),
                OsStr::new("--bare"),
                OsStr::new("--template="),
                cache.as_os_str(),
            ],
        )
        .expect("the bare cache should be initialized");

        let mut state = 0x9e37_79b9_u32;
        let contents = (0..128 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state.to_le_bytes()[0]
            })
            .collect::<Vec<_>>();
        let input = temporary.path().join("input");
        fs::write(&input, contents).expect("the input should be written");
        let limit = cache_size(&cache).expect("the cache should be measurable") + 1024;
        let result = run_with_limit(
            Operation::FetchTags,
            &cache,
            [
                OsStr::new("hash-object"),
                OsStr::new("-w"),
                input.as_os_str(),
            ],
            None,
            limit,
            Duration::from_secs(30),
        );

        assert!(matches!(result, Err(Error::CacheTooLarge { .. })));
        assert!(!cache.exists());
    }

    #[test]
    fn network_timeout_must_be_a_positive_number_of_seconds() {
        assert_eq!(
            parse_network_timeout("12".to_owned()).expect("the timeout should be valid"),
            Duration::from_secs(12),
        );
        assert!(matches!(
            parse_network_timeout("0".to_owned()),
            Err(Error::InvalidNetworkTimeout { .. })
        ));
        assert!(matches!(
            parse_network_timeout("later".to_owned()),
            Err(Error::InvalidNetworkTimeout { .. })
        ));
    }

    #[test]
    fn remote_tag_processes_stop_at_the_network_deadline() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60"]).stdout(Stdio::piped());
        let mut process = GitProcess::spawn(command, Operation::ListRemoteTags)
            .expect("the test process should start");
        let output = process.take_output().expect("stdout should be piped");
        let reader = thread::spawn(move || read_remote_tags(BufReader::new(output), 1));
        let result = finish_remote_tags(process, reader, Instant::now(), Duration::from_millis(50));

        assert!(matches!(result, Err(Error::NetworkTimeout { .. })));
    }
}
