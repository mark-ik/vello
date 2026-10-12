// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// A curve could not be flattened at the device-pixel quality target within
/// bounded work. This is a geometry refusal, not an allocation-retry request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlattenError {
    /// A transformed curve cannot be represented by the f32 raster pipeline.
    NonFiniteCurve,
    /// Generated curve lines would exceed the per-path work budget.
    LineBudgetExceeded {
        /// Number of generated lines requested.
        requested: usize,
        /// Maximum generated lines accepted for one path.
        limit: usize,
    },
    /// An oversized cubic could not fit the SIMD scratch within bounded depth.
    SubdivisionDepthExceeded {
        /// Maximum subdivision depth.
        limit: usize,
    },
}

impl core::fmt::Display for FlattenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonFiniteCurve => write!(f, "curve exceeds finite f32 raster coordinates"),
            Self::LineBudgetExceeded { requested, limit } => {
                write!(
                    f,
                    "curve line budget exceeded: {requested} requested, limit {limit}"
                )
            }
            Self::SubdivisionDepthExceeded { limit } => {
                write!(f, "curve SIMD subdivision depth exceeded: limit {limit}")
            }
        }
    }
}

impl core::error::Error for FlattenError {}

/// A path may generate this many curve lines before an explicit refusal.
/// Straight input lines retain their existing operation admission contract.
pub const MAX_GENERATED_CURVE_LINES: usize = 65_536;

/// DFS subdivision storage is fixed; no heap growth occurs for cubic chunks.
pub(super) const MAX_CUBIC_SUBDIVISION_DEPTH: usize = 64;
