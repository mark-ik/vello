# Merely maintenance branch

`main` follows Linebender upstream. `mark-ik/all-vellos` is the existing
wgpu 30 maintenance line for NetRender's Classic, Hybrid and CPU integrations.

The 2026-09-28 consolidation merges the published Classic buffer-recovery
history (`mark-ik/scene-sized-buffers`, through `266c81a2`) with the retained
Hybrid composition history (`ca3f40ea`). It preserves the `netrender-vello`
package name and 0.10.1 manifest version. Published package contents and tags
are unchanged; this merge is not a new package release.

Upstream main was refreshed to `b408cd00`. Its move of Classic under
`research/` and Hybrid's rename to `vello_gpu` are not part of this maintenance
merge. Porting the maintained changes onto that tree needs separate consumer
compatibility checks.

Integration checks:

```text
cargo test --locked -j2 -p vello_tests -p netrender-vello -p vello_hybrid
  --lib --tests -- --test-threads=1
```

The combined tree passed 191 tests; one long-running test remains ignored.
This includes the dynamic-buffer recovery regression, all nine enabled
Classic CPU/GPU comparisons and 126 Hybrid unit tests. The local integration
log is retained outside the retiring worktree at
`Code/testing/mere/vello-maintenance-integration.log`.

## Chrome Radeon coarse-command traversal (2026-10-08)

The Graphshell grouped and flat host-dataset viewers produced opaque black
frames in headed Chrome 152.0.7977.83 on Radeon Pro Vega 56 (`amd`, `gcn-5`).
SwiftShader and native Netrender rectangle controls rendered correctly. GPU
readback showed valid binning, path bounds and tile bitmaps, but coarse output
retained an opening clip without the subsequent draw or closing clip commands.
For a sampled tile, the bitmap slices were `7, 256, 0, 0, 0, 0, 0, 0`: four
draw entries. Temporary counters showed only one processed entry.

Coarse command generation now iterates the eight bitmap slices explicitly,
then clears and consumes each set bit in that slice. It preserves ascending
draw order, skips empty slices and retains clip state across slices. The
previous combined infinite loop with `continue` and `break` is removed. The
diagnostic source substitution makes the original flat Graphshell pixel gate
pass on the hardware adapter without GPU or page errors. This narrows the
failure to that traversal's browser/Metal execution; it does not identify the
exact Tint or driver instruction responsible.

`vello_tests/tests/coarse_bitmap.rs` compares native GPU and CPU output for
empty early slices, bit 31, a later draw that must win, and a clip closing
after multiple slices. It also asserts visible colors and untouched pixels,
so two empty images cannot satisfy the comparison. The unchanged Graphshell
headed scenarios provide the browser consumer regression gate. Source-build
qualification and immutable consumer pins are recorded with that receipt;
browser instrumentation is only a diagnostic and is not shipped.

The source-built viewer at Mere `26f4c19a0` (main `fbc149bd9`, Genet
`15713014`) passes the original nested-group and flat-dataset capture gates
on the default Radeon adapter, without shader hooks or a software adapter.
Provider qualification passed 12 native tests: the two bitmap regressions,
nine enabled CPU/GPU comparisons, and allocation-count buffer recovery. The
existing long-running large-bin comparison remains ignored. Commands:

```text
cargo test --locked -p vello_tests --test coarse_bitmap --test compare_gpu_cpu
  -j2 -- --test-threads=1
cargo test --locked -p vello_tests --test dynamic_memory -j2 -- --test-threads=1
```

This is a fix on the 0.10 maintenance line. It does not adopt the upstream
0.11 release or change the published package version.
