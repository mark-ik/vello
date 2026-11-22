// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use vello::{
    Scene,
    kurbo::{Affine, Rect},
    peniko::{Color, Fill, Mix},
};
use vello_research_tests::{TestParams, get_checked_scene_image_sync};

fn blend_scene(width: u32, height: u32, layers: usize) -> Scene {
    let rect = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let mut scene = Scene::new();
    for _ in 0..layers {
        scene.push_layer(Fill::NonZero, Mix::Normal, 0.8, Affine::IDENTITY, &rect);
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None, &rect);
    for _ in 0..layers {
        scene.pop_layer();
    }
    scene
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn deep_blend_first_frame_exceeds_the_default_spill_buffer() {
    let (width, height) = (1024, 600);
    // Six full-surface layers require two spill slots per tile, more than
    // the old buffer's capacity of 4096 slots.
    let image = get_checked_scene_image_sync(
        &TestParams::new("deep_blend_large_spill", width, height),
        &blend_scene(width, height, 6),
    )
    .unwrap();
    let pixel = ((height / 2 * width + width / 2) * 4) as usize;
    let rgba = &image.data.as_ref()[pixel..pixel + 4];
    assert_eq!(rgba[3], 255);
    for value in &rgba[..3] {
        assert!(
            (64..=70).contains(value),
            "unexpected center pixel: {rgba:?}"
        );
    }
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn checked_render_reports_spill_beyond_the_device_binding_limit() {
    let (width, height) = (1024, 1024);
    let image = get_checked_scene_image_sync(
        &TestParams::new("deep_blend_over_binding_limit", width, height),
        // 36 spill slots per tile need 144 MiB. The test device requests
        // the default 128 MiB storage binding limit.
        &blend_scene(width, height, 40),
    );
    let Err(error) = image else {
        panic!("a frame exceeding the binding limit was accepted");
    };
    let message = error.to_string();
    assert!(message.contains("GPU allocation shortfall"), "{message}");
}
