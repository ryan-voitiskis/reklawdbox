//! Platform-specific primitives for managed Essentia lifecycle operations.

use std::fs::File;
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub(super) enum AtomicExchangeError {
    InvalidPath(&'static str),
    Io(std::io::Error),
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Unsupported(&'static str),
}

impl std::fmt::Display for AtomicExchangeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath(message) => formatter.write_str(message),
            Self::Io(error) => error.fmt(formatter),
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            Self::Unsupported(message) => formatter.write_str(message),
        }
    }
}

#[cfg(unix)]
pub(super) fn output_ready(
    descriptor: std::os::fd::RawFd,
    timeout: Duration,
) -> std::io::Result<bool> {
    let timeout_ms = i32::try_from(timeout.as_millis().max(1)).unwrap_or(i32::MAX);
    let mut descriptor = libc::pollfd {
        fd: descriptor,
        events: libc::POLLIN | libc::POLLHUP,
        revents: 0,
    };
    // SAFETY: `descriptor` references one valid pollfd for the duration of the
    // call. Its owning reader retains the underlying descriptor until return.
    let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
    if ready >= 0 {
        Ok(ready > 0)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
pub(super) fn output_ready(_descriptor: (), _timeout: Duration) -> std::io::Result<bool> {
    Ok(true)
}

#[cfg(target_os = "macos")]
pub(super) fn atomic_exchange_paths(left: &Path, right: &Path) -> Result<(), AtomicExchangeError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    unsafe extern "C" {
        fn renamex_np(
            from: *const libc::c_char,
            to: *const libc::c_char,
            flags: u32,
        ) -> libc::c_int;
    }
    const RENAME_SWAP: u32 = 0x0000_0002;
    let left = CString::new(left.as_os_str().as_bytes())
        .map_err(|_| AtomicExchangeError::InvalidPath("left exchange path contains a NUL byte"))?;
    let right = CString::new(right.as_os_str().as_bytes())
        .map_err(|_| AtomicExchangeError::InvalidPath("right exchange path contains a NUL byte"))?;
    // SAFETY: both C strings are valid for the duration of the call. The paths
    // are on the same managed filesystem and RENAME_SWAP preserves both names.
    let result = unsafe { renamex_np(left.as_ptr(), right.as_ptr(), RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(AtomicExchangeError::Io(std::io::Error::last_os_error()))
    }
}

#[cfg(target_os = "linux")]
pub(super) fn atomic_exchange_paths(left: &Path, right: &Path) -> Result<(), AtomicExchangeError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    const RENAME_EXCHANGE: libc::c_uint = 0x2;
    let left = CString::new(left.as_os_str().as_bytes())
        .map_err(|_| AtomicExchangeError::InvalidPath("left exchange path contains a NUL byte"))?;
    let right = CString::new(right.as_os_str().as_bytes())
        .map_err(|_| AtomicExchangeError::InvalidPath("right exchange path contains a NUL byte"))?;
    // SAFETY: arguments follow renameat2(2); both C strings remain valid for
    // the call and RENAME_EXCHANGE preserves both names atomically.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(AtomicExchangeError::Io(std::io::Error::last_os_error()))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn atomic_exchange_paths(
    _left: &Path,
    _right: &Path,
) -> Result<(), AtomicExchangeError> {
    Err(AtomicExchangeError::Unsupported(
        "atomic legacy-directory migration is unsupported on this platform",
    ))
}

pub(super) fn try_lock_exclusive(file: &File) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd as _;

    // SAFETY: `file` remains open for the call and flock only touches its
    // owned descriptor.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) || error.raw_os_error() == Some(libc::EAGAIN)
    {
        Ok(false)
    } else {
        Err(error)
    }
}

pub(super) fn unlock(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd as _;

    // SAFETY: this is the descriptor previously locked by this owner.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
