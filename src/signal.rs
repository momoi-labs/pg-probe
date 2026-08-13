use std::io;
use std::ptr;

use pg_probe::CancelFlag;

static CANCEL: CancelFlag = CancelFlag::new();

pub fn flag() -> &'static CancelFlag {
    &CANCEL
}

extern "C" fn handle(signal: libc::c_int) {
    CANCEL.request(signal);
}

/// Installs a cooperative handler for `SIGINT` only.
///
/// The handler requests cancellation rather than terminating the process, so
/// the capture pipeline can finalize whatever it has collected before exiting.
/// `SA_RESTART` keeps in-flight reads from being aborted mid-collection;
/// cancellation is observed at the pipeline's checkpoints instead.
///
/// Only the first `SIGINT` requests cancellation; subsequent signals are
/// silently ignored because [`CancelFlag::request`] is a first-writer-wins
/// atomic store.
pub fn install() -> io::Result<()> {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handle as *const () as libc::sighandler_t;
    action.sa_flags = libc::SA_RESTART;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    let result = unsafe { libc::sigaction(libc::SIGINT, &action, ptr::null_mut()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
