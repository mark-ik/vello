// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Optional timestamps around commands in the renderer's existing encoder.
//! Mapping is observed only by the existing completion worker, without polls
//! or waits here. GPU intervals overlap CPU and quality-readback intervals.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, mpsc};

const CAPACITY: usize = 256;
pub const GPU_SCOPE: &str = "existing provider command encoder: dispatches, copies, barriers and GPU stalls; excludes host encoding and later presentation";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuStatus {
    Disabled,
    Unsupported,
    NoRecording,
    Pending,
    Available,
    Failed,
    CapacityExceeded,
    HistoryExpired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuReport {
    pub batch_id: u64,
    pub recordings: u64,
    pub completed: u64,
    pub pending: u64,
    pub failed: u64,
    pub first_recording_id: Option<u64>,
    pub last_recording_id: Option<u64>,
    pub elapsed_ns: Option<u64>,
    pub timestamp_period_bits: Option<u32>,
    pub status: GpuStatus,
    pub scope: &'static str,
}

impl GpuReport {
    pub fn disabled(batch_id: u64) -> Self {
        Self {
            batch_id,
            recordings: 0,
            completed: 0,
            pending: 0,
            failed: 0,
            first_recording_id: None,
            last_recording_id: None,
            elapsed_ns: None,
            timestamp_period_bits: None,
            status: GpuStatus::Disabled,
            scope: GPU_SCOPE,
        }
    }
}

struct Readback {
    buffer: wgpu::Buffer,
    result: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

struct Recording {
    batch_id: u64,
    id: u64,
    period_bits: u32,
    status: GpuStatus,
    elapsed_ns: Option<u64>,
    readback: Option<Readback>,
}

/// Bounded metadata and probe buffers belonging to one renderer/device.
pub struct GpuState {
    enabled: bool,
    batch_id: u64,
    next_id: u64,
    records: VecDeque<Recording>,
    overflow_batch: Option<u64>,
    expired_through: Option<u64>,
}

impl GpuState {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            batch_id: 0,
            next_id: 1,
            records: VecDeque::new(),
            overflow_batch: None,
            expired_through: None,
        }
    }

    pub fn begin_batch(&mut self, batch_id: u64) {
        self.batch_id = batch_id;
    }

    fn mark_overflow(&mut self) {
        if let Some(previous) = self.overflow_batch.replace(self.batch_id) {
            if previous != self.batch_id {
                // Conservatively expire history through the previous skipped
                // batch. Replacing the marker must never expose a partial sum.
                self.expire_through(previous);
            }
        }
    }

    fn expire_through(&mut self, batch_id: u64) {
        self.expired_through = Some(
            self.expired_through
                .map_or(batch_id, |old| old.max(batch_id)),
        );
    }

    /// Called only inside the existing worker operation/error scopes. A map
    /// still pending stays pending; this method never polls the device.
    pub fn poll_ready(&mut self) {
        for record in &mut self.records {
            let Some(readback) = record.readback.as_ref() else {
                continue;
            };
            let result = match readback.result.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    record.status = GpuStatus::Failed;
                    record.readback = None;
                    continue;
                }
            };
            let readback = record.readback.take().expect("observed readback");
            if result.is_err() {
                record.status = GpuStatus::Failed;
                continue;
            }
            let bytes = readback.buffer.slice(..16).get_mapped_range();
            record.elapsed_ns = match bytes {
                Ok(bytes) => {
                    let start = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                    let end = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                    elapsed_ns(start, end, record.period_bits)
                }
                Err(_) => None,
            };
            readback.buffer.unmap();
            record.status = if record.elapsed_ns.is_some() {
                GpuStatus::Available
            } else {
                GpuStatus::Failed
            };
        }
    }

    /// Immutable CPU metadata only; no buffer reads, unmapping or retirement.
    pub fn snapshot(&self, batch_id: u64) -> GpuReport {
        let mut report = GpuReport::disabled(batch_id);
        report.status = if self.enabled {
            GpuStatus::NoRecording
        } else {
            GpuStatus::Unsupported
        };
        let mut total = Some(0u64);
        for record in self
            .records
            .iter()
            .filter(|record| record.batch_id == batch_id)
        {
            report.recordings += 1;
            report.first_recording_id.get_or_insert(record.id);
            report.last_recording_id = Some(record.id);
            report
                .timestamp_period_bits
                .get_or_insert(record.period_bits);
            if report.timestamp_period_bits != Some(record.period_bits) {
                total = None;
            }
            match record.status {
                GpuStatus::Available => {
                    report.completed += 1;
                    total = total.and_then(|value| value.checked_add(record.elapsed_ns?));
                }
                GpuStatus::Pending => {
                    report.pending += 1;
                }
                _ => {
                    report.failed += 1;
                }
            }
        }
        if self.overflow_batch == Some(batch_id) {
            report.status = GpuStatus::CapacityExceeded;
        } else if self
            .expired_through
            .is_some_and(|expired| batch_id <= expired)
        {
            report.status = GpuStatus::HistoryExpired;
        } else if report.failed > 0 || (report.recordings > 0 && total.is_none()) {
            report.status = GpuStatus::Failed;
        } else if report.pending > 0 {
            report.status = GpuStatus::Pending;
        } else if report.completed > 0 {
            report.status = GpuStatus::Available;
            report.elapsed_ns = total;
        }
        report
    }
}

fn elapsed_ns(start: u64, end: u64, period_bits: u32) -> Option<u64> {
    let period = f32::from_bits(period_bits);
    if !period.is_finite() || period <= 0.0 {
        return None;
    }
    let value = end.checked_sub(start)? as f64 * f64::from(period);
    if !value.is_finite() || value < 0.0 || value >= u64::MAX as f64 {
        return None;
    }
    Some(value.round() as u64)
}

pub(crate) struct GpuMarker {
    state: Arc<Mutex<GpuState>>,
    id: u64,
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
}

/// Base TIMESTAMP_QUERY uses pass boundaries, without INSIDE_ENCODERS.
pub(crate) fn begin(
    state: &Option<Arc<Mutex<GpuState>>>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
) -> Option<GpuMarker> {
    let shared = state.as_ref()?;
    let mut state = shared.lock().expect("GPU diagnostic state poisoned");
    if !state.enabled || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        state.enabled = false;
        return None;
    }
    while state.records.len() >= CAPACITY {
        let index = state
            .records
            .iter()
            .position(|record| record.readback.is_none() && record.batch_id != state.batch_id);
        let Some(index) = index else {
            state.mark_overflow();
            return None;
        };
        let old = state.records.remove(index).unwrap();
        state.expire_through(old.batch_id);
    }
    let id = state.next_id;
    let Some(next) = id.checked_add(1) else {
        state.mark_overflow();
        return None;
    };
    state.next_id = next;
    let batch_id = state.batch_id;
    state.records.push_back(Recording {
        batch_id,
        id,
        period_bits: queue.get_timestamp_period().to_bits(),
        status: GpuStatus::Failed,
        elapsed_ns: None,
        readback: None,
    });
    drop(state);
    let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("native diagnostic GPU bracket"),
        ty: wgpu::QueryType::Timestamp,
        count: 2,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("native diagnostic GPU resolve"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("native diagnostic GPU readback"),
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("native diagnostic GPU begin"),
        timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
            query_set: &queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: None,
        }),
    }));
    Some(GpuMarker {
        state: shared.clone(),
        id,
        queries,
        resolve,
        readback,
    })
}

impl GpuMarker {
    pub(crate) fn end(&self, encoder: &mut wgpu::CommandEncoder) {
        drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("native diagnostic GPU end"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(1),
            }),
        }));
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, 16);
    }

    /// Register after the original submission. Callback only sends readiness;
    /// the completion worker owns all mapped-range reads and unmapping.
    pub(crate) fn submitted(self) {
        let (sender, result) = mpsc::channel();
        self.readback
            .slice(..16)
            .map_async(wgpu::MapMode::Read, move |outcome| {
                let _ = sender.send(outcome);
            });
        let mut state = self.state.lock().expect("GPU diagnostic state poisoned");
        let record = state
            .records
            .iter_mut()
            .find(|record| record.id == self.id)
            .expect("retained diagnostic recording");
        record.status = GpuStatus::Pending;
        record.readback = Some(Readback {
            buffer: self.readback,
            result,
        });
    }
}

impl crate::Renderer {
    pub fn set_diagnostic_gpu(&mut self, state: Option<Arc<Mutex<GpuState>>>) {
        self.engine.diagnostic_gpu = state;
    }
    pub fn begin_diagnostic_batch(&mut self, batch_id: u64) {
        if let Some(state) = &self.engine.diagnostic_gpu {
            state
                .lock()
                .expect("GPU diagnostic state poisoned")
                .begin_batch(batch_id);
        }
    }
    pub fn poll_diagnostic_gpu(&mut self) {
        if let Some(state) = &self.engine.diagnostic_gpu {
            state
                .lock()
                .expect("GPU diagnostic state poisoned")
                .poll_ready();
        }
    }
    pub fn diagnostic_gpu_snapshot(&self, batch_id: u64) -> GpuReport {
        self.engine.diagnostic_gpu.as_ref().map_or_else(
            || GpuReport::disabled(batch_id),
            |state| {
                state
                    .lock()
                    .expect("GPU diagnostic state poisoned")
                    .snapshot(batch_id)
            },
        )
    }
}

#[cfg(test)]
mod tests;
