//! Platform constants exposed by name so Whim code never hard-codes ABI values.

use whim_macros::whim_constant;

#[whim_constant("Whim\\_Private\\ERRNO_NOT_FOUND", "int")]
pub(crate) const ERRNO_NOT_FOUND: i64 = libc::ENOENT as i64;

#[whim_constant("Whim\\_Private\\ERRNO_NOT_DIRECTORY", "int")]
pub(crate) const ERRNO_NOT_DIRECTORY: i64 = libc::ENOTDIR as i64;

#[whim_constant("Whim\\_Private\\ERRNO_IS_DIRECTORY", "int")]
pub(crate) const ERRNO_IS_DIRECTORY: i64 = libc::EISDIR as i64;

#[whim_constant("Whim\\_Private\\ERRNO_ALREADY_EXISTS", "int")]
pub(crate) const ERRNO_ALREADY_EXISTS: i64 = libc::EEXIST as i64;

#[whim_constant("Whim\\_Private\\ERRNO_PERMISSION_DENIED", "int")]
pub(crate) const ERRNO_PERMISSION_DENIED: i64 = libc::EACCES as i64;

#[whim_constant("Whim\\_Private\\ERRNO_OPERATION_NOT_PERMITTED", "int")]
pub(crate) const ERRNO_OPERATION_NOT_PERMITTED: i64 = libc::EPERM as i64;

#[whim_constant("Whim\\_Private\\ERRNO_DIRECTORY_NOT_EMPTY", "int")]
pub(crate) const ERRNO_DIRECTORY_NOT_EMPTY: i64 = libc::ENOTEMPTY as i64;

#[whim_constant("Whim\\_Private\\ERRNO_INVALID_ARGUMENT", "int")]
pub(crate) const ERRNO_INVALID_ARGUMENT: i64 = libc::EINVAL as i64;

#[whim_constant("Whim\\_Private\\ERRNO_WOULD_BLOCK", "int")]
pub(crate) const ERRNO_WOULD_BLOCK: i64 = libc::EAGAIN as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_MASK", "int")]
pub(crate) const FILE_TYPE_MASK: i64 = libc::S_IFMT as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_REGULAR", "int")]
pub(crate) const FILE_TYPE_REGULAR: i64 = libc::S_IFREG as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_DIRECTORY", "int")]
pub(crate) const FILE_TYPE_DIRECTORY: i64 = libc::S_IFDIR as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_SYMBOLIC_LINK", "int")]
pub(crate) const FILE_TYPE_SYMBOLIC_LINK: i64 = libc::S_IFLNK as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_CHARACTER_DEVICE", "int")]
pub(crate) const FILE_TYPE_CHARACTER_DEVICE: i64 = libc::S_IFCHR as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_BLOCK_DEVICE", "int")]
pub(crate) const FILE_TYPE_BLOCK_DEVICE: i64 = libc::S_IFBLK as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_NAMED_PIPE", "int")]
pub(crate) const FILE_TYPE_NAMED_PIPE: i64 = libc::S_IFIFO as i64;

#[whim_constant("Whim\\_Private\\FILE_TYPE_SOCKET", "int")]
pub(crate) const FILE_TYPE_SOCKET: i64 = libc::S_IFSOCK as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_WRITE_ONLY", "int")]
pub(crate) const OPEN_FLAG_WRITE_ONLY: i64 = libc::O_WRONLY as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_CREATE", "int")]
pub(crate) const OPEN_FLAG_CREATE: i64 = libc::O_CREAT as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_EXCLUSIVE", "int")]
pub(crate) const OPEN_FLAG_EXCLUSIVE: i64 = libc::O_EXCL as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_CLOSE_ON_EXEC", "int")]
pub(crate) const OPEN_FLAG_CLOSE_ON_EXEC: i64 = libc::O_CLOEXEC as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_READ_ONLY", "int")]
pub(crate) const OPEN_FLAG_READ_ONLY: i64 = libc::O_RDONLY as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_READ_WRITE", "int")]
pub(crate) const OPEN_FLAG_READ_WRITE: i64 = libc::O_RDWR as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_APPEND", "int")]
pub(crate) const OPEN_FLAG_APPEND: i64 = libc::O_APPEND as i64;

#[whim_constant("Whim\\_Private\\OPEN_FLAG_TRUNCATE", "int")]
pub(crate) const OPEN_FLAG_TRUNCATE: i64 = libc::O_TRUNC as i64;

#[whim_constant("Whim\\_Private\\SEEK_ORIGIN_START", "int")]
pub(crate) const SEEK_ORIGIN_START: i64 = libc::SEEK_SET as i64;

#[whim_constant("Whim\\_Private\\SEEK_ORIGIN_CURRENT", "int")]
pub(crate) const SEEK_ORIGIN_CURRENT: i64 = libc::SEEK_CUR as i64;

#[whim_constant("Whim\\_Private\\SEEK_ORIGIN_END", "int")]
pub(crate) const SEEK_ORIGIN_END: i64 = libc::SEEK_END as i64;

#[whim_constant("Whim\\_Private\\LOCK_KIND_SHARED", "int")]
pub(crate) const LOCK_KIND_SHARED: i64 = libc::LOCK_SH as i64;

#[whim_constant("Whim\\_Private\\LOCK_KIND_EXCLUSIVE", "int")]
pub(crate) const LOCK_KIND_EXCLUSIVE: i64 = libc::LOCK_EX as i64;

#[whim_constant("Whim\\_Private\\LOCK_KIND_RELEASE", "int")]
pub(crate) const LOCK_KIND_RELEASE: i64 = libc::LOCK_UN as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_HANGUP", "int")]
pub(crate) const SIGNAL_HANGUP: i64 = libc::SIGHUP as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_INTERRUPT", "int")]
pub(crate) const SIGNAL_INTERRUPT: i64 = libc::SIGINT as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_QUIT", "int")]
pub(crate) const SIGNAL_QUIT: i64 = libc::SIGQUIT as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_ILLEGAL_INSTRUCTION", "int")]
pub(crate) const SIGNAL_ILLEGAL_INSTRUCTION: i64 = libc::SIGILL as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_TRAP", "int")]
pub(crate) const SIGNAL_TRAP: i64 = libc::SIGTRAP as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_ABORT", "int")]
pub(crate) const SIGNAL_ABORT: i64 = libc::SIGABRT as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_BUS_ERROR", "int")]
pub(crate) const SIGNAL_BUS_ERROR: i64 = libc::SIGBUS as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_FLOATING_POINT_EXCEPTION", "int")]
pub(crate) const SIGNAL_FLOATING_POINT_EXCEPTION: i64 = libc::SIGFPE as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_KILL", "int")]
pub(crate) const SIGNAL_KILL: i64 = libc::SIGKILL as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_USER_DEFINED_1", "int")]
pub(crate) const SIGNAL_USER_DEFINED_1: i64 = libc::SIGUSR1 as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_SEGMENTATION_FAULT", "int")]
pub(crate) const SIGNAL_SEGMENTATION_FAULT: i64 = libc::SIGSEGV as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_USER_DEFINED_2", "int")]
pub(crate) const SIGNAL_USER_DEFINED_2: i64 = libc::SIGUSR2 as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_BROKEN_PIPE", "int")]
pub(crate) const SIGNAL_BROKEN_PIPE: i64 = libc::SIGPIPE as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_ALARM", "int")]
pub(crate) const SIGNAL_ALARM: i64 = libc::SIGALRM as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_TERMINATE", "int")]
pub(crate) const SIGNAL_TERMINATE: i64 = libc::SIGTERM as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_CHILD", "int")]
pub(crate) const SIGNAL_CHILD: i64 = libc::SIGCHLD as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_CONTINUE", "int")]
pub(crate) const SIGNAL_CONTINUE: i64 = libc::SIGCONT as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_STOP", "int")]
pub(crate) const SIGNAL_STOP: i64 = libc::SIGSTOP as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_TERMINAL_STOP", "int")]
pub(crate) const SIGNAL_TERMINAL_STOP: i64 = libc::SIGTSTP as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_TERMINAL_INPUT", "int")]
pub(crate) const SIGNAL_TERMINAL_INPUT: i64 = libc::SIGTTIN as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_TERMINAL_OUTPUT", "int")]
pub(crate) const SIGNAL_TERMINAL_OUTPUT: i64 = libc::SIGTTOU as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_URGENT", "int")]
pub(crate) const SIGNAL_URGENT: i64 = libc::SIGURG as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_CPU_TIME_LIMIT_EXCEEDED", "int")]
pub(crate) const SIGNAL_CPU_TIME_LIMIT_EXCEEDED: i64 = libc::SIGXCPU as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_FILE_SIZE_LIMIT_EXCEEDED", "int")]
pub(crate) const SIGNAL_FILE_SIZE_LIMIT_EXCEEDED: i64 = libc::SIGXFSZ as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_VIRTUAL_ALARM", "int")]
pub(crate) const SIGNAL_VIRTUAL_ALARM: i64 = libc::SIGVTALRM as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_PROFILING_TIMER_EXPIRED", "int")]
pub(crate) const SIGNAL_PROFILING_TIMER_EXPIRED: i64 = libc::SIGPROF as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_WINDOW_CHANGE", "int")]
pub(crate) const SIGNAL_WINDOW_CHANGE: i64 = libc::SIGWINCH as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_INPUT_OUTPUT", "int")]
pub(crate) const SIGNAL_INPUT_OUTPUT: i64 = libc::SIGIO as i64;

#[whim_constant("Whim\\_Private\\SIGNAL_BAD_SYSTEM_CALL", "int")]
pub(crate) const SIGNAL_BAD_SYSTEM_CALL: i64 = libc::SIGSYS as i64;

#[cfg(target_os = "linux")]
#[whim_constant("Whim\\_Private\\SIGNAL_STACK_FAULT", "int")]
pub(crate) const SIGNAL_STACK_FAULT: i64 = libc::SIGSTKFLT as i64;

#[cfg(target_os = "macos")]
#[whim_constant("Whim\\_Private\\SIGNAL_STACK_FAULT", "int")]
pub(crate) const SIGNAL_STACK_FAULT: i64 = -1;

#[cfg(target_os = "linux")]
#[whim_constant("Whim\\_Private\\SIGNAL_POWER_FAILURE", "int")]
pub(crate) const SIGNAL_POWER_FAILURE: i64 = libc::SIGPWR as i64;

#[cfg(target_os = "macos")]
#[whim_constant("Whim\\_Private\\SIGNAL_POWER_FAILURE", "int")]
pub(crate) const SIGNAL_POWER_FAILURE: i64 = -2;

#[whim_constant("Whim\\_Private\\RESOURCE_LIMIT_CPU_TIME", "int")]
pub(crate) const RESOURCE_LIMIT_CPU_TIME: i64 = libc::RLIMIT_CPU as i64;

#[whim_constant("Whim\\_Private\\RESOURCE_LIMIT_FILE_SIZE", "int")]
pub(crate) const RESOURCE_LIMIT_FILE_SIZE: i64 = libc::RLIMIT_FSIZE as i64;

#[whim_constant("Whim\\_Private\\RESOURCE_LIMIT_DATA", "int")]
pub(crate) const RESOURCE_LIMIT_DATA: i64 = libc::RLIMIT_DATA as i64;

#[whim_constant("Whim\\_Private\\RESOURCE_LIMIT_STACK", "int")]
pub(crate) const RESOURCE_LIMIT_STACK: i64 = libc::RLIMIT_STACK as i64;

#[whim_constant("Whim\\_Private\\RESOURCE_LIMIT_CORE", "int")]
pub(crate) const RESOURCE_LIMIT_CORE: i64 = libc::RLIMIT_CORE as i64;

#[whim_constant("Whim\\_Private\\RESOURCE_LIMIT_OPEN_FILES", "int")]
pub(crate) const RESOURCE_LIMIT_OPEN_FILES: i64 = libc::RLIMIT_NOFILE as i64;

#[whim_constant("Whim\\_Private\\RESOURCE_LIMIT_ADDRESS_SPACE", "int")]
pub(crate) const RESOURCE_LIMIT_ADDRESS_SPACE: i64 = libc::RLIMIT_AS as i64;

#[whim_constant("Whim\\_Private\\STREAM_DISPOSITION_INHERIT", "int")]
pub(crate) const STREAM_DISPOSITION_INHERIT: i64 = 0;

#[whim_constant("Whim\\_Private\\STREAM_DISPOSITION_NULL", "int")]
pub(crate) const STREAM_DISPOSITION_NULL: i64 = 1;

#[whim_constant("Whim\\_Private\\STREAM_DISPOSITION_PIPE", "int")]
pub(crate) const STREAM_DISPOSITION_PIPE: i64 = 2;

#[whim_constant("Whim\\_Private\\STREAM_DISPOSITION_DESCRIPTOR", "int")]
pub(crate) const STREAM_DISPOSITION_DESCRIPTOR: i64 = 3;

#[whim_constant("Whim\\_Private\\STREAM_DISPOSITION_TERMINAL", "int")]
pub(crate) const STREAM_DISPOSITION_TERMINAL: i64 = 4;

#[whim_constant("Whim\\_Private\\SOCKET_FAMILY_INET", "int")]
pub(crate) const SOCKET_FAMILY_INET: i64 = libc::AF_INET as i64;

#[whim_constant("Whim\\_Private\\SOCKET_FAMILY_INET6", "int")]
pub(crate) const SOCKET_FAMILY_INET6: i64 = libc::AF_INET6 as i64;

#[whim_constant("Whim\\_Private\\SOCKET_FAMILY_UNIX", "int")]
pub(crate) const SOCKET_FAMILY_UNIX: i64 = libc::AF_UNIX as i64;

#[whim_constant("Whim\\_Private\\SOCKET_KIND_STREAM", "int")]
pub(crate) const SOCKET_KIND_STREAM: i64 = libc::SOCK_STREAM as i64;

#[whim_constant("Whim\\_Private\\SOCKET_KIND_DATAGRAM", "int")]
pub(crate) const SOCKET_KIND_DATAGRAM: i64 = libc::SOCK_DGRAM as i64;

#[whim_constant("Whim\\_Private\\SOCKET_LEVEL_SOCKET", "int")]
pub(crate) const SOCKET_LEVEL_SOCKET: i64 = libc::SOL_SOCKET as i64;

#[whim_constant("Whim\\_Private\\SOCKET_LEVEL_TCP", "int")]
pub(crate) const SOCKET_LEVEL_TCP: i64 = libc::IPPROTO_TCP as i64;

#[whim_constant("Whim\\_Private\\SOCKET_LEVEL_INET6", "int")]
pub(crate) const SOCKET_LEVEL_INET6: i64 = libc::IPPROTO_IPV6 as i64;

#[whim_constant("Whim\\_Private\\SOCKET_OPTION_REUSE_ADDRESS", "int")]
pub(crate) const SOCKET_OPTION_REUSE_ADDRESS: i64 = libc::SO_REUSEADDR as i64;

#[whim_constant("Whim\\_Private\\SOCKET_OPTION_REUSE_PORT", "int")]
pub(crate) const SOCKET_OPTION_REUSE_PORT: i64 = libc::SO_REUSEPORT as i64;

#[whim_constant("Whim\\_Private\\SOCKET_OPTION_BROADCAST", "int")]
pub(crate) const SOCKET_OPTION_BROADCAST: i64 = libc::SO_BROADCAST as i64;

#[whim_constant("Whim\\_Private\\SOCKET_OPTION_RECEIVE_BUFFER", "int")]
pub(crate) const SOCKET_OPTION_RECEIVE_BUFFER: i64 = libc::SO_RCVBUF as i64;

#[whim_constant("Whim\\_Private\\SOCKET_OPTION_SEND_BUFFER", "int")]
pub(crate) const SOCKET_OPTION_SEND_BUFFER: i64 = libc::SO_SNDBUF as i64;

#[whim_constant("Whim\\_Private\\SOCKET_OPTION_NO_DELAY", "int")]
pub(crate) const SOCKET_OPTION_NO_DELAY: i64 = libc::TCP_NODELAY as i64;

#[whim_constant("Whim\\_Private\\SOCKET_OPTION_ONLY_INET6", "int")]
pub(crate) const SOCKET_OPTION_ONLY_INET6: i64 = libc::IPV6_V6ONLY as i64;
