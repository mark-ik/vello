// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

fn completed(batch_id: u64, id: u64, elapsed_ns: u64) -> Recording {
    Recording {
        batch_id,
        id,
        period_bits: 1.0f32.to_bits(),
        status: GpuStatus::Available,
        elapsed_ns: Some(elapsed_ns),
        readback: None,
    }
}

#[test]
fn disabled_unsupported_and_no_recording_are_not_zero_gpu_execution() {
    assert_eq!(GpuReport::disabled(7).status, GpuStatus::Disabled);
    let mut state = GpuState::new(false);
    state.begin_batch(7);
    assert_eq!(state.snapshot(7).status, GpuStatus::Unsupported);
    assert_eq!(state.snapshot(7).elapsed_ns, None);
    let enabled = GpuState::new(true);
    assert_eq!(enabled.snapshot(7).status, GpuStatus::NoRecording);
    assert_eq!(enabled.snapshot(7).elapsed_ns, None);
}

#[test]
fn complete_zero_measurements_and_other_batch_provenance_are_preserved() {
    let mut state = GpuState::new(true);
    state.records.push_back(completed(7, 1, 0));
    state.records.push_back(completed(8, 2, 900));
    let report = state.snapshot(7);
    assert_eq!(report.status, GpuStatus::Available);
    assert_eq!(report.elapsed_ns, Some(0));
    assert_eq!(report.recordings, 1);
    assert_eq!(report.first_recording_id, Some(1));
    assert_eq!(report.last_recording_id, Some(1));
    assert_eq!(state.snapshot(8).elapsed_ns, Some(900));
}

#[test]
fn partial_pending_failed_or_capacity_reports_never_publish_a_partial_gpu_sum() {
    let mut state = GpuState::new(true);
    state.records.push_back(completed(7, 1, 50));
    let mut pending = completed(7, 2, 0);
    pending.status = GpuStatus::Pending;
    pending.elapsed_ns = None;
    state.records.push_back(pending);
    let before = state.snapshot(7);
    assert_eq!(before.status, GpuStatus::Pending);
    assert_eq!((before.completed, before.pending), (1, 1));
    assert_eq!(before.elapsed_ns, None);
    assert_eq!(
        state.snapshot(7),
        before,
        "snapshot must not collect or consume results"
    );
    state.records[1].status = GpuStatus::Failed;
    assert_eq!(state.snapshot(7).status, GpuStatus::Failed);
    assert_eq!(state.snapshot(7).elapsed_ns, None);
    state.overflow_batch = Some(7);
    assert_eq!(state.snapshot(7).status, GpuStatus::CapacityExceeded);
    assert_eq!(state.snapshot(7).elapsed_ns, None);
}

#[test]
fn timestamp_conversion_rejects_counter_regression_invalid_period_and_overflow() {
    assert_eq!(elapsed_ns(10, 20, 2.5f32.to_bits()), Some(25));
    assert_eq!(elapsed_ns(10, 10, 1.0f32.to_bits()), Some(0));
    assert_eq!(elapsed_ns(20, 10, 1.0f32.to_bits()), None);
    for period in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(elapsed_ns(1, 2, period.to_bits()), None);
    }
    assert_eq!(elapsed_ns(0, u64::MAX, f32::MAX.to_bits()), None);
}

#[test]
fn a_later_capacity_overflow_cannot_make_an_earlier_partial_batch_available() {
    let mut state = GpuState::new(true);
    // One retained recording of batch7 completes after another recording of
    // that batch was skipped. A later batch overflows before snapshot7.
    state.records.push_back(completed(7, 1, 50));
    state.begin_batch(7);
    state.mark_overflow();
    assert_eq!(state.snapshot(7).status, GpuStatus::CapacityExceeded);
    state.begin_batch(8);
    state.mark_overflow();
    let earlier = state.snapshot(7);
    assert_eq!(earlier.completed, 1);
    assert_eq!(earlier.status, GpuStatus::HistoryExpired);
    assert_eq!(
        earlier.elapsed_ns, None,
        "missing GPU work stays unavailable"
    );
    assert_eq!(state.snapshot(8).status, GpuStatus::CapacityExceeded);
    state.expire_through(u64::MAX);
    assert_eq!(state.snapshot(u64::MAX).status, GpuStatus::HistoryExpired);
}
