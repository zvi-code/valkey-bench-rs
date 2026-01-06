//! Benchmark control for thread synchronization
//!
//! Provides progress tracking, duration control, and shutdown coordination.
//! Request counting is driven by iterator exhaustion from `keyspace_tracker`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Benchmark control shared between all worker threads.
///
/// Handles:
/// - Duration limits (time-based benchmarks)
/// - Progress tracking (for UI)
/// - Error counting
/// - Shutdown signaling
///
/// Note: Request counting is driven by iterator exhaustion from `keyspace_tracker`.
/// When `WorkloadContext::claim_next_id()` returns `None`, the worker stops.
pub struct GlobalCounters {
    /// Total requests issued (for progress tracking)
    requests_issued: AtomicU64,

    /// Total requests completed (responses received)
    requests_finished: AtomicU64,

    /// Total errors encountered
    error_count: AtomicU64,

    /// Shutdown signal
    shutdown: AtomicBool,

    /// Benchmark start time (for duration-based benchmarks)
    start_time: Option<Instant>,

    /// Duration limit (if set, benchmark runs until time expires)
    duration_limit: Option<Duration>,
}

impl GlobalCounters {
    /// Create new counters (unlimited mode - driven by iterator)
    pub fn new() -> Self {
        Self {
            requests_issued: AtomicU64::new(0),
            requests_finished: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            start_time: None,
            duration_limit: None,
        }
    }

    /// Create counters with a duration limit (time-based benchmarking)
    pub fn with_duration(duration_secs: u64) -> Self {
        Self {
            start_time: Some(Instant::now()),
            duration_limit: Some(Duration::from_secs(duration_secs)),
            ..Self::new()
        }
    }

    /// Check if duration has been exceeded (for time-based benchmarks)
    #[inline]
    pub fn is_duration_exceeded(&self) -> bool {
        if let (Some(start), Some(limit)) = (self.start_time, self.duration_limit) {
            start.elapsed() >= limit
        } else {
            false
        }
    }

    /// Check if running in duration mode
    #[inline]
    pub fn is_duration_mode(&self) -> bool {
        self.duration_limit.is_some()
    }

    /// Record that requests were issued (for progress tracking)
    #[inline]
    pub fn record_issued(&self, count: u64) {
        self.requests_issued.fetch_add(count, Ordering::Relaxed);
    }

    /// Record completed requests
    #[inline]
    pub fn record_finished(&self, count: u64) {
        self.requests_finished.fetch_add(count, Ordering::Relaxed);
    }

    /// Record an error
    #[inline]
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Signal shutdown to all workers
    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Check if shutdown has been signaled
    #[inline]
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// Get current progress (finished, issued)
    pub fn progress(&self) -> (u64, u64) {
        (
            self.requests_finished.load(Ordering::Relaxed),
            self.requests_issued.load(Ordering::Relaxed),
        )
    }

    /// Get error count
    pub fn errors(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    /// Reset counters (for warmup -> measurement transition)
    pub fn reset(&self) {
        self.requests_issued.store(0, Ordering::SeqCst);
        self.requests_finished.store(0, Ordering::SeqCst);
        self.error_count.store(0, Ordering::SeqCst);
    }
}

impl Default for GlobalCounters {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_record_issued() {
        let counters = GlobalCounters::new();

        counters.record_issued(10);
        counters.record_issued(20);

        let (_, issued) = counters.progress();
        assert_eq!(issued, 30);
    }

    #[test]
    fn test_concurrent_tracking() {
        let counters = Arc::new(GlobalCounters::new());

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = Arc::clone(&counters);
                thread::spawn(move || {
                    for _ in 0..100 {
                        c.record_issued(1);
                        c.record_finished(1);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let (finished, issued) = counters.progress();
        assert_eq!(issued, 400);
        assert_eq!(finished, 400);
    }

    #[test]
    fn test_shutdown_signal() {
        let counters = GlobalCounters::new();

        assert!(!counters.is_shutdown());
        counters.signal_shutdown();
        assert!(counters.is_shutdown());
    }

    #[test]
    fn test_progress() {
        let counters = GlobalCounters::new();

        counters.record_issued(50);
        counters.record_finished(25);

        let (finished, issued) = counters.progress();
        assert_eq!(finished, 25);
        assert_eq!(issued, 50);
    }

    #[test]
    fn test_reset() {
        let counters = GlobalCounters::new();

        counters.record_issued(50);
        counters.record_finished(25);
        counters.record_error();

        counters.reset();

        let (finished, issued) = counters.progress();
        assert_eq!(finished, 0);
        assert_eq!(issued, 0);
        assert_eq!(counters.errors(), 0);
    }

    #[test]
    fn test_duration_mode() {
        let counters = GlobalCounters::with_duration(1);
        
        assert!(counters.is_duration_mode());
        assert!(!counters.is_duration_exceeded());
    }

    #[test]
    fn test_errors() {
        let counters = GlobalCounters::new();
        
        counters.record_error();
        counters.record_error();
        counters.record_error();
        
        assert_eq!(counters.errors(), 3);
    }
}
