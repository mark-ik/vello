// Copyright 2025 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Flattening filled and stroked paths.

use crate::flatten_simd::{Callback, LinePathEl};
use crate::geometry::RectU16;
use crate::kurbo::{self, Affine, PathEl, Stroke, StrokeCtx, StrokeOpts};
use alloc::vec::Vec;
use fearless_simd::{Level, Simd, dispatch};
use log::warn;

pub use crate::flatten_simd::{FlattenCtx, FlattenError, MAX_GENERATED_CURVE_LINES};

// Device-pixel curve flatness target shared by CPU/GPU sparse geometry and the
// maintained Classic flattener. A quarter-pixel approximation amplified tiny
// outline rounding changes into large coverage jumps at subdivision thresholds.
// 1/256px keeps those jumps below the existing alpha acceptance budget, without
// changing canonical curve controls, glyph placement, hinting or brush alpha.
// sqrt(1/256) is exactly representable; derive the squared constants from it.
pub(crate) const SQRT_TOL: f64 = 1.0 / 16.0;
pub(crate) const TOL: f64 = SQRT_TOL * SQRT_TOL;
pub(crate) const TOL_2: f64 = TOL * TOL;

#[cfg(test)]
mod quality_tests {
    use super::*;
    use crate::kurbo::{BezPath, CubicBez, ParamCurve, Point as KurboPoint, QuadBez};

    fn measured_deviation(
        path: &BezPath,
        sample: impl Fn(f64) -> KurboPoint,
        transform: Affine,
    ) -> (f64, usize) {
        let mut lines = Vec::new();
        let mut ctx = FlattenCtx::default();
        fill(
            Level::baseline(),
            path.elements().iter().copied(),
            transform,
            &mut lines,
            &mut ctx,
            RectU16::new(0, 0, 512, 512),
        );
        assert!(!lines.is_empty(), "a visible curve must not disappear");
        let mut maximum = 0.0f64;
        for i in 0..=2048 {
            let p = transform * sample(i as f64 / 2048.0);
            let nearest = lines
                .iter()
                .map(|line| {
                    let a = KurboPoint::new(f64::from(line.p0.x), f64::from(line.p0.y));
                    let b = KurboPoint::new(f64::from(line.p1.x), f64::from(line.p1.y));
                    let d = b - a;
                    let t = if d.hypot2() == 0.0 {
                        0.0
                    } else {
                        ((p - a).dot(d) / d.hypot2()).clamp(0.0, 1.0)
                    };
                    p.distance(a + t * d)
                })
                .fold(f64::INFINITY, f64::min);
            maximum = maximum.max(nearest);
        }
        (maximum, lines.len())
    }

    #[test]
    fn device_pixel_flatness_preserves_canonical_ellipse_and_transforms() {
        let k = 0.5522848f64;
        let curve = CubicBez::new(
            (19.0, 0.0),
            (19.0 + 5.0 * k, 0.0),
            (24.0, 3.0 - 3.0 * k),
            (24.0, 3.0),
        );
        let mut path = BezPath::new();
        path.move_to(curve.p0);
        path.curve_to(curve.p1, curve.p2, curve.p3);
        for transform in [
            Affine::IDENTITY,
            Affine::new([1.25, 0.375, -0.2, 2.5, 4.0, 3.0]),
        ] {
            let (deviation, lines) = measured_deviation(&path, |t| curve.eval(t), transform);
            assert!(
                deviation <= 1.0 / 128.0,
                "device-space deviation {deviation}"
            );
            assert!(
                lines <= 256,
                "small canonical curve has bounded output: {lines}"
            );
        }
    }

    #[test]
    fn tiny_outline_rounding_does_not_trigger_quarter_pixel_chord_loss() {
        for points in [
            [(7.21875, 3.8125), (7.21875, 2.046875), (6.359375, 0.890625)],
            [
                (7.2255859375, 3.814453125),
                (7.2255859375, 2.0439453125),
                (6.3642578125, 0.888671875),
            ],
        ] {
            let curve = QuadBez::new(points[0], points[1], points[2]);
            let mut path = BezPath::new();
            path.move_to(curve.p0);
            path.quad_to(curve.p1, curve.p2);
            let (deviation, lines) = measured_deviation(&path, |t| curve.eval(t), Affine::IDENTITY);
            assert!(
                deviation <= 1.0 / 128.0,
                "rounded outline deviation {deviation}"
            );
            assert!(
                (3..=256).contains(&lines),
                "the visible curved edge needs multiple bounded segments: {lines}"
            );
        }
    }

    #[test]
    fn oversized_cubic_keeps_quality_with_fixed_simd_scratch() {
        let curve = CubicBez::new((8.0, 8.0), (500.0, 500.0), (8.0, 500.0), (500.0, 8.0));
        let mut path = BezPath::new();
        path.move_to(curve.p0);
        path.curve_to(curve.p1, curve.p2, curve.p3);
        let (deviation, lines) = measured_deviation(&path, |t| curve.eval(t), Affine::IDENTITY);
        assert!(
            deviation <= 1.0 / 128.0,
            "scratch saturation must not degrade device quality: {deviation}"
        );
        assert!(
            lines <= 8192,
            "large visible cubic output remains bounded: {lines}"
        );
    }

    #[test]
    fn work_refusal_is_typed_sticky_and_requires_explicit_epoch_reset() {
        let mut path = BezPath::new();
        for _ in 0..8192 {
            path.move_to((7.2255859375, 3.814453125));
            path.quad_to((7.2255859375, 2.0439453125), (6.3642578125, 0.888671875));
        }
        let mut lines = Vec::new();
        let mut ctx = FlattenCtx::default();
        let bounds = RectU16::new(0, 0, 32, 32);
        fill(
            Level::baseline(),
            path.elements().iter().copied(),
            Affine::IDENTITY,
            &mut lines,
            &mut ctx,
            bounds,
        );
        let refused = ctx
            .error()
            .expect("repeated visible curves must hit bounded work");
        assert!(matches!(
            refused,
            FlattenError::LineBudgetExceeded {
                limit: MAX_GENERATED_CURVE_LINES,
                ..
            }
        ));
        assert!(
            lines.is_empty(),
            "partial failed geometry cannot reach strips"
        );
        let mut valid = BezPath::new();
        valid.move_to((1.0, 1.0));
        valid.line_to((3.0, 3.0));
        fill(
            Level::baseline(),
            valid.elements().iter().copied(),
            Affine::IDENTITY,
            &mut lines,
            &mut ctx,
            bounds,
        );
        assert_eq!(
            ctx.error(),
            Some(refused),
            "later draws cannot erase a pending refusal"
        );
        ctx.clear_error();
        fill(
            Level::baseline(),
            valid.elements().iter().copied(),
            Affine::IDENTITY,
            &mut lines,
            &mut ctx,
            bounds,
        );
        assert_eq!(ctx.error(), None);
        assert!(
            !lines.is_empty(),
            "an explicit new epoch accepts valid geometry"
        );
    }
}

/// A point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// The x coordinate of the point.
    pub x: f32,
    /// The y coordinate of the point.
    pub y: f32,
}

impl Point {
    /// The point `(0, 0)`.
    pub const ZERO: Self = Self::new(0., 0.);

    /// Create a new point.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<kurbo::Point> for Point {
    #[inline(always)]
    fn from(value: kurbo::Point) -> Self {
        Self {
            x: value.x as f32,
            y: value.y as f32,
        }
    }
}

impl core::ops::Add for Point {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl core::ops::Sub for Point {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl core::ops::Mul<f32> for Point {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

/// A line.
#[derive(Clone, Copy, Debug)]
pub struct Line {
    /// The start point of the line.
    pub p0: Point,
    /// The end point of the line.
    pub p1: Point,
}

impl Line {
    /// Create a new line.
    pub fn new(p0: Point, p1: Point) -> Self {
        Self { p0, p1 }
    }
}

/// Flatten a filled Bézier path into line segments.
///
/// # Open subpaths and culling
///
/// Open subpaths in the input path get closed by connecting the last endpoint in the subpath to
/// the starting point. The output lines in `line_buf` describe the flattened path, but these lines
/// may describe open subpaths, as some path elements may have been culled.
///
/// For example, consider the following, where the box describes the viewport, a path is marked by
/// `*`, and the region to be filled in the viewport is shaded. For ease of drawing the ASCII art,
/// the path elements are all lines ([`PathEl::LineTo`]), but the same also holds for Bézier path
/// elements.
///
/// ```text
///    ---> winding scan direction
///
///                   * * * * *
///                 *         *
///    ---------- * -----     *
///    |        *░░░░░░░|     *
///    |      *░░░░░░░░░|     *
///    |    *░░░░░░░░░░░|     *
///    |  *░░░░░░░░░░░░░|     *
///    |*░░░░░░░░░░░░░░░|     *
///   *|░░░░░░░░░░░░░░░░|     *
/// *  |░░░░░░░░░░░░░░░░|     *
/// *  |░░░░░░░░░░░░░░░░|     *
/// *  ------------------     *
/// *                         *
/// *                         *
/// *                         *
/// *                         *
/// *                         *
/// *                         *
/// *                         *
/// * * * * * * * * * * * * * *
/// ```
///
/// Because the winding scan direction is from left to right, only the left-of-viewport and
/// diagonal lines matter in later stages of rendering for the winding number and pixel coverage.
/// The other three lines can be culled.
///
/// ```text
///                   *
///                 *
///    ---------- * -----
///    |        *░░░░░░░|
///    |      *░░░░░░░░░|
///    |    *░░░░░░░░░░░|
///    |  *░░░░░░░░░░░░░|
///    |*░░░░░░░░░░░░░░░|
///   *|░░░░░░░░░░░░░░░░|
/// *  |░░░░░░░░░░░░░░░░|
/// *  |░░░░░░░░░░░░░░░░|
/// *  ------------------
/// *
/// *
/// *
/// *
/// *
/// *
/// *
/// ```
///
/// It is important to keep these flattened subpaths open after culling, as closing the subpaths
/// might yield different geometry like the following.
///
/// ```text
///                   *
///                 **
///    ---------- * *----
///    |        *░░*    |
///    |      *░░░*     |
///    |    *░░░░*      |
///    |  *░░░░░*       |
///    |*░░░░░░*        |
///   *|░░░░░░*         |
/// *  |░░░░░*          |
/// *  |░░░░*           |
/// *  --- * ------------
/// *     *
/// *    *
/// *   *
/// *  *
/// * *
/// **
/// *
/// ```
pub fn fill(
    level: Level,
    path: impl IntoIterator<Item = PathEl>,
    affine: Affine,
    line_buf: &mut Vec<Line>,
    ctx: &mut FlattenCtx,
    cull_bbox: RectU16,
) {
    dispatch!(level, simd => fill_impl(simd, path, affine, line_buf, ctx, cull_bbox));
}

/// Flatten a filled bezier path into line segments.
///
/// See the note about open subpaths and culling on [`fill`].
#[inline(always)]
pub fn fill_impl<S: Simd>(
    simd: S,
    path: impl IntoIterator<Item = PathEl>,
    affine: Affine,
    line_buf: &mut Vec<Line>,
    flatten_ctx: &mut FlattenCtx,
    cull_bbox: RectU16,
) {
    line_buf.clear();
    let mut lb = FlattenerCallback {
        line_buf,
        start: Point::ZERO,
        p0: Point::ZERO,
        is_nan: false,
    };

    crate::flatten_simd::flatten(simd, path, affine, &mut lb, flatten_ctx, cull_bbox);

    if flatten_ctx.error().is_some() {
        // No partial curve stream can reach strip generation. The typed error
        // remains sticky until the owner explicitly resets its frame epoch.
        line_buf.clear();
        return;
    }

    // A path that contains NaN is ill-defined, so ignore it.
    if lb.is_nan {
        warn!("A path contains NaN, ignoring it.");

        line_buf.clear();
    }
}
/// Flatten a stroked Bézier path into line segments.
///
/// See the note about open subpaths and culling on [`fill`].
pub fn stroke(
    level: Level,
    path: impl IntoIterator<Item = PathEl>,
    style: &Stroke,
    affine: Affine,
    line_buf: &mut Vec<Line>,
    flatten_ctx: &mut FlattenCtx,
    stroke_ctx: &mut StrokeCtx,
    cull_bbox: RectU16,
) {
    // TODO: Temporary hack to ensure that strokes are scaled properly by the transform.
    let tolerance = TOL
        / affine.as_coeffs()[0]
            .abs()
            .max(affine.as_coeffs()[3].abs())
            .max(1.);

    expand_stroke(path, style, tolerance, stroke_ctx);
    fill(
        level,
        stroke_ctx.output(),
        affine,
        line_buf,
        flatten_ctx,
        cull_bbox,
    );
}

/// Expand a stroked path to a filled path.
pub fn expand_stroke(
    path: impl IntoIterator<Item = PathEl>,
    style: &Stroke,
    tolerance: f64,
    stroke_ctx: &mut StrokeCtx,
) {
    kurbo::stroke_with(path, style, &StrokeOpts::default(), tolerance, stroke_ctx);
}

struct FlattenerCallback<'a> {
    line_buf: &'a mut Vec<Line>,
    start: Point,
    p0: Point,
    is_nan: bool,
}

impl Callback for FlattenerCallback<'_> {
    #[inline(always)]
    fn callback(&mut self, el: LinePathEl) {
        match el {
            LinePathEl::MoveTo(p) => {
                self.is_nan |= p.is_nan();

                let p = p.into();
                self.start = p;
                self.p0 = p;
            }
            LinePathEl::LineTo(p) => {
                self.is_nan |= p.is_nan();

                let p = p.into();
                self.line_buf.push(Line::new(self.p0, p));
                self.p0 = p;
            }
        }
    }
}
