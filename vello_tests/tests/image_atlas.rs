// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Image atlas residency must survive intervening renders without resources.

use vello::{
    Scene,
    kurbo::{Affine, Rect},
    peniko::{
        Color, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality, ImageSampler,
    },
};
use vello_tests::{TestParams, get_scene_sequence_images_sync};

const SIDE: u32 = 32;

#[test]
#[cfg_attr(skip_gpu_tests, ignore)]
fn resident_image_survives_repeated_patchless_solid_and_empty_frames() {
    let mut pixels = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            // Four distinct opaque quadrants catch lost, shifted or truncated
            // atlas content. Integer placement and nearest sampling make every
            // output pixel exact, without a snapshot tolerance.
            let color = match (x < SIDE / 2, y < SIDE / 2) {
                (true, true) => [255, 0, 0, 255],
                (false, true) => [0, 255, 0, 255],
                (true, false) => [0, 0, 255, 255],
                (false, false) => [255, 255, 0, 255],
            };
            pixels.extend_from_slice(&color);
        }
    }
    let image = ImageBrush {
        image: ImageData {
            data: pixels.clone().into(),
            format: ImageFormat::Rgba8,
            width: SIDE,
            height: SIDE,
            alpha_type: ImageAlphaType::Alpha,
        },
        sampler: ImageSampler {
            quality: ImageQuality::Low,
            ..Default::default()
        },
    };
    let mut image_scene = Scene::new();
    image_scene.draw_image(&image, Affine::IDENTITY);
    let mut solid_scene = Scene::new();
    solid_scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(255, 0, 255, 255),
        None,
        &Rect::new(0.0, 0.0, f64::from(SIDE), f64::from(SIDE)),
    );
    let empty_scene = Scene::new();

    // The very same image resource must remain usable: rebuilding it would
    // allocate a new atlas entry and hide the original residency failure.
    let mut sequence = vec![&image_scene];
    for _ in 0..3 {
        sequence.extend([&solid_scene, &image_scene, &empty_scene, &image_scene]);
    }
    let mut params = TestParams::new("image_atlas_after_patchless_frames", SIDE, SIDE);
    params.base_color = Some(Color::from_rgba8(0, 0, 0, 0));
    let frames = get_scene_sequence_images_sync(&params, &sequence).unwrap();
    assert_eq!(frames.len(), sequence.len());
    let solid_pixels = [255, 0, 255, 255].repeat((SIDE * SIDE) as usize);
    let empty_pixels = vec![0; (SIDE * SIDE * 4) as usize];
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!((frame.width, frame.height), (SIDE, SIDE));
        let expected = if index == 0 || index % 2 == 0 {
            &pixels
        } else if index % 4 == 1 {
            &solid_pixels
        } else {
            &empty_pixels
        };
        assert_eq!(
            frame.data.data(),
            expected.as_slice(),
            "frame {index} must preserve the exact image or patchless-frame pixels"
        );
    }
}
