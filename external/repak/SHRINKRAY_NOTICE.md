# vendored repak

This directory is a vendored copy of [trumank/repak](https://github.com/trumank/repak)
at commit `e215472c51db69328b1ce77be2db24d24c1d646b` (release v0.2.3) plus a
single local patch.

## Why vendored, not a git submodule

We need a one-line patch on top of upstream (see below). A submodule would
require pushing the patch to a fork we own, which is more moving parts than
the patch is worth. Vendoring the source keeps the repo self-contained and
clonable in one step.

## The patch

`repak/src/data.rs` — `Compression::Zlib` and `Compression::Gzip` swap
`flate2::Compression::fast()` (level 1) → `flate2::Compression::best()`
(level 9), and `Compression::Zstd` swaps the default level (3) for level 19.

Look for the `shrinkray-patch` comment.

### Why the patch matters

UE's pak cooker compresses entries at ~zlib level 9. Upstream repak's level 1
re-emits cooked entries ~13% larger than the original. On a typical AAA-class
pak, that bloat is more than the mip-drop savings shrinkray produces — the
result is a *bigger* pak on disk after "apply".

With the level-9 patch, Pamali T_hairMask03 (4096² DXT5, 13 mips) at a 1024 px
cap nets ~13 MB of real on-disk shrinkage, which matches the original v0.6.0
test corpus expectations.

The compression bump is ~4× slower per block, which is fine for shrinkray's
"click apply, wait a few seconds" UX.

## Upstreaming

A proper fix would expose the compression level via repak's `PakBuilder` API
so shrinkray (or anyone else) can pass `Compression::best()` without forking.
That's a future PR to trumank/repak; until then this vendored patch is the
load-bearing piece for shrinkray's v0.6.2+ write-side.

## Licensing

Upstream repak is dual-licensed Apache-2.0 OR MIT. shrinkray's distribution
of this vendored copy is under the same terms — see `LICENSE-APACHE` and
`LICENSE-MIT` in this directory.
