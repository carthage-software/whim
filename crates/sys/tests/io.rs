use std::error::Error as StdError;
use std::net::IpAddr;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::id;
use std::str::from_utf8;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use std::{env, fs, result};

#[cfg(windows)]
use whim_sys::Error;
use whim_sys::file;
use whim_sys::filesystem;
use whim_sys::message;
use whim_sys::operation::Operation;
use whim_sys::path;
use whim_sys::process::{self, Processes, Spawn, Stream};
use whim_sys::socket;
use whim_sys::{Descriptor, Interest, constants};

type Result<T = ()> = result::Result<T, Box<dyn StdError>>;

struct Directory(PathBuf);

impl Directory {
    fn new() -> Result<Self> {
        Ok(Self(filesystem::create_temporary_directory(
            &env::temp_dir(),
            b"whim-sys-",
        )?))
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn until<T>(mut operation: impl FnMut() -> whim_sys::Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(value) = operation()? {
            return Ok(value);
        }

        assert!(
            Instant::now() < deadline,
            "I/O did not finish before the deadline"
        );

        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn file_operations_preserve_offsets_metadata_and_unlinked_temporary_files() -> Result {
    let directory = Directory::new()?;
    let (file, path) = filesystem::create_temporary_file(&directory.0, b"file-", 0o600)?;
    assert_eq!(file::write(&file, b"abcdefgh")?, 8);
    file::synchronize(&file)?;
    assert_eq!(file::seek(&file, 2, 0)?, 2);
    assert_eq!(file::read(&file, 3)?, b"cde");
    assert_eq!(file::seek(&file, -2, 1)?, 3);
    assert_eq!(file::read_path(&path, 3, Some(2))?, b"de");
    file::truncate(&file, 4)?;
    assert_eq!(file::metadata(&file)?.size, 4);
    assert_eq!(file::path_metadata(&path, true)?.size, 4);
    assert_eq!(file::read_path(&path, 10, None)?, b"");
    let temporary = file::temporary(&directory.0)?;
    assert_eq!(file::write(&temporary, b"temporary")?, 9);
    assert_eq!(file::seek(&temporary, 0, 0)?, 0);
    assert_eq!(file::read(&temporary, 9)?, b"temporary");
    drop(temporary);
    assert_eq!(filesystem::read_directory(&directory.0)?.len(), 1);
    Ok(())
}

#[test]
fn locks_keep_their_handle_until_unlock() -> Result {
    let directory = Directory::new()?;
    let (file, path) = filesystem::create_temporary_file(&directory.0, b"lock-", 0o600)?;
    let descriptor = Descriptor::from_file(file);
    let other = file::open(&path, i64::from(constants::O_RDWR), 0o600)?;
    assert!(file::lock(
        descriptor.file_for_lock()?.as_ref(),
        i64::from(constants::LOCK_EX),
        false
    )?);

    assert!(!file::lock(&other, i64::from(constants::LOCK_EX), false)?);
    assert!(file::lock(
        descriptor.file_for_lock()?.as_ref(),
        i64::from(constants::LOCK_UN),
        false
    )?);

    assert!(file::lock(&other, i64::from(constants::LOCK_EX), false)?);
    assert!(file::lock(&other, i64::from(constants::LOCK_UN), false)?);
    Ok(())
}

#[test]
fn directory_relative_open_rejects_escapes_and_follows_the_open_directory() -> Result {
    let directory = Directory::new()?;
    fs::create_dir(directory.0.join("root"))?;
    fs::create_dir(directory.0.join("root/nested"))?;
    fs::write(directory.0.join("root/nested/value"), b"inside")?;
    fs::write(directory.0.join("outside"), b"outside")?;
    let root =
        filesystem::open_directory(&directory.0.join("root")).ok_or("directory did not open")?;
    for path in [
        b"../outside".as_slice(),
        b"nested/../../outside",
        b"nested//value",
        b"/outside",
        b"nested/./value",
        b"nested/value\0extra",
    ] {
        assert!(filesystem::open_regular_file_beneath(&root, path)?.is_none());
    }

    fs::rename(directory.0.join("root"), directory.0.join("moved"))?;
    let (file, metadata) = filesystem::open_regular_file_beneath(&root, b"nested/value")?
        .ok_or("file did not open")?;
    assert_eq!(metadata.size, 6);
    assert_eq!(file.read(6)?, Some(b"inside".to_vec()));
    #[cfg(unix)]
    {
        symlink("../../outside", directory.0.join("moved/nested/link"))?;
        symlink("nested", directory.0.join("moved/link"))?;
        assert!(filesystem::open_regular_file_beneath(&root, b"nested/link")?.is_none());
        assert!(filesystem::open_regular_file_beneath(&root, b"link/value")?.is_none());
        assert!(filesystem::open_directory(&directory.0.join("moved/link")).is_none());
    }

    Ok(())
}

#[test]
fn temporary_names_cannot_escape_the_directory() -> Result {
    let directory = Directory::new()?;
    for prefix in [b"../escape".as_slice(), b"nested/name", b"nul\0name"] {
        assert!(filesystem::create_temporary_file(&directory.0, prefix, 0o600).is_err());
        assert!(filesystem::create_temporary_directory(&directory.0, prefix).is_err());
    }

    assert!(filesystem::read_directory(&directory.0)?.is_empty());
    Ok(())
}

#[test]
fn pipes_report_pending_data_eof_and_write_backpressure() -> Result {
    let (read, write) = Descriptor::pipe()?;
    read.set_non_blocking(true)?;
    write.set_non_blocking(true)?;
    assert_eq!(read.read(32)?, None);
    let payload = vec![b'x'; 1024];
    let mut written = 0;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let count = write.write(&payload)?;
        if count == 0 {
            break;
        }

        written += count;
        assert!(
            Instant::now() < deadline,
            "pipe never reported backpressure"
        );
    }

    assert!(written > 0);
    assert!(read.is_ready(Interest::Readable)?);
    let mut received = 0;
    while let Some(bytes) = read.read(4096)? {
        assert!(!bytes.is_empty());
        received += bytes.len();
    }

    assert_eq!(received, written);
    drop(write);
    assert_eq!(until(|| read.read(1))?, b"");
    Ok(())
}

#[test]
fn sockets_preserve_pending_reads_and_peer_eof() -> Result {
    let (first, second) = Descriptor::socket_pair()?;
    assert_eq!(first.read(32)?, None);
    assert_eq!(second.write(b"hello")?, 5);
    assert_eq!(until(|| first.read(32))?, b"hello");
    drop(second);
    assert_eq!(until(|| first.read(32))?, b"");
    Ok(())
}

#[test]
fn datagrams_keep_addresses_congestion_and_truncation() -> Result {
    let receiver = socket::create(
        i64::from(constants::AF_INET),
        i64::from(constants::SOCK_DGRAM),
    )?;

    socket::bind(&receiver, b"127.0.0.1", 0)?;
    let local = socket::local_address(&receiver)?;
    message::enable_metadata(&receiver)?;
    let sender = socket::create(
        i64::from(constants::AF_INET),
        i64::from(constants::SOCK_DGRAM),
    )?;

    assert_eq!(
        message::send(
            &sender,
            b"a datagram",
            Some(b"127.0.0.1"),
            local.port,
            b"127.0.0.1",
            0,
            2
        )?,
        10
    );
    let received = until(|| message::receive(&receiver, 4, local.port))?;
    assert_eq!(received.bytes, b"a da");
    assert_eq!(received.local, local);
    assert_eq!(received.peer.host, b"127.0.0.1");
    assert!(received.peer.port > 0);
    assert_eq!(received.congestion, 2);
    assert!(received.truncated);
    Ok(())
}

#[test]
fn datagrams_reply_from_wildcard_and_dual_stack_sockets() -> Result {
    for (family, wildcard, loopback, binding, only_ipv6) in [
        (
            constants::AF_INET,
            "0.0.0.0",
            "127.0.0.1",
            "127.0.0.1",
            true,
        ),
        (constants::AF_INET6, "::", "::1", "::1", true),
        (
            constants::AF_INET6,
            "::",
            "::ffff:127.0.0.1",
            "::ffff:127.0.0.1",
            false,
        ),
        (constants::AF_INET6, "::", "::ffff:127.0.0.1", "::", false),
    ] {
        let receiver = socket::create(i64::from(family), i64::from(constants::SOCK_DGRAM))?;
        let sender = socket::create(i64::from(family), i64::from(constants::SOCK_DGRAM))?;
        for socket in [&receiver, &sender] {
            if family == constants::AF_INET6 {
                socket::set_option(
                    socket,
                    i64::from(constants::IPPROTO_IPV6),
                    i64::from(constants::IPV6_V6ONLY),
                    i64::from(only_ipv6),
                )?;
            }
        }
        socket::bind(&receiver, wildcard.as_bytes(), 0)?;
        socket::bind(&sender, binding.as_bytes(), 0)?;
        message::enable_metadata(&receiver)?;
        message::enable_metadata(&sender)?;
        let destination = socket::local_address(&receiver)?;
        assert_eq!(
            message::send(
                &sender,
                b"request",
                Some(loopback.as_bytes()),
                destination.port,
                loopback.as_bytes(),
                0,
                1,
            )
            .map_err(|error| format!("{loopback} request: {error}"))?,
            7,
            "{loopback}"
        );
        let received = until(|| message::receive(&receiver, 64, destination.port))?;
        assert_eq!(received.bytes, b"request", "{loopback}");
        assert_eq!(received.local.host, loopback.as_bytes(), "{loopback}");
        assert!(received.interface > 0, "{loopback}");
        assert_eq!(received.congestion, 1, "{loopback}");
        let interface = if family == constants::AF_INET6 {
            received.interface
        } else {
            0
        };
        assert_eq!(
            message::send(
                &receiver,
                b"reply",
                Some(&received.peer.host),
                received.peer.port,
                &received.local.host,
                interface,
                2,
            )
            .map_err(|error| format!("{loopback} reply: {error}"))?,
            5,
            "{loopback}"
        );
        let reply = until(|| message::receive(&sender, 64, received.peer.port))?;
        assert_eq!(reply.bytes, b"reply", "{loopback}");
        let peer = from_utf8(&reply.peer.host)?.parse::<IpAddr>()?;
        assert_eq!(
            peer.to_canonical(),
            loopback.parse::<IpAddr>()?.to_canonical()
        );
        if !cfg!(target_os = "freebsd") || binding != "::ffff:127.0.0.1" {
            assert!(reply.interface > 0, "{loopback}");
            assert_eq!(reply.congestion, 2, "{loopback}");
        }
    }
    Ok(())
}

#[test]
fn operation_completion_and_cancellation_wake_waiters() -> Result {
    for cancelled in [false, true] {
        let operation = Operation::<u32, ()>::new()?;
        let worker = Arc::clone(&operation);
        let (send, receive) = mpsc::channel();
        let waiter = thread::spawn(move || {
            while !worker.is_complete() {
                worker.wait().unwrap();
                worker.drain();
            }
            send.send(worker.take()).unwrap();
        });

        if cancelled {
            operation.cancel();
            operation.complete(Ok(7));
        } else {
            operation.complete(Ok(7));
        }

        assert_eq!(
            receive.recv_timeout(Duration::from_secs(15))?,
            Some(Ok(if cancelled { None } else { Some(7) }))
        );

        waiter.join().unwrap();
        operation.complete(Ok(9));
        if !cancelled {
            assert!(operation.take().is_none());
        }
    }

    Ok(())
}

#[test]
fn spawned_processes_report_output_and_exit_status() -> Result {
    #[cfg(unix)]
    let (program, arguments) = (
        "/bin/sh",
        vec!["-c", "printf output; printf error >&2; exit 7"],
    );

    #[cfg(windows)]
    let (program, arguments) = (
        "C:\\Windows\\System32\\cmd.exe",
        vec!["/d", "/c", "echo output & echo error 1>&2 & exit /b 7"],
    );

    let mut processes = Processes::default();
    let spawned = processes.spawn(Spawn {
        program: program.into(),
        arguments: arguments.into_iter().map(Into::into).collect(),
        environment: None,
        directory: None,
        streams: [Stream::Null, Stream::Pipe, Stream::Pipe],
        inherited: Vec::new(),
        group: None,
    })?;

    let watch = processes.watch(i64::from(spawned.id))?;
    let status = until(|| process::read_exit(&watch))?;
    assert_eq!(processes.record_exit(status), 7 << 8);
    assert!(until(|| spawned.output.as_ref().unwrap().read(32))?.starts_with(b"output"));
    assert!(until(|| spawned.error.as_ref().unwrap().read(32))?.starts_with(b"error"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn signal_subscriptions_deliver_readiness() -> Result {
    let descriptor = process::watch_signal(i64::from(constants::SIGUSR1))?;
    process::send_signal(i64::from(id()), i64::from(constants::SIGUSR1))?;
    assert_eq!(until(|| descriptor.read(1))?.len(), 1);
    Ok(())
}

#[cfg(windows)]
#[test]
fn unsupported_operations_fail_before_spawning() -> Result {
    assert!(matches!(process::user(), Err(Error::Unsupported(_))));
    assert!(matches!(
        process::watch_signal(2),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        process::validate_spawn(true, false),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        process::validate_spawn(false, true),
        Err(Error::Unsupported(_))
    ));
    Ok(())
}

#[test]
fn platform_path_bytes_round_trip() -> Result {
    #[cfg(unix)]
    let bytes = b"name-\xff".as_slice();
    #[cfg(windows)]
    let bytes = b"name-\xed\xa0\x80".as_slice();
    let file = path::path_from_bytes(bytes)?;
    assert_eq!(path::path_bytes(&file), bytes);
    let argument = path::os_string_from_bytes(b"https://example.com/a/b")?;
    assert_eq!(argument, "https://example.com/a/b");
    Ok(())
}
