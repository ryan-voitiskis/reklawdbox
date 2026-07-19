//! Platform-specific primitives for managed Essentia lifecycle operations.

use std::time::Duration;

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
