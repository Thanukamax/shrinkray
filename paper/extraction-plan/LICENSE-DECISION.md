# LICENSE-DECISION — public license for `delta-mip`

The parent shrinkray repo ships under a custom source-available
"No Redistribution" license. That license **cannot** carry over to a public
crate: it forbids the very redistribution that publishing to crates.io
requires, and it would scare off every commercial evaluator on contact.

We need a separate, standards-track license for this crate. Below: the four
realistic candidates, ranked against the actual goals.

## Goals (in priority order)

1. **B2B uptake.** Memory `project_shrinkray_distribution_codec.md` records
   the "FitGirl-but-legit" / install-time recompression opportunity as the
   business angle. That means studios + publishers + storefronts must be
   able to evaluate the crate without dragging their legal department into
   a six-week diligence pass. **A standards-track SPDX license is mandatory.**
2. **Patent grant.** The crate stakes a narrow novelty claim (one-bitstream
   byte-exact + lossy). If we ever file something, or if a downstream
   commercial user is worried about the *idea* not just the code, an
   explicit patent grant from the project to its users defuses that worry.
3. **Compatibility with shrinkray's own license-shaped goals.** Shrinkray
   is source-available. The codec needs to flow back *into* shrinkray
   without conflict. Permissive licenses are one-way-compatible with the
   strictest possible parent license; we keep that option.
4. **Optionality.** We are not yet sure whether `delta-mip` becomes a
   paid SDK, an open MIT toy, or a Mozilla-style hybrid. The license we
   pick at v0.1 should not pre-commit us.

## Candidates

### MIT

- **Pros:** Maximally permissive. Zero friction for any consumer.
  Universally recognised by legal teams. Compatible with shrinkray's
  source-available license (we can vendor MIT code into anything).
- **Cons:** **No patent grant.** A patent-aware commercial user has to do
  their own clearance work. For a codec specifically — where prior-art /
  novelty-claim conversations are normal — this is a real friction point.

### Apache-2.0

- **Pros:** Permissive like MIT but **with an explicit patent grant**
  (§3). The patent termination clause is well-understood and considered
  fair by every legal team that has ever evaluated it. Universally
  pre-cleared at large engineering orgs. The contributor patent grant
  (§3 second paragraph) means downstream contributors automatically license
  their patents to the project — important if collaborators land
  optimisations later. Standards-track, SPDX, OSI-approved.
- **Cons:** Slightly more verbose to vendor (must preserve the LICENSE +
  NOTICE files). Mild incompatibility with GPLv2 (consumers using GPLv2 code
  need GPLv3+); not our problem for *this* crate but worth noting.

### MPL-2.0

- **Pros:** File-level copyleft — modifications to the codec files must be
  shared back, but a downstream binary linking the crate stays
  unconstrained. Patent grant included. Mozilla legal team backing.
- **Cons:** Misreads as "viral" to legal teams that don't read carefully.
  We will get rejected by some commercial evaluators who pattern-match
  "MPL = scary" without verifying. Reduces B2B uptake for no gain over
  Apache-2.0 unless we expect significant downstream forking. We don't,
  and we shouldn't optimise for it.

### BSL (Business Source License, 1.1)

- **Pros:** Lets us reserve commercial use for a window (say 3 years),
  then auto-converts to Apache-2.0 / MIT. Models: Sentry, MariaDB, CockroachDB.
  Would let us sell B2B licenses now and still have an open codec long-term.
- **Cons:** Not OSI-approved. Crates.io accepts it (it's a freeform
  `license-file` field), but `cargo`/`crates.io` users see a non-SPDX
  identifier and the "open source" badge does not light up. Reduces casual
  adoption by 80%+; the codec becomes a closed source-available product,
  not a community library. **Wrong for v0.1** — we have zero downstream
  pressure to monetise yet, and the codec needs proof-of-life uptake before
  it can be a B2B product. BSL kills uptake; revisit at v1.0 if there's
  real demand, not before.

## Recommendation: **Apache-2.0**

Single best-fit. Justification distilled:

- The patent grant matters specifically for *this* crate. We're in
  residual-coding / neural-image-codec territory — every commercial
  evaluator's first question will be "are there patent landmines here?"
  An Apache-2.0 grant doesn't make the answer "no", but it makes the
  answer "you have a defensible position from the project itself".
  That's the difference between a 30-minute legal review and a six-week
  one. The B2B goal lives or dies on this.
- MIT loses on the patent question and gains us nothing in return for a
  technical crate where the patent question is real.
- MPL-2.0 introduces friction without buying anything we want.
- BSL is premature; revisit at v1.0 if there's actual paid uptake.

### Cargo.toml header

```toml
[package]
name        = "delta-mip"
version     = "0.1.0"
edition     = "2021"
license     = "Apache-2.0"
description = "Residual-coded image codec with per-bitstream lossy/lossless selection."
repository  = "https://github.com/<owner>/delta-mip"
authors     = ["Thanuka Sehasna Perera"]
keywords    = ["compression", "codec", "image", "residual", "bcn"]
categories  = ["compression", "multimedia::images"]
```

Drop `license-file` (don't dual-list); the SPDX identifier `Apache-2.0` is
what crates.io's badge system expects.

### Files to add at repo root

- `LICENSE` — the Apache-2.0 text verbatim (download from
  https://www.apache.org/licenses/LICENSE-2.0.txt).
- `NOTICE` — short attribution: `delta-mip — Copyright 2026 Thanuka
  Sehasna Perera`. Apache-2.0 §4(d) requires us to ship a NOTICE if we
  want downstreams to propagate it; even if we don't, it's good hygiene.

### Header comment in source files

Apache convention is a per-file header:

```rust
// Copyright 2026 Thanuka Sehasna Perera
// SPDX-License-Identifier: Apache-2.0
```

Not strictly required (Apache-2.0 §4 only mandates LICENSE + NOTICE in
distribution), but most large consumers' license-scanning tools expect a
per-file SPDX identifier. Add it before publishing v0.1; it's a five-minute
sed pass.

### Compatibility back into shrinkray

Apache-2.0 → shrinkray (source-available proprietary): **fine**. The
shrinkray repo can vendor or depend on Apache-2.0 code without conflict;
the shrinkray license simply governs the larger work it forms a part of,
while the Apache-2.0 LICENSE + NOTICE files travel with the crate's source
inside shrinkray's tree (typically inside `crates/delta-mip/LICENSE` or
similar). This is the standard pattern; every Rust app that ships with
`anyhow` or `serde` does the same thing.
