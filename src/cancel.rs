use std::sync::atomic::{AtomicI32, Ordering};

/// A cooperative cancellation flag shared between an operating-system signal
/// handler and the capture pipeline.
///
/// The flag stores the number of the signal that requested cancellation. A
/// value of zero means the run has not been cancelled. Only a single atomic
/// store is performed on cancellation, which keeps [`CancelFlag::request`]
/// async-signal-safe so it can be called directly from a signal handler.
///
/// The first `request` wins (compare-exchange from 0); all subsequent calls
/// are no-ops. This guarantees that a second Ctrl+C has no effect.
#[derive(Debug)]
pub struct CancelFlag {
    signal: AtomicI32,
}

impl CancelFlag {
    pub const fn new() -> Self {
        Self {
            signal: AtomicI32::new(0),
        }
    }

    /// Records that cancellation was requested by the given signal number.
    /// The first request wins; subsequent calls are silently ignored.
    pub fn request(&self, signal: i32) {
        let signal = if signal == 0 { -1 } else { signal };
        let _ = self
            .signal
            .compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.signal.load(Ordering::SeqCst) != 0
    }

    pub fn signal(&self) -> Option<i32> {
        match self.signal.load(Ordering::SeqCst) {
            0 => None,
            signal => Some(signal),
        }
    }
}

impl Default for CancelFlag {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_flag_is_not_cancelled() {
        let flag = CancelFlag::new();
        assert!(!flag.is_cancelled());
        assert_eq!(flag.signal(), None);
    }

    #[test]
    fn requesting_records_the_signal() {
        let flag = CancelFlag::new();
        flag.request(2);
        assert!(flag.is_cancelled());
        assert_eq!(flag.signal(), Some(2));
    }

    #[test]
    fn the_first_request_wins() {
        let flag = CancelFlag::new();
        flag.request(2);
        flag.request(15);
        assert_eq!(flag.signal(), Some(2));
    }

    #[test]
    fn second_and_third_signals_have_no_effect() {
        let flag = CancelFlag::new();
        flag.request(2);
        flag.request(2);
        flag.request(2);
        assert_eq!(flag.signal(), Some(2));
    }
}
