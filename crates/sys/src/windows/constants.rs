pub use libc::{
    EACCES, EAGAIN, EEXIST, EINVAL, EISDIR, ENOENT, ENOTDIR, ENOTEMPTY, EPERM, O_APPEND, O_CREAT,
    O_EXCL, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY, S_IFCHR, S_IFDIR, S_IFMT, S_IFREG, SEEK_CUR,
    SEEK_END, SEEK_SET,
};
pub use windows_sys::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, AF_UNIX, IPPROTO_IPV6, IPPROTO_TCP, IPV6_V6ONLY, SO_BROADCAST, SO_RCVBUF,
    SO_REUSEADDR, SO_SNDBUF, SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET, TCP_NODELAY,
};

pub const S_IFIFO: i32 = 0o010000;
pub const S_IFLNK: i32 = 0o120000;
pub const S_IFBLK: i32 = 0o060000;
pub const S_IFSOCK: i32 = 0o140000;
pub const O_CLOEXEC: i32 = libc::O_NOINHERIT;
pub const LOCK_SH: i32 = 1;
pub const LOCK_EX: i32 = 2;
pub const LOCK_UN: i32 = 8;
pub const SO_REUSEPORT: i32 = -1;
pub const RLIMIT_CPU: i32 = -1;
pub const RLIMIT_FSIZE: i32 = -2;
pub const RLIMIT_DATA: i32 = -3;
pub const RLIMIT_STACK: i32 = -4;
pub const RLIMIT_CORE: i32 = -5;
pub const RLIMIT_NOFILE: i32 = -6;
pub const RLIMIT_AS: i32 = -7;
pub const SIGSTKFLT: i32 = -1;
pub const SIGPWR: i32 = -2;
pub const SIGHUP: i32 = -3;
pub const SIGINT: i32 = -4;
pub const SIGQUIT: i32 = -5;
pub const SIGILL: i32 = -6;
pub const SIGTRAP: i32 = -7;
pub const SIGABRT: i32 = -8;
pub const SIGBUS: i32 = -9;
pub const SIGFPE: i32 = -10;
pub const SIGKILL: i32 = -11;
pub const SIGUSR1: i32 = -12;
pub const SIGSEGV: i32 = -13;
pub const SIGUSR2: i32 = -14;
pub const SIGPIPE: i32 = -15;
pub const SIGALRM: i32 = -16;
pub const SIGTERM: i32 = -17;
pub const SIGCHLD: i32 = -18;
pub const SIGCONT: i32 = -19;
pub const SIGSTOP: i32 = -20;
pub const SIGTSTP: i32 = -21;
pub const SIGTTIN: i32 = -22;
pub const SIGTTOU: i32 = -23;
pub const SIGURG: i32 = -24;
pub const SIGXCPU: i32 = -25;
pub const SIGXFSZ: i32 = -26;
pub const SIGVTALRM: i32 = -27;
pub const SIGPROF: i32 = -28;
pub const SIGWINCH: i32 = -29;
pub const SIGIO: i32 = -30;
pub const SIGSYS: i32 = -31;
