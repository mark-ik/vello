// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A synthetic repeated-preview composition through the production Scene
//! producer and renderer. This is not a captured Tabard DOM/editor workload.

use vello::{
    Scene,
    kurbo::{Affine, Circle, Line, Rect, Stroke},
    peniko::{Color, Fill, ImageData, Mix, color::palette},
};
use vello_research_tests::{TestParams, get_scene_images_sync};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 192;
const SURFACE: [u8; 4] = [32, 48, 64, 255];

fn preview_scene(accent: Color) -> Scene {
    let mut scene = Scene::new();
    for row in 0..2 {
        for col in 0..3 {
            let x = 4.0 + f64::from(col) * 84.0;
            let y = 4.0 + f64::from(row) * 92.0;
            let card = Rect::new(x, y, x + 80.0, y + 88.0);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(32, 48, 64, 255),
                None,
                &card,
            );
            // Six partially covered clip layers exercise the producer's
            // clip commands and fine's stack/spill split on boundary tiles.
            for _ in 0..6 {
                scene.push_layer(
                    Fill::NonZero,
                    Mix::Normal,
                    1.0,
                    Affine::IDENTITY,
                    &Rect::new(x + 8.0, y + 8.0, x + 72.0, y + 80.0),
                );
            }
            // More than two dynamic PTCL chunks per covered tile. The
            // renderer must execute coarse-generated forward command links.
            for _ in 0..192 {
                scene.fill(Fill::NonZero, Affine::IDENTITY, accent, None, &card);
            }
            scene.stroke(
                &Stroke::new(2.0),
                Affine::IDENTITY,
                palette::css::WHITE,
                None,
                &Line::new((x + 22.0, y + 30.0), (x + 57.0, y + 55.0)),
            );
            for (cx, cy, radius) in [(x + 22.0, y + 30.0, 7.0), (x + 57.0, y + 55.0, 5.0)] {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    palette::css::WHITE,
                    None,
                    &Circle::new((cx, cy), radius),
                );
            }
            for _ in 0..6 {
                scene.pop_layer();
            }
        }
    }
    scene
}

fn pixel(image: &ImageData, x: u32, y: u32) -> &[u8] {
    let index = ((y * WIDTH + x) * 4) as usize;
    &image.data.data()[index..index + 4]
}

#[test]
#[ignore = "Offscreen renderer diagnostic; run deliberately after shader review"]
fn repeated_clipped_previews_preserve_pixels_across_frames_and_paint_changes() {
    let params = TestParams::new("repeated_clipped_previews", WIDTH, HEIGHT);
    for (accent, expected) in [
        (palette::css::RED, [255, 0, 0, 255]),
        (palette::css::BLUE, [0, 0, 255, 255]),
    ] {
        let frames = get_scene_images_sync(&params, &preview_scene(accent), 3).unwrap();
        assert_eq!(frames.len(), 3);
        for image in &frames {
            assert_eq!(pixel(image, 0, 0), [0, 0, 0, 255]);
            assert_eq!(pixel(image, WIDTH - 1, HEIGHT - 1), [0, 0, 0, 255]);
            for row in 0..2 {
                for col in 0..3 {
                    let x = 4 + col * 84;
                    let y = 4 + row * 92;
                    assert_eq!(
                        pixel(image, x + 3, y + 3),
                        SURFACE,
                        "clip exterior must retain its card surface"
                    );
                    assert_eq!(
                        pixel(image, x + 12, y + 72),
                        expected,
                        "clipped interior must retain the selected paint"
                    );
                    assert_eq!(
                        pixel(image, x + 22, y + 30),
                        [255, 255, 255, 255],
                        "the curved foreground specimen must render"
                    );
                }
            }
            assert_eq!(
                image.data.data(),
                frames[0].data.data(),
                "repeated production render submissions must retain exact pixels"
            );
        }
    }
}
