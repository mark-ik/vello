// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Coverage for sparse bitmap slices and clip commands spanning those slices.
//! Native CPU/GPU comparison complements the Chrome WebGPU consumer receipt:
//! the original traversal fails only on the qualified Radeon/Dawn path.

use vello::{
    Scene,
    kurbo::{Affine, Rect},
    peniko::{Fill, Mix, color::palette},
};
use vello_tests::{TestParams, compare_gpu_cpu_sync};

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn coarse_bitmap_empty_slices_and_late_bits() {
    let mut scene = Scene::new();
    // All draws share one bin. On the right tile, the first two bitmap slices
    // are empty, and the visible draws include bit 31 and a later slice.
    for _ in 0..64 {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette::css::RED,
            None,
            &Rect::new(0.0, 0.0, 16.0, 16.0),
        );
    }
    for _ in 0..32 {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette::css::GREEN,
            None,
            &Rect::new(32.0, 0.0, 48.0, 16.0),
        );
    }
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        palette::css::BLUE,
        None,
        &Rect::new(32.0, 0.0, 48.0, 16.0),
    );
    let mut result = compare_gpu_cpu_sync(
        scene,
        TestParams::new("coarse_bitmap_empty_slices_and_late_bits", 64, 32),
    )
    .unwrap();
    result.assert_mean_less_than(0.001);
    assert_eq!(
        &result.gpu_rendered.data.as_ref()[(8 * 64 + 8) * 4..][..4],
        &[255, 0, 0, 255]
    );
    assert_eq!(
        &result.gpu_rendered.data.as_ref()[(8 * 64 + 40) * 4..][..4],
        &[0, 0, 255, 255]
    );
    assert_eq!(
        &result.gpu_rendered.data.as_ref()[(8 * 64 + 56) * 4..][..4],
        &[0, 0, 0, 255]
    );
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn coarse_bitmap_clip_closes_after_multiple_slices() {
    let mut scene = Scene::new();
    scene.push_layer(
        Fill::NonZero,
        Mix::Normal,
        1.0,
        Affine::IDENTITY,
        &Rect::new(8.0, 8.0, 56.0, 24.0),
    );
    for _ in 0..96 {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette::css::BLUE,
            None,
            &Rect::new(0.0, 0.0, 64.0, 32.0),
        );
    }
    scene.pop_layer();
    let mut result = compare_gpu_cpu_sync(
        scene,
        TestParams::new("coarse_bitmap_clip_closes_after_multiple_slices", 64, 32),
    )
    .unwrap();
    result.assert_mean_less_than(0.001);
    assert_eq!(
        &result.gpu_rendered.data.as_ref()[(16 * 64 + 32) * 4..][..4],
        &[0, 0, 255, 255]
    );
    assert_eq!(
        &result.gpu_rendered.data.as_ref()[(16 * 64 + 4) * 4..][..4],
        &[0, 0, 0, 255]
    );
}
