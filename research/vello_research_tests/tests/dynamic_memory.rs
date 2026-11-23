// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A scene that outgrows the fixed dynamic buffers.
//!
//! The coarse pass sizes its bump-allocated buffers from a fixed table. A scene
//! that outgrows those buffers must be refused after the GPU counters arrive.
//! A later submission can use the counts from that refusal to grow the buffers.

use vello::kurbo::{Affine, Line, Stroke};
use vello::peniko::color::palette;
use vello::{Error, Scene};
use vello_research_tests::TestParams;

const SIZE: u32 = 1024;
const TILE_ALLOCATION_FAILED: u32 = 0x2;

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
    let frames =
        vello_research_tests::get_scene_allocation_attempts_sync(&params, &scene, 3).unwrap();
    let Err(Error::AllocationShortfall {
        needed,
        line_binding_exceeded: false,
        ..
    }) = &frames[0]
    else {
        panic!("the first frame should be refused for tile capacity");
    };
    assert_ne!(
        needed.failed & TILE_ALLOCATION_FAILED,
        0,
        "tile allocation must have failed"
    );
    let recovered = frames[2].as_ref().expect("the third frame should qualify");
    assert!(
        red_pixels(recovered) > 0,
        "the renderer sized the third frame from earlier counters"
    );
}
