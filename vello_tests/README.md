# Vello Tests

This folder contains the infrastructure used for testing Vello.
The kinds of test currently used are:

- Property tests
    - These tests are run on both the GPU and CPU.
    - These create scenes with
- Snapshot tests
    - These tests use the GPU shaders as a source of truth, but the CPU shaders are also ran for these tests.
    - These have a non-exact comparison metric, because of small differences between rendering on different platforms.
      This includes differences from "fast math" on Apple platforms.
- Comparison tests
    - These tests compare the results from running a scene through the CPU and GPU pathways.
    - This ensures that the GPU renderer matches the reference CPU renderer.
    - We hope to largely phase these out in favour of additional snapshot tests.

## Image atlas across frames

`image_atlas` renders one retained image with solid-only and empty scenes
between its uses, through the same renderer. It checks every output pixel over
13 frames; no snapshot downloads are required. Run this on the actual adapter:

```sh
cargo test -p vello_tests --test image_atlas -- --nocapture --test-threads=1
```

The test prints the adapter/backend and uses a bounded five-second wait for each
owned readback. The resolver's CPU tests separately check retained atlas extent,
empty upload lists, and an override refresh pending across a patchless frame.

## LFS

We have two groups of snapshot tests.
The first of these groups are the smoke snapshot tests.
This is a small set of tests for which the reference files are included within this repository.
These reference files can be found in `smoke_snapshots`.
These are always required to pass.

We use git Large File Storage for the rest of the snapshot tests.
This is an experiment to determine how suitable git LFS is for our needs.
These tests will detect whether the LFS files failed to download properly, and will pass on CI in that case.
LFS downloads could fail if the Linebender organisation has run out of LFS bandwidth or storage.
If this occurs, we will re-evaluate our LFS based snapshot testing solution.

To run these tests locally, install [git lfs](https://git-lfs.com/), then run `git lfs pull`.
