//! "Close what is running", asked from outside: holding Start makes keyd
//! (as root) run `pkill -USR1 -x pockettui35`.
//!
//! The handler only sets a flag. It is installed without `SA_RESTART`, so a
//! launcher blocked in `waitpid` on an app wakes with `EINTR` and acts at
//! once. While a script runs, the main loop sees the flag within 50 ms.

use std::sync::atomic::{AtomicBool, Ordering};

static CLOSE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_usr1(_: libc::c_int) {
    CLOSE.store(true, Ordering::SeqCst);
}

/// Installs the SIGUSR1 handler. Call first thing: SIGUSR1's default action
/// would end the process.
pub fn install() {
    // SAFETY: a zeroed sigaction with a handler that only touches an atomic,
    // which is async-signal-safe.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_usr1 as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = 0; // no SA_RESTART: interrupt waitpid
        libc::sigaction(libc::SIGUSR1, &sa, std::ptr::null_mut());
    }
}

/// Blocks SIGUSR1 in the calling thread. Every helper thread calls this, so
/// the signal reaches the main thread and interrupts its `waitpid`.
pub fn block_in_this_thread() {
    // SAFETY: a signal set built with sigemptyset/sigaddset, for this thread.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGUSR1);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Tests that send SIGUSR1 or wait on apps hold this: the flag is global
/// and tests run in parallel.
#[cfg(test)]
pub static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes the request, if one came since the last call.
pub fn take_close() -> bool {
    CLOSE.swap(false, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usr1_sets_the_flag_once() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        install();
        take_close();
        // SAFETY: signals this test process, whose handler is installed.
        unsafe { libc::raise(libc::SIGUSR1) };
        assert!(take_close());
        assert!(!take_close());
    }
}
