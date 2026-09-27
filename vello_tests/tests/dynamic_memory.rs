// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A scene that outgrows the fixed dynamic buffers.
//!
//! The coarse pass sizes its bump-allocated buffers from a fixed table. A scene
//! that needs more renders nothing and reports no error. The renderer reads the
//! GPU's bump counters back after each frame and sizes later frames to what the
//! scene needed. Recovery takes as long as the asynchronous readback needs.

use vello::Scene;
use vello::kurbo::{Affine, Line, Stroke};
use vello::peniko::color::palette;
use vello_tests::TestParams;

const SIZE: u32 = 1024;

/// Red pixels in an RGBA8 image, against the black base.
fn red_pixels(image: &vello::peniko::ImageData) -> usize {
    image
        .data
        .data()
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 64 && pixel[1] < 64 && pixel[2] < 64)
        .count()
}

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn a_scene_past_the_tile_table_recovers_after_readback() {
    // Each full diagonal's bounding box covers the whole 64 by 64 tile target,
    // 4,096 tiles, so a thousand of them need about twice the table's 2^21.
    let mut scene = Scene::new();
    let size = f64::from(SIZE);
    for i in 0..1000 {
        let offset = f64::from(i % 64) * 16.0;
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            palette::css::RED,
            None,
            &Line::new((0.0, offset), (size, size - offset)),
        );
    }
    let params = TestParams::new("a_scene_past_the_tile_table", SIZE, SIZE);
    let frames = vello_tests::get_scene_images_sync(&params, &scene, 3).unwrap();
    assert_eq!(
        red_pixels(&frames[0]),
        0,
        "the first frame must overflow the fixed table, or this test proves nothing"
    );
    assert!(
        red_pixels(&frames[2]) > 0,
        "the renderer sized the third frame from the first frame's counters"
    );
}
