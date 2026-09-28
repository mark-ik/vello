// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Appending one settled [`Scene`] onto another.
//!
//! Prototype for the retained-composition use case: a consumer that
//! caches lowered scenes across frames (a tile cache, a retained
//! widget/fragment system) needs to compose cached pieces into a frame
//! without re-recording their geometry. `vello::Scene::append` serves
//! that role on the compute pipeline; this is the sparse-strip
//! equivalent.
//!
//! Because hybrid generates strips at record time, an appended scene's
//! geometry already exists as viewport-space coverage. That makes two
//! composition shapes cheap and mechanical:
//!
//! - **In place** (`translate = None`): concatenate strip/alpha/paint
//!   storage and replay the recording with indices offset.
//! - **Tile-granular translation**: additionally shift strip
//!   coordinates by multiples of [`Tile::WIDTH`] × [`Tile::HEIGHT`].
//!   The granularity is structural, in both axes: strips are generated
//!   tile-aligned with alpha bytes packed relative to that alignment
//!   (measured, not assumed — a 5px x-shift moves a strip start off
//!   its tile column and the reference recording pads differently), so
//!   a sub-tile shift would require re-slicing coverage, not moving
//!   it. The appended result at tile-granular offsets is
//!   byte-identical to recording the same geometry pre-translated.
//!
//! Arbitrary affine composition is out of scope by construction: the
//! source paths are gone by the time a scene is settled, so a general
//! transform requires retaining input geometry, which is an
//! architectural decision rather than an append implementation.
//!
//! Current limitations, alongside the tile granularity above: the donor is
//! consumed (`EncodedPaint` is not `Clone`, so a by-reference append
//! would first need `Clone`/`Arc` through the encoded-paint chain);
//! filter layers are rejected (their placement data is computed against
//! the donor's coordinates and translating it has open questions);
//! both scenes must share viewport dimensions; and image paints assume
//! both scenes target the same renderer's atlas namespace, which is the
//! ordinary single-renderer case.

use alloc::vec::Vec;
use core::fmt;

use vello_common::paint::{IndexedPaint, Paint};
use vello_common::record::LayerClip;
use vello_common::strip::Strip;
use vello_common::tile::Tile;

use crate::scene::{RecordedDraw, Scene};

/// Why an [`Scene::append_scene`] call was rejected. No partial work is
/// performed on rejection: the target scene is untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendSceneError {
    /// The two scenes have different viewport dimensions.
    ViewportMismatch,
    /// One of the scenes has an unbalanced `push_layer` (the recording
    /// is not settled).
    OpenLayers,
    /// The donor scene contains filter layers, which this prototype
    /// does not translate.
    UnsupportedFilterLayers,
    /// The donor scene's clip strips live in a non-default thread-local
    /// storage, which a single-storage append cannot absorb.
    UnsupportedThreadedClips,
    /// The requested translation is not tile-granular (multiples of
    /// [`Tile::WIDTH`] in x and [`Tile::HEIGHT`] in y). Coverage is
    /// generated in tile-aligned strips with alpha bytes packed
    /// relative to that alignment, so a sub-tile shift would require
    /// re-slicing coverage, not moving it.
    UnalignedTranslation,
    /// An index or coordinate would overflow its storage type.
    Overflow,
}

impl fmt::Display for AppendSceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::ViewportMismatch => "appended scenes must share viewport dimensions",
            Self::OpenLayers => "both scenes must have balanced layers before appending",
            Self::UnsupportedFilterLayers => "appending scenes with filter layers is unsupported",
            Self::UnsupportedThreadedClips => {
                "appending scenes with non-default thread-local clip storage is unsupported"
            }
            Self::UnalignedTranslation => {
                "translation must be tile-granular (multiples of Tile::WIDTH / Tile::HEIGHT)"
            }
            Self::Overflow => "appending would overflow an index or coordinate",
        };
        f.write_str(msg)
    }
}

impl Scene {
    /// Append the contents of `other` onto this scene, in painter order
    /// (after everything already recorded here), optionally shifted by
    /// an integer translation in pixels.
    ///
    /// See the module docs for the composition model and the current
    /// limitations. `other` is consumed; its strip, paint, and command
    /// storage move into `self` with indices rewritten.
    pub fn append_scene(
        &mut self,
        other: Self,
        translate: Option<(u16, u16)>,
    ) -> Result<(), AppendSceneError> {
        let (dx, dy) = translate.unwrap_or((0, 0));
        if dx % Tile::WIDTH != 0 || dy % Tile::HEIGHT != 0 {
            return Err(AppendSceneError::UnalignedTranslation);
        }
        if self.width != other.width || self.height != other.height {
            return Err(AppendSceneError::ViewportMismatch);
        }
        if self.recorder.has_open_layers() || other.recorder.has_open_layers() {
            return Err(AppendSceneError::OpenLayers);
        }
        if !other.recorder.filter_layers.is_empty() {
            return Err(AppendSceneError::UnsupportedFilterLayers);
        }
        for layer in &other.recorder.layers {
            if let Some(clip) = &layer.props.clip_path
                && clip.thread_idx != 0
            {
                return Err(AppendSceneError::UnsupportedThreadedClips);
            }
        }

        let mut other_storage = other.strip_storage.into_inner();
        let mut other_recorder = other.recorder;
        let other_paints = other.encoded_paints;

        // Offsets into this scene's storage that the donor's indices
        // shift by. Everything below is validated before any mutation
        // of `self`, so a failure leaves the target untouched (the
        // consumed donor is the caller's loss either way, which the
        // by-value signature makes visible).
        let strip_base = self.strip_storage.borrow().strips.len();
        let alpha_base: u32 = self
            .strip_storage
            .borrow()
            .alphas
            .len()
            .try_into()
            .map_err(|_| AppendSceneError::Overflow)?;
        let paint_base = self.encoded_paints.len();
        let draw_base: u32 = self
            .recorder
            .draws
            .len()
            .try_into()
            .map_err(|_| AppendSceneError::Overflow)?;
        let layer_base: u32 = self
            .recorder
            .layers
            .len()
            .try_into()
            .map_err(|_| AppendSceneError::Overflow)?;

        // Strips: shift coordinates and alpha indices. `Strip::new`
        // re-packs the alpha index + fill-gap flag. Sentinel strips
        // (`x == u16::MAX`) terminate a path's strip list and must keep
        // their sentinel `x`; their row and alpha index still shift
        // with the content so same-row comparisons and width
        // derivations stay consistent.
        for strip in &mut other_storage.strips {
            let alpha_idx = strip
                .alpha_idx()
                .checked_add(alpha_base)
                .ok_or(AppendSceneError::Overflow)?;
            let y = strip.y.checked_add(dy).ok_or(AppendSceneError::Overflow)?;
            let x = if strip.is_sentinel() {
                strip.x
            } else {
                strip.x.checked_add(dx).ok_or(AppendSceneError::Overflow)?
            };
            *strip = Strip::new(x, y, alpha_idx, strip.fill_gap());
        }

        // Draws: shift strip ranges, translate fast-path rectangles,
        // and re-index non-solid paints into the merged paint table.
        for draw in &mut other_recorder.draws {
            match draw {
                RecordedDraw::Path(path) => {
                    path.strips = (path.strips.start + strip_base)..(path.strips.end + strip_base);
                    remap_paint(&mut path.paint, paint_base);
                }
                RecordedDraw::Rect(rect) => {
                    rect.rect =
                        rect.rect + vello_common::kurbo::Vec2::new(f64::from(dx), f64::from(dy));
                    remap_paint(&mut rect.paint, paint_base);
                }
            }
        }

        // Root nodes and per-layer nodes: shift draw ranges and layer ids.
        for node in &mut other_recorder.nodes {
            node.draws = (node.draws.start + draw_base)..(node.draws.end + draw_base);
            if let Some(layer) = &mut node.layer {
                *layer += layer_base;
            }
        }
        for layer in &mut other_recorder.layers {
            for node in &mut layer.nodes {
                node.draws = (node.draws.start + draw_base)..(node.draws.end + draw_base);
                if let Some(child) = &mut node.layer {
                    *child += layer_base;
                }
            }
            if let Some(clip) = &mut layer.props.clip_path {
                *clip = LayerClip {
                    strip_range: (clip.strip_range.start + strip_base)
                        ..(clip.strip_range.end + strip_base),
                    thread_idx: clip.thread_idx,
                    bbox: translated_bbox(clip.bbox, dx, dy).ok_or(AppendSceneError::Overflow)?,
                };
            }
            layer.bbox = translated_bbox(layer.bbox, dx, dy).ok_or(AppendSceneError::Overflow)?;
        }

        // All donor data is rewritten; move it in.
        {
            let mut storage = self.strip_storage.borrow_mut();
            storage.strips.append(&mut other_storage.strips);
            storage.alphas.append(&mut other_storage.alphas);
        }
        self.encoded_paints.extend(other_paints);
        self.recorder.draws.append(&mut other_recorder.draws);
        // Root nodes: if this scene's trailing node is an open batch
        // (no layer composite), continue it into the donor's first
        // node, exactly as direct recording would have. This keeps the
        // appended recording canonical — byte-identical to recording
        // the same content into one scene — rather than merely
        // equivalent.
        let mut donor_nodes = core::mem::take(&mut other_recorder.nodes);
        if let (Some(last), Some(first)) = (self.recorder.nodes.last_mut(), donor_nodes.first())
            && last.layer.is_none()
        {
            debug_assert_eq!(
                last.draws.end, first.draws.start,
                "settled recordings leave the trailing open batch ending at draws.len()"
            );
            last.draws = last.draws.start..first.draws.end;
            last.layer = first.layer;
            donor_nodes.remove(0);
        }
        self.recorder.nodes.append(&mut donor_nodes);
        self.recorder
            .layers
            .append(&mut other_recorder.layers.drain(..).collect::<Vec<_>>());

        // Conservative scalar merges: maxima grow, flags OR. All of
        // these only ever cause the renderer to provision more, never
        // less.
        self.recorder.max_layer_depth = self
            .recorder
            .max_layer_depth
            .max(other_recorder.max_layer_depth);
        self.recorder.root_is_blend_target |= other_recorder.root_is_blend_target;
        self.recorder.has_non_default_blend |= other_recorder.has_non_default_blend;
        self.recorder.largest_layer_size = max_size(
            self.recorder.largest_layer_size,
            other_recorder.largest_layer_size,
        );
        self.recorder.largest_filter_layer_size = max_size(
            self.recorder.largest_filter_layer_size,
            other_recorder.largest_filter_layer_size,
        );

        Ok(())
    }
}

/// Re-index a non-solid paint into the merged encoded-paint table.
fn remap_paint(paint: &mut Paint, paint_base: usize) {
    if let Paint::Indexed(indexed) = paint {
        *paint = Paint::Indexed(IndexedPaint::new(indexed.index() + paint_base));
    }
}

/// Translate a tile-aligned bbox, re-snapping outward so the result
/// stays tile-aligned when `dx` is not a tile multiple. Outward
/// snapping only enlarges the region a layer is provisioned for, which
/// is safe (the extra area is transparent).
fn translated_bbox(
    bbox: vello_common::geometry::RectU16,
    dx: u16,
    dy: u16,
) -> Option<vello_common::geometry::RectU16> {
    use vello_common::util::RectExt as _;
    let translated = vello_common::geometry::RectU16::new(
        bbox.x0.checked_add(dx)?,
        bbox.y0.checked_add(dy)?,
        bbox.x1.checked_add(dx)?,
        bbox.y1.checked_add(dy)?,
    );
    Some(translated.snap_to_tile_coordinates())
}

fn max_size(
    a: Option<vello_common::geometry::SizeU16>,
    b: Option<vello_common::geometry::SizeU16>,
) -> Option<vello_common::geometry::SizeU16> {
    match (a, b) {
        (Some(a), Some(b)) => Some(vello_common::geometry::SizeU16([
            a.0[0].max(b.0[0]),
            a.0[1].max(b.0[1]),
        ])),
        (a, None) => a,
        (None, b) => b,
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    use vello_common::color::palette::css;
    use vello_common::kurbo::{Affine, BezPath, Point, Rect, Vec2};
    use vello_common::peniko::{ColorStop, ColorStops, Gradient};

    use super::AppendSceneError;
    use crate::scene::Scene;

    const W: u16 = 128;
    const H: u16 = 128;

    fn tri(offset: Vec2) -> BezPath {
        let mut p = BezPath::new();
        p.move_to(Point::new(10.0, 40.0) + offset);
        p.line_to(Point::new(40.0, 10.0) + offset);
        p.curve_to(
            Point::new(50.0, 30.0) + offset,
            Point::new(45.0, 38.0) + offset,
            Point::new(12.0, 41.0) + offset,
        );
        p.close_path();
        p
    }

    fn gradient() -> Gradient {
        Gradient::new_linear(Point::new(0.0, 0.0), Point::new(64.0, 0.0)).with_stops(ColorStops(
            [
                ColorStop::from((0.0, css::RED)),
                ColorStop::from((1.0, css::BLUE)),
            ]
            .into_iter()
            .collect(),
        ))
    }

    /// Content block A: a solid rect and a filled path.
    fn record_a(scene: &mut Scene) {
        scene.set_paint(css::DARK_SLATE_GRAY);
        scene.fill_rect(&Rect::new(4.0, 4.0, 60.0, 24.0));
        scene.set_paint(css::GOLDENROD);
        scene.fill_path(&tri(Vec2::ZERO));
    }

    /// Content block B, at `offset`: a gradient rect (exercises the
    /// encoded-paint index remap), an opacity layer, and a clip layer
    /// (exercises layers, nodes, and clip strip ranges).
    fn record_b(scene: &mut Scene, offset: Vec2) {
        scene.set_paint(gradient());
        scene.fill_rect(&(Rect::new(20.0, 50.0, 84.0, 70.0) + offset));

        scene.push_opacity_layer(0.5);
        scene.set_paint(css::CRIMSON);
        scene.fill_rect(&(Rect::new(30.0, 60.0, 90.0, 92.0) + offset));
        scene.pop_layer();

        scene.push_clip_layer(&(Affine::translate(offset) * tri(Vec2::new(40.0, 55.0))));
        scene.set_paint(css::SEA_GREEN);
        scene.fill_rect(&(Rect::new(40.0, 55.0, 100.0, 110.0) + offset));
        scene.pop_layer();
    }

    /// Content block B without layers, for the translated comparison
    /// (translated layer bboxes re-snap outward, which is functionally
    /// safe but not byte-identical; the layered case is pinned by the
    /// in-place test where snapping is the identity).
    fn record_b_flat(scene: &mut Scene, offset: Vec2) {
        scene.set_paint(gradient());
        scene.fill_rect(&(Rect::new(20.0, 50.0, 84.0, 70.0) + offset));
        scene.set_paint(css::CRIMSON);
        scene.fill_path(&(Affine::translate(offset) * tri(Vec2::new(30.0, 52.0))));
    }

    fn recorder_fingerprint(scene: &Scene) -> (String, String, String, usize) {
        (
            format!("{:?}", scene.recorder.draws),
            format!("{:?}", scene.recorder.nodes),
            format!("{:?}", scene.recorder.layers),
            scene.encoded_paints.len(),
        )
    }

    fn storages_equal(a: &Scene, b: &Scene) -> bool {
        *a.strip_storage.borrow() == *b.strip_storage.borrow()
    }

    /// `record A; record B` in one scene must equal `record A` +
    /// `append(record B)` across every retained table: strips, alphas,
    /// draws, nodes, layers, and paint indices.
    #[test]
    fn append_in_place_matches_direct_recording() {
        let mut reference = Scene::new(W, H);
        record_a(&mut reference);
        record_b(&mut reference, Vec2::ZERO);

        let mut target = Scene::new(W, H);
        record_a(&mut target);
        let mut donor = Scene::new(W, H);
        record_b(&mut donor, Vec2::ZERO);
        target.append_scene(donor, None).expect("append succeeds");

        assert!(
            storages_equal(&reference, &target),
            "strip and alpha storage must match the direct recording byte for byte"
        );
        assert_eq!(
            recorder_fingerprint(&reference),
            recorder_fingerprint(&target),
            "draws, nodes, layers, and paint count must match the direct recording"
        );
    }

    /// Appending with an integer translation must equal recording the
    /// same geometry pre-translated: strip generation commutes with
    /// integer translation (x by pixels, y by strip-height multiples).
    #[test]
    fn append_translated_matches_pretranslated_recording() {
        let offset = Vec2::new(4.0, 8.0);

        let mut reference = Scene::new(W, H);
        record_a(&mut reference);
        record_b_flat(&mut reference, offset);

        let mut target = Scene::new(W, H);
        record_a(&mut target);
        let mut donor = Scene::new(W, H);
        record_b_flat(&mut donor, Vec2::ZERO);
        target
            .append_scene(donor, Some((4, 8)))
            .expect("append succeeds");

        {
            let r = reference.strip_storage.borrow();
            let t = target.strip_storage.borrow();
            if *r != *t {
                let mut diag = format!(
                    "strips: ref {} vs got {}; alphas: ref {} vs got {}",
                    r.strips.len(),
                    t.strips.len(),
                    r.alphas.len(),
                    t.alphas.len()
                );
                for (i, (a, b)) in r.strips.iter().zip(t.strips.iter()).enumerate() {
                    if a != b {
                        diag += &format!("; first differing strip [{i}]: ref {a:?} vs got {b:?}");
                        break;
                    }
                }
                for (i, (a, b)) in r.alphas.iter().zip(t.alphas.iter()).enumerate() {
                    if a != b {
                        diag += &format!("; first differing alpha [{i}]: ref {a} vs got {b}");
                        break;
                    }
                }
                panic!("integer-translated strips must match pre-translated recording: {diag}");
            }
        }
        assert_eq!(
            recorder_fingerprint(&reference),
            recorder_fingerprint(&target),
            "translated draws must match the pre-translated recording"
        );
    }

    /// Repeated appends of the same content at different offsets: the
    /// tile-cache shape. Each append lands after the previous in
    /// painter order.
    #[test]
    fn repeated_appends_accumulate_in_painter_order() {
        let mut reference = Scene::new(W, H);
        record_b_flat(&mut reference, Vec2::ZERO);
        record_b_flat(&mut reference, Vec2::new(4.0, 4.0));
        record_b_flat(&mut reference, Vec2::new(8.0, 8.0));

        let mut target = Scene::new(W, H);
        record_b_flat(&mut target, Vec2::ZERO);
        for (dx, dy) in [(4, 4), (8, 8)] {
            let mut donor = Scene::new(W, H);
            record_b_flat(&mut donor, Vec2::ZERO);
            target
                .append_scene(donor, Some((dx, dy)))
                .expect("append succeeds");
        }

        assert!(storages_equal(&reference, &target));
        assert_eq!(
            recorder_fingerprint(&reference),
            recorder_fingerprint(&target)
        );
    }

    #[test]
    fn rejections_leave_the_target_untouched() {
        let mut target = Scene::new(W, H);
        record_a(&mut target);
        let before_strips = target.strip_storage.borrow().strips.len();
        let before = recorder_fingerprint(&target);

        // Unaligned y translation.
        let mut donor = Scene::new(W, H);
        record_b_flat(&mut donor, Vec2::ZERO);
        assert_eq!(
            target.append_scene(donor, Some((0, 3))),
            Err(AppendSceneError::UnalignedTranslation)
        );

        // Viewport mismatch.
        let mut small = Scene::new(64, 64);
        record_b_flat(&mut small, Vec2::ZERO);
        assert_eq!(
            target.append_scene(small, None),
            Err(AppendSceneError::ViewportMismatch)
        );

        // Open layer on the donor.
        let mut open = Scene::new(W, H);
        open.push_opacity_layer(0.5);
        assert_eq!(
            target.append_scene(open, None),
            Err(AppendSceneError::OpenLayers)
        );

        // Filter layer on the donor.
        use vello_common::filter_effects::{EdgeMode, Filter, FilterPrimitive};
        let mut filtered = Scene::new(W, H);
        filtered.push_filter_layer(Filter::from_primitive(FilterPrimitive::GaussianBlur {
            std_deviation: 0.5,
            edge_mode: EdgeMode::None,
        }));
        filtered.set_paint(css::CRIMSON);
        filtered.fill_rect(&Rect::new(10.0, 10.0, 30.0, 30.0));
        filtered.pop_layer();
        assert_eq!(
            target.append_scene(filtered, None),
            Err(AppendSceneError::UnsupportedFilterLayers)
        );

        assert_eq!(target.strip_storage.borrow().strips.len(), before_strips);
        assert_eq!(recorder_fingerprint(&target), before);
        let _ = Vec::<u8>::new();
    }
}
