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
