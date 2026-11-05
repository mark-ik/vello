// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opt-in observations; clocks and readback readiness do not qualify frames.
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub(crate) mod gpu;
pub use gpu::{GpuReport, GpuState, GpuStatus};

pub type CpuClock = fn() -> Option<Duration>;

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkSpan {
    pub wall: Duration,
    pub thread_cpu: Option<Duration>,
    pub calls: u64,
}

impl WorkSpan {
    pub(crate) fn add(&mut self, sample: WorkSpan) {
        if sample.calls == 0 {
            return;
        }
        self.wall += sample.wall;
        self.thread_cpu = if self.calls == 0 {
            sample.thread_cpu
        } else {
            self.thread_cpu.zip(sample.thread_cpu).map(|(a, b)| a + b)
        };
        self.calls += sample.calls;
    }
}

pub(crate) struct Sample {
    wall: Option<Instant>,
    cpu: Option<Duration>,
    clock: Option<CpuClock>,
}

impl Sample {
    pub(crate) fn start(clock: Option<CpuClock>) -> Self {
        Self {
            wall: clock.map(|_| Instant::now()),
            cpu: clock.and_then(|f| f()),
            clock,
        }
    }
    pub(crate) fn finish(self) -> WorkSpan {
        let cpu = self
            .clock
            .and_then(|f| f())
            .zip(self.cpu)
            .and_then(|(end, start)| end.checked_sub(start));
        self.wall.map_or_else(WorkSpan::default, |wall| WorkSpan {
            wall: wall.elapsed(),
            thread_cpu: cpu,
            calls: 1,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct QualityObservation {
    pub available: bool,
    pub driver_poll: WorkSpan,
    /// Already-mapped range read, parse, unmap, free and counter validation.
    pub map: WorkSpan,
    /// Map request to callback wall time; overlaps GPU and driver observation.
    /// No cross-thread CPU duration is inferred from this interval.
    pub map_ready_wall: Option<Duration>,
    pub map_callbacks: u64,
    pub mapped_buffers: u64,
    pub mapped_bytes: u64,
    pub failed_readbacks: u64,
}

#[derive(Clone)]
pub(crate) struct ReadbackReady {
    requested: Instant,
    ready: Arc<Mutex<Option<Instant>>>,
}

impl ReadbackReady {
    pub(crate) fn new() -> Self {
        Self {
            requested: Instant::now(),
            ready: Arc::new(Mutex::new(None)),
        }
    }
    pub(crate) fn notify(&self) {
        *self.ready.lock().expect("diagnostic readiness poisoned") = Some(Instant::now());
    }
    pub(crate) fn elapsed(&self) -> Option<Duration> {
        self.ready
            .lock()
            .expect("diagnostic readiness poisoned")
            .and_then(|at| at.checked_duration_since(self.requested))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_cpu_is_preserved_and_never_forced_to_match_wall() {
        let mut span = WorkSpan::default();
        span.add(WorkSpan {
            wall: Duration::from_nanos(7),
            thread_cpu: Some(Duration::from_nanos(3)),
            calls: 1,
        });
        span.add(WorkSpan {
            wall: Duration::from_nanos(11),
            thread_cpu: None,
            calls: 1,
        });
        assert_eq!(span.wall, Duration::from_nanos(18));
        assert_eq!(span.thread_cpu, None);
        assert_eq!(span.calls, 2);
        let cpu_larger = WorkSpan {
            wall: Duration::from_nanos(1),
            thread_cpu: Some(Duration::from_nanos(2)),
            calls: 1,
        };
        assert_eq!(
            cpu_larger.wall.checked_sub(cpu_larger.thread_cpu.unwrap()),
            None
        );
    }
    #[test]
    fn map_readiness_is_separate_from_synchronous_map_cpu() {
        let ready = ReadbackReady::new();
        assert_eq!(ready.elapsed(), None);
        ready.notify();
        assert!(ready.elapsed().is_some());
    }
}
