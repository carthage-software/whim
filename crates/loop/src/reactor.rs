//! OS file-descriptor readiness, over the `polling` crate.

use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;

use polling::Event;
use polling::Events;
use polling::PollMode;
use polling::Poller;

/// The readiness direction a coroutine waits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interest {
    /// Read readiness.
    Readable,
    /// Write readiness.
    Writable,
    /// Read or write readiness.
    ReadableOrWritable,
}

/// The engine's single reactor.
pub(crate) struct Reactor {
    poller: Poller,
    events: Events,
    mode: PollMode,
}

impl Reactor {
    /// Opens the platform poller.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the poller cannot be opened.
    pub(crate) fn new() -> io::Result<Self> {
        let poller = Poller::new()?;
        let mode = if poller.supports_edge() {
            PollMode::Edge
        } else {
            PollMode::Oneshot
        };
        Ok(Self {
            poller,
            events: Events::new(),
            mode,
        })
    }

    /// Registers `fd` for `interest`, tagged with `key`.
    ///
    /// # Safety
    ///
    /// Keep `fd` open until [`deregister`](Reactor::deregister) removes it.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when registration fails.
    pub(crate) unsafe fn register(
        &self,
        fd: BorrowedFd<'_>,
        key: usize,
        interest: Interest,
    ) -> io::Result<()> {
        // SAFETY: the caller keeps `fd` open until it is deregistered.
        unsafe {
            self.poller
                .add_with_mode(&fd, event_for(key, interest), self.mode)
        }
    }

    /// Re-arms an already-registered `fd` for `interest` under `key`.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the registration cannot be changed.
    pub(crate) fn rearm(
        &self,
        fd: BorrowedFd<'_>,
        key: usize,
        interest: Interest,
    ) -> io::Result<()> {
        self.poller
            .modify_with_mode(fd, event_for(key, interest), self.mode)
    }

    /// Whether this platform needs an explicit rearm after each event.
    #[must_use]
    pub(crate) const fn requires_rearm(&self) -> bool {
        matches!(self.mode, PollMode::Oneshot)
    }

    /// Removes `fd` from the reactor. The caller must do this before closing the
    /// descriptor.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the descriptor cannot be removed.
    pub(crate) fn deregister(&self, fd: BorrowedFd<'_>) -> io::Result<()> {
        self.poller.delete(fd)
    }

    /// Blocks until a descriptor is ready or `timeout` elapses.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when polling fails.
    pub(crate) fn wait(
        &mut self,
        timeout: Option<Duration>,
        readiness: &mut Vec<usize>,
    ) -> io::Result<()> {
        self.events.clear();
        self.poller.wait(&mut self.events, timeout)?;
        readiness.clear();
        readiness.extend(self.events.iter().map(|event| event.key));
        Ok(())
    }
}

const fn event_for(key: usize, interest: Interest) -> Event {
    match interest {
        Interest::Readable => Event::readable(key),
        Interest::Writable => Event::writable(key),
        Interest::ReadableOrWritable => Event::all(key),
    }
}

#[cfg(test)]
mod tests {
    use crate::reactor::*;
    use std::io::Read;
    use std::io::Write;
    use std::net::TcpListener;
    use std::net::TcpStream;
    use std::os::fd::AsFd;

    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let address = listener.local_addr().expect("local address");
        let client = TcpStream::connect(address).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        server.set_nonblocking(true).expect("non-blocking");
        (server, client)
    }

    #[test]
    fn a_registered_descriptor_reports_readable_when_data_arrives() {
        let (server, mut client) = socket_pair();
        let mut reactor = Reactor::new().expect("reactor");
        // SAFETY: `server` remains open until it is deregistered.
        unsafe { reactor.register(server.as_fd(), 42, Interest::Readable) }.expect("register");

        let mut idle = Vec::new();
        reactor
            .wait(Some(Duration::from_millis(10)), &mut idle)
            .expect("wait");
        assert!(idle.is_empty(), "no data yet, expected no readiness");

        client.write_all(b"ping").expect("write");
        let mut ready = Vec::new();
        reactor
            .wait(Some(Duration::from_secs(2)), &mut ready)
            .expect("wait");
        assert_eq!(ready.len(), 1, "exactly one source ready");
        assert_eq!(ready[0], 42);

        reactor.deregister(server.as_fd()).expect("deregister");
    }

    #[test]
    fn re_arming_delivers_a_second_readiness() {
        let (mut server, mut client) = socket_pair();
        let mut reactor = Reactor::new().expect("reactor");
        // SAFETY: `server` remains open for the whole test.
        unsafe { reactor.register(server.as_fd(), 7, Interest::Readable) }.expect("register");

        client.write_all(b"a").expect("write");
        let mut first = Vec::new();
        reactor
            .wait(Some(Duration::from_secs(2)), &mut first)
            .expect("wait");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0], 7);

        let mut buffer = [0u8; 1];
        server.read_exact(&mut buffer).expect("read");
        reactor
            .rearm(server.as_fd(), 7, Interest::Readable)
            .expect("rearm");

        client.write_all(b"b").expect("write");
        let mut second = Vec::new();
        reactor
            .wait(Some(Duration::from_secs(2)), &mut second)
            .expect("wait");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0], 7);
    }
}
