// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use vello_encoding::DrawTag;

fn nested(depth: usize) -> Vec<DrawTag> {
    let mut tags = vec![DrawTag::BEGIN_CLIP; depth];
    tags.extend(std::iter::repeat_n(DrawTag::END_CLIP, depth));
    tags
}

#[test]
fn blend_floor_tracks_maximum_depth_and_caps_without_rejecting() {
    let cap = 128 << 20;
    assert_eq!(conservative_blend_floor(&nested(4), 1024, 600, cap), 0);
    assert_eq!(
        conservative_blend_floor(&nested(5), 1024, 600, cap),
        622_592
    );
    assert_eq!(
        conservative_blend_floor(&nested(6), 1024, 600, cap),
        1_245_184
    );
    let mut disjoint = nested(6);
    disjoint.extend(nested(6));
    assert_eq!(
        conservative_blend_floor(&disjoint, 1024, 600, cap),
        1_245_184
    );
    assert_eq!(
        conservative_blend_floor(&nested(6), 1024, 600, 4 << 20),
        1 << 20
    );
    assert_eq!(
        conservative_blend_floor(&nested(6), u32::MAX, u32::MAX, cap),
        cap as u32 / 4
    );
    assert_eq!(
        conservative_blend_floor(&[DrawTag::END_CLIP], 1024, 600, cap),
        cap as u32 / 4
    );
}

#[test]
fn resolver_closes_open_layers_before_estimating() {
    use crate::Scene;
    use peniko::kurbo::{Affine, Rect};
    use peniko::{Fill, Mix};
    let mut scene = Scene::new();
    for _ in 0..6 {
        scene.push_layer(
            Fill::NonZero,
            Mix::Normal,
            1.0,
            Affine::IDENTITY,
            &Rect::new(0., 0., 1024., 600.),
        );
    }
    let mut resolver = Resolver::new();
    let mut packed = Vec::new();
    let (layout, _, _) = resolver.resolve(scene.encoding(), &mut packed);
    let tags = layout.draw_tags(&packed);
    assert_eq!(
        tags.iter()
            .filter(|tag| **tag == DrawTag::BEGIN_CLIP)
            .count(),
        6
    );
    assert_eq!(
        tags.iter().filter(|tag| **tag == DrawTag::END_CLIP).count(),
        6
    );
    assert_eq!(
        conservative_blend_floor(tags, 1024, 600, 128 << 20),
        1_245_184
    );
}

#[test]
fn color_glyph_expansion_is_in_resolved_tag_stream() {
    use crate::{Glyph, Scene};
    use peniko::{Blob, Color, Fill, FontData};
    use skrifa::MetadataProvider;
    let bytes = include_bytes!("../../../assets/noto_color_emoji/NotoColorEmoji-Subset.ttf");
    let parsed = skrifa::FontRef::new(bytes).unwrap();
    let id = parsed.charmap().map('✅').expect("fixture glyph");
    assert!(
        parsed.color_glyphs().get(id).is_some(),
        "fixture must be COLR"
    );
    let font = FontData {
        data: Blob::new(std::sync::Arc::new(bytes.to_vec())),
        index: 0,
    };
    let mut scene = Scene::new();
    scene
        .draw_glyphs(&font)
        .font_size(24.)
        .brush(Color::WHITE)
        .draw(
            Fill::NonZero,
            [Glyph {
                id: id.to_u32(),
                x: 0.,
                y: 24.,
            }]
            .into_iter(),
        );
    let mut resolver = Resolver::new();
    let mut packed = Vec::new();
    let (layout, _, _) = resolver.resolve(scene.encoding(), &mut packed);
    let tags = layout.draw_tags(&packed);
    assert!(
        tags.iter().any(|tag| *tag == DrawTag::BEGIN_CLIP),
        "COLR clip tags must be present"
    );
    assert_eq!(
        tags.iter()
            .filter(|tag| **tag == DrawTag::BEGIN_CLIP)
            .count(),
        tags.iter().filter(|tag| **tag == DrawTag::END_CLIP).count()
    );
}

#[test]
fn capped_estimate_still_reports_an_observed_blend_shortfall() {
    let cap = 4 << 20;
    let needed = BumpAllocators {
        failed: 0x10,
        blend: 1_245_184,
        ..Default::default()
    };
    let mut high_water = BumpAllocators::default();
    assert_eq!(
        conservative_blend_floor(&nested(6), 1024, 600, cap),
        1 << 20
    );
    match crate::accept_quality_counters(&mut high_water, needed, cap) {
        Err(crate::Error::AllocationShortfall {
            needed,
            max_binding_bytes,
            line_binding_exceeded,
        }) => {
            assert_eq!(needed.failed, 0x10);
            assert_eq!(needed.blend, 1_245_184);
            assert_eq!(max_binding_bytes, cap);
            assert!(!line_binding_exceeded);
        }
        other => panic!("observed blend shortfall lost its typed refusal: {other:?}"),
    }
    assert!(high_water.blend > needed.blend);
}
