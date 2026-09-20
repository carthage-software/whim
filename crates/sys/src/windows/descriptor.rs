use std::fs::File;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::os::windows::io::{
    AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, OwnedSocket, RawHandle,
};
use std::process;
use std::ptr;
use std::sync::{Arc, Once};
use std::thread;
use std::time::Duration;

use socket2::SockRef;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_PIPE_CONNECTED_STATE, FILE_PIPE_LOCAL_INFORMATION, FilePipeLocalInformation,
    NtQueryInformationFile,
};
use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_BROKEN_PIPE, ERROR_NO_DATA,
    ERROR_PIPE_NOT_CONNECTED, RtlNtStatusToDosError,
};
use windows_sys::Win32::Networking::WinSock as ws;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_TYPE_CHAR, FILE_TYPE_PIPE, GetFileType, ReadFile, WriteFile,
};
use windows_sys::Win32::System::Console::{
    ENABLE_LINE_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
    GetNumberOfConsoleInputEvents, GetStdHandle, INPUT_RECORD, KEY_EVENT, PeekConsoleInputW,
    STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCP, SetConsoleMode,
    SetConsoleOutputCP,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
use windows_sys::Win32::System::Pipes::{
    CreatePipe, PIPE_NOWAIT, PIPE_WAIT, SetNamedPipeHandleState,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{Error, Interest, RawDescriptor, Readiness, Result, StandardStream};

pub struct Descriptor {
    pub(crate) inner: Resource,
}

pub(crate) enum Resource {
    File(Arc<File>),
    Socket(OwnedSocket),
    Standard(StandardStream),
}

unsafe extern "C" {
    fn __acrt_iob_func(index: u32) -> *mut libc::FILE;
}

pub fn initialize_console() {
    static INITIALIZED: Once = Once::new();
    INITIALIZED.call_once(|| {
        // SAFETY: these calls configure the attached console; redirected streams fail harmlessly.
        unsafe {
            SetConsoleCP(65001);
            SetConsoleOutputCP(65001);
            for stream in [StandardStream::Output, StandardStream::Error] {
                let mut mode = 0;
                if GetConsoleMode(stream.handle(), &raw mut mode) != 0 {
                    SetConsoleMode(stream.handle(), mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                }
            }
        }
    });
}

impl StandardStream {
    fn file(self) -> *mut libc::FILE {
        let index = match self {
            Self::Input => 0,
            Self::Output => 1,
            Self::Error => 2,
        };
        // SAFETY: the CRT owns these streams throughout the process lifetime.
        unsafe { __acrt_iob_func(index) }
    }

    pub(crate) fn handle(self) -> RawHandle {
        let index = match self {
            Self::Input => STD_INPUT_HANDLE,
            Self::Output => STD_OUTPUT_HANDLE,
            Self::Error => STD_ERROR_HANDLE,
        };
        // SAFETY: this query does not transfer ownership of the standard handle.
        unsafe { GetStdHandle(index) }
    }

    pub fn write_all_blocking(self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            let mut written = 0;
            // SAFETY: the standard handle stays open and bytes contains length readable bytes.
            if unsafe {
                WriteFile(
                    self.handle(),
                    bytes.as_ptr(),
                    length,
                    &raw mut written,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                thread::sleep(Duration::from_millis(1));
            } else {
                bytes = &bytes[written as usize..];
            }
        }
        Ok(())
    }
}

pub(crate) fn duplicate_handle(handle: RawHandle) -> io::Result<File> {
    let mut duplicate = ptr::null_mut();
    // SAFETY: DuplicateHandle validates handle and fills live output storage.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &raw mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: DuplicateHandle returned a new owned handle.
    Ok(unsafe { File::from_raw_handle(duplicate) })
}

fn pipe_information(handle: RawHandle) -> io::Result<FILE_PIPE_LOCAL_INFORMATION> {
    let mut information = FILE_PIPE_LOCAL_INFORMATION::default();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: the OS validates the handle and fills the supplied buffers.
    let result = unsafe {
        NtQueryInformationFile(
            handle,
            &raw mut status,
            (&raw mut information).cast(),
            u32::try_from(size_of_val(&information)).unwrap(),
            FilePipeLocalInformation,
        )
    };
    if result < 0 {
        // SAFETY: this function only translates a status code.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) }.cast_signed(),
        ));
    }
    Ok(information)
}

fn console_ready(handle: RawHandle) -> io::Result<bool> {
    let mut mode = 0;
    // SAFETY: GetConsoleMode validates the handle and fills mode.
    if unsafe { GetConsoleMode(handle, &raw mut mode) } == 0 {
        return Ok(true);
    }
    let mut count = 0;
    // SAFETY: the OS validates the input handle and fills count.
    if unsafe { GetNumberOfConsoleInputEvents(handle, &raw mut count) } == 0 {
        return Ok(true);
    }
    if count == 0 {
        return Ok(false);
    }
    let mut events = Vec::new();
    events
        .try_reserve_exact(count as usize)
        .map_err(io::Error::other)?;
    events.resize(count as usize, INPUT_RECORD::default());
    // SAFETY: events contains count writable input records.
    if unsafe { PeekConsoleInputW(handle, events.as_mut_ptr(), count, &raw mut count) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(events[..count as usize].iter().any(|event| {
        if u32::from(event.EventType) != KEY_EVENT {
            return false;
        }
        // SAFETY: EventType identifies the KeyEvent union member.
        let key = unsafe { event.Event.KeyEvent };
        // SAFETY: PeekConsoleInputW supplies UnicodeChar in the key-event union.
        key.bKeyDown != 0
            && (mode & ENABLE_LINE_INPUT == 0 || unsafe { key.uChar.UnicodeChar } == 13)
    }))
}

impl Descriptor {
    pub(crate) fn from_socket(socket: socket2::Socket) -> Self {
        Self::socket(socket.into())
    }

    pub(crate) fn with_socket<T>(
        &self,
        operation: impl FnOnce(&socket2::Socket) -> io::Result<T>,
    ) -> io::Result<T> {
        match &self.inner {
            Resource::Socket(socket) => operation(&SockRef::from(socket)),
            _ => Err(io::Error::from_raw_os_error(ws::WSAENOTSOCK)),
        }
    }

    pub fn from_file(file: File) -> Self {
        Self {
            inner: Resource::File(Arc::new(file)),
        }
    }

    pub(crate) fn socket(socket: OwnedSocket) -> Self {
        Self {
            inner: Resource::Socket(socket),
        }
    }

    pub fn standard(stream: StandardStream) -> Result<Self> {
        let descriptor = Self {
            inner: Resource::Standard(stream),
        };
        descriptor.set_non_blocking(true)?;
        Ok(descriptor)
    }

    pub fn number(&self) -> i64 {
        match &self.inner {
            Resource::File(file) => i64::try_from(file.as_raw_handle() as usize).unwrap(),
            Resource::Socket(socket) => i64::try_from(socket.as_raw_socket()).unwrap(),
            Resource::Standard(stream) => i64::try_from(stream.handle() as usize).unwrap_or(0),
        }
    }

    pub(crate) fn handle(&self) -> io::Result<RawHandle> {
        match &self.inner {
            Resource::File(file) => Ok(file.as_raw_handle()),
            Resource::Standard(stream) => Ok(stream.handle()),
            Resource::Socket(_) => Err(io::Error::from_raw_os_error(6)),
        }
    }

    pub fn try_clone_file(&self) -> Result<File> {
        let result = match &self.inner {
            Resource::File(file) => file.try_clone(),
            Resource::Standard(stream) => duplicate_handle(stream.handle()),
            Resource::Socket(_) => Err(io::Error::from_raw_os_error(6)),
        };
        result.map_err(|e| Error::new("DuplicateHandle", e))
    }

    pub fn file_for_lock(&self) -> Result<Arc<File>> {
        match &self.inner {
            Resource::File(file) => Ok(Arc::clone(file)),
            _ => self.try_clone_file().map(Arc::new),
        }
    }

    pub fn duplicate(number: i64) -> Result<Self> {
        let number =
            usize::try_from(number).map_err(|_| Error::bad_descriptor("DuplicateHandle"))?;
        if number == 0 || number == usize::MAX {
            return Err(Error::bad_descriptor("DuplicateHandle"));
        }
        let mut information = ws::WSAPROTOCOL_INFOW::default();
        // SAFETY: Winsock validates number and fills information without closing the original.
        if unsafe { ws::WSADuplicateSocketW(number, process::id(), &raw mut information) } == 0 {
            // SAFETY: information came from WSADuplicateSocketW and remains alive through the call.
            let socket = unsafe {
                ws::WSASocketW(
                    ws::FROM_PROTOCOL_INFO,
                    ws::FROM_PROTOCOL_INFO,
                    ws::FROM_PROTOCOL_INFO,
                    &raw const information,
                    0,
                    ws::WSA_FLAG_OVERLAPPED | ws::WSA_FLAG_NO_HANDLE_INHERIT,
                )
            };
            if socket == ws::INVALID_SOCKET {
                // SAFETY: WSAGetLastError reads only the current thread's error.
                return Err(Error::new(
                    "WSASocketW",
                    io::Error::from_raw_os_error(unsafe { ws::WSAGetLastError() }),
                ));
            }
            // SAFETY: WSASocketW returned a new owned socket.
            return Ok(Self::socket(unsafe {
                OwnedSocket::from_raw_socket(socket as u64)
            }));
        }
        duplicate_handle(number as RawHandle)
            .map(Self::from_file)
            .map_err(|e| Error::new("DuplicateHandle", e))
    }

    pub fn set_non_blocking(&self, enabled: bool) -> Result<()> {
        self.non_blocking(enabled)
            .map_err(|e| Error::new("set_non_blocking", e))
    }

    fn non_blocking(&self, enabled: bool) -> io::Result<()> {
        if let Resource::Socket(socket) = &self.inner {
            return SockRef::from(socket).set_nonblocking(enabled);
        }
        let handle = self.handle()?;
        // SAFETY: GetFileType validates the handle.
        if unsafe { GetFileType(handle) } == FILE_TYPE_PIPE {
            let mode = if enabled { PIPE_NOWAIT } else { PIPE_WAIT };
            // SAFETY: mode is a valid pipe wait mode and outlives the call.
            if unsafe { SetNamedPipeHandleState(handle, &raw const mode, ptr::null(), ptr::null()) }
                == 0
            {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::PermissionDenied
                    || pipe_information(handle).is_err()
                {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn read(&self, maximum: usize) -> Result<Option<Vec<u8>>> {
        self.read_bytes(maximum.min(i32::MAX as usize))
            .map_err(|e| Error::new("read", e))
    }

    fn read_bytes(&self, maximum: usize) -> io::Result<Option<Vec<u8>>> {
        if !matches!(self.inner, Resource::Socket(_)) && !self.ready(Interest::Readable)? {
            return Ok(None);
        }
        let mut bytes = Vec::<u8>::new();
        bytes.try_reserve_exact(maximum).map_err(io::Error::other)?;
        let count = if let Resource::Socket(socket) = &self.inner {
            match SockRef::from(socket).recv(&mut bytes.spare_capacity_mut()[..maximum]) {
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) => return Err(error),
            }
        } else {
            let mut count = 0;
            // SAFETY: bytes has maximum writable spare bytes and the handle remains open.
            if unsafe {
                ReadFile(
                    self.handle()?,
                    bytes.as_mut_ptr(),
                    u32::try_from(maximum).unwrap(),
                    &raw mut count,
                    ptr::null_mut(),
                )
            } == 0
            {
                let error = io::Error::last_os_error();
                match error.raw_os_error().map(i32::cast_unsigned) {
                    Some(ERROR_NO_DATA) => return Ok(None),
                    Some(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED) => {
                        return Ok(Some(Vec::new()));
                    }
                    _ => return Err(error),
                }
            }
            count as usize
        };
        // SAFETY: the read initialized exactly count bytes within capacity.
        unsafe { bytes.set_len(count) };
        Ok(Some(bytes))
    }

    pub fn write(&self, bytes: &[u8]) -> Result<usize> {
        self.write_bytes(bytes).map_err(|e| Error::new("write", e))
    }

    fn write_bytes(&self, bytes: &[u8]) -> io::Result<usize> {
        let bytes = &bytes[..bytes.len().min(i32::MAX as usize)];
        if let Resource::Socket(socket) = &self.inner {
            return match SockRef::from(socket).send(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(0),
                result => result,
            };
        }
        let mut count = 0;
        // SAFETY: the handle stays open and bytes is readable for the supplied length.
        if unsafe {
            WriteFile(
                self.handle()?,
                bytes.as_ptr(),
                u32::try_from(bytes.len()).unwrap(),
                &raw mut count,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_DATA.cast_signed()) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            return Err(error);
        }
        Ok(count as usize)
    }

    pub fn flush(&self) -> Result<()> {
        if let Resource::Standard(stream) = &self.inner {
            // SAFETY: the CRT owns the standard stream.
            if unsafe { libc::fflush(stream.file()) } != 0 {
                return Err(Error::last("fflush"));
            }
        }
        Ok(())
    }

    pub fn pipe() -> Result<(Self, Self)> {
        let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
        // SAFETY: both outputs are live; the returned handles are not inheritable.
        if unsafe { CreatePipe(&raw mut read, &raw mut write, ptr::null(), 0) } == 0 {
            return Err(Error::last("CreatePipe"));
        }
        // SAFETY: CreatePipe returned two distinct owned handles.
        Ok(unsafe {
            (
                Self::from_file(File::from_raw_handle(read)),
                Self::from_file(File::from_raw_handle(write)),
            )
        })
    }

    pub fn socket_pair() -> Result<(Self, Self)> {
        let pair = || -> io::Result<_> {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let first = TcpStream::connect(listener.local_addr()?)?;
            let (second, peer) = listener.accept()?;
            if peer != first.local_addr()? {
                return Err(io::Error::other("unexpected socket-pair peer"));
            }
            first.set_nodelay(true)?;
            second.set_nodelay(true)?;
            first.set_nonblocking(true)?;
            second.set_nonblocking(true)?;
            Ok((first, second))
        };
        let (first, second) = pair().map_err(|e| Error::new("socketpair", e))?;
        Ok((Self::socket(first.into()), Self::socket(second.into())))
    }

    pub fn readiness(&self) -> Readiness {
        match &self.inner {
            Resource::Socket(socket) => {
                Readiness::Descriptor(RawDescriptor::Socket(socket.as_raw_socket()))
            }
            _ => Readiness::Poll(Duration::from_millis(1)),
        }
    }

    pub fn is_ready(&self, interest: Interest) -> Result<bool> {
        self.ready(interest).map_err(|e| Error::new("poll", e))
    }

    fn ready(&self, interest: Interest) -> io::Result<bool> {
        let handle = self.handle()?;
        // SAFETY: GetFileType validates the handle.
        match unsafe { GetFileType(handle) } {
            FILE_TYPE_PIPE => match pipe_information(handle) {
                Ok(info) => Ok(info.NamedPipeState != FILE_PIPE_CONNECTED_STATE
                    || match interest {
                        Interest::Readable => info.ReadDataAvailable != 0,
                        Interest::Writable => info.WriteQuotaAvailable != 0,
                        Interest::ReadableOrWritable => {
                            info.ReadDataAvailable != 0 || info.WriteQuotaAvailable != 0
                        }
                    }),
                Err(error)
                    if matches!(
                        error.raw_os_error().map(i32::cast_unsigned),
                        Some(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED)
                    ) =>
                {
                    Ok(true)
                }
                Err(error)
                    if error.kind() == io::ErrorKind::PermissionDenied
                        && interest == Interest::Writable =>
                {
                    Ok(true)
                }
                Err(error) => Err(error),
            },
            FILE_TYPE_CHAR if interest == Interest::Readable => console_ready(handle),
            _ => Ok(true),
        }
    }
}
