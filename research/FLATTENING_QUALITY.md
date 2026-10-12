# Maintained device-pixel curve quality

Classic 0.11 and Sparse Strips 0.3 retain canonical curves and caller-shaped
glyph inputs. Their device-pixel approximation target is 1/256 pixel. Sparse
subdivision retains fixed SIMD scratch and chunks oversized cubic work.
Round geometry and conservative allocation estimates use the same finer target.

Work remains bounded: 65,536 generated lines per curve, with a 16,777,216 total
Classic line limit. Sparse recordings preserve typed, sticky flattening errors
until explicit reset. Checked CPU/GPU render and append entry points refuse bad
recordings before target mutation. Conservative Classic estimates that exceed
the total line budget refuse before resolver or target preparation, separately
from recoverable dynamic-buffer allocation shortfalls.

Every normal Classic frame downloads quality counters. Hosts must call
`Renderer::poll_curve_quality` after bounded completion and require
`pending_curve_quality() == (0, 0)` before qualifying a final frame. Pending
counter downloads are limited to 256 and track exact buffer bytes. A geometry
quality refusal does not raise allocation high-water marks or retry at a coarser
tolerance. Debug captures remain separate from ordinary quality downloads.

Qualification on the AMD Radeon Pro Vega 56 / Metal covers canonical curve and
glyph probes, twelve intact Netrender/Cambium panels and twenty-four production
producer sources, using the existing premultiplied encoded-sRGB RGBA8 gates:
edge RGBA 32 / alpha 16, interior RGBA 3 / alpha 2, mean channel delta 1.
Allocation-recovery and malformed-command forward-progress GPU regressions pass.
The complete font-bearing source bundles remain local; Netrender owns the
published receipts and input digests.

The finer geometry has measured cost in unoptimized fresh-session qualification:
paired full-panel CPU encode/raster median ratio 1.52, Sparse host encode/submit
ratio 2.01. These are host observations, with completion/readback separate for
GPU backends, and are not warm-frame or GPU-timestamp measurements. Classic's
current encode/submit medians are 37.21 ms wide and 21.34 ms narrow; its old input
was not identity-matched, so no Classic regression ratio is claimed.
