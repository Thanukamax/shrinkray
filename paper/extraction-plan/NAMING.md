# NAMING — picking the public repo + crate name

All five candidates below are unclaimed on crates.io as of 2026-05-22
(checked via `https://crates.io/api/v1/crates/<name>` → HTTP 404 on each).
That's a low bar — domain availability and dictionary collisions matter
more for long-term SEO. Pros/cons follow; recommendation at the bottom.

| Name | crates.io | github.com/<name> | Pitch | Risk |
|------|-----------|-------------------|-------|------|
| `delta-codec` | free | likely taken as user/org but the suffix `-codec` saves us; org name `delta-codec` not checked but unlikely to matter — repo can live under user namespace | Clean, dictionary-sounding. Implies "delta encoding" without overpromising. | "Delta" is overloaded (delta lake, delta updates, delta robotics). High collision with non-codec uses; weak search ranking unless the README earns it. |
| `mip-delta` | free | almost certainly free | Honest. Calls out the actual unit of compression: mip levels. | Niche-sounding. The codec is not strictly tied to mips — pixel-space delta works on any RGBA pair. Locks the brand to a use case that's narrower than the code. |
| `residual-codec` | free | almost certainly free | Most technically accurate name. Residual coding is the entire mechanism. | Reads academic; doesn't sell the "byte-exact restore" hook. Less memorable. |
| `ue-mip-residual` | free | almost certainly free | Maximally honest about provenance — this came out of shrinkray, which is a UE tool. | **Wrong**: the codec is NOT UE-specific. Burning "UE" into the crate name guarantees that nobody in any other ecosystem will look at it, *and* invites the assumption that the code somewhere imports UE-shaped types. We're trying to escape that, not embed it. Reject. |
| `dcdc` | free | likely free as `Thanukamax/dcdc` | Matches the bitstream magic bytes (`DCDC` = 0x44434443). Short, weird, memorable. | Collides with the electronics term ("DC-DC converter") — every search will be polluted with power-supply parts. Domains like `dcdc.dev` near-certainly taken (or expensive). Too short to be greppable. |

## A sixth I want to float: `delta-mip`

Same root as `mip-delta` but flipped reads better in code (`delta_mip::encode`
parses as "encode a delta mip"). crates.io status: **free** (HTTP 404).
github.com namespace: free. Searches won't be polluted because "delta mip"
isn't a phrase that maps to anything else in the field.

Downsides match `mip-delta`: still feels narrower than the code is, given
that pixel-space residual works on any RGBA pair, not just mip levels. But:
the crate's *primary* use case really is mip-level reconstruction (that's
where you get a free low-res version of the same image to seed the
predictor). Honest narrowing matches honest framing.

## Recommendation: **`delta-mip`**

Reasons in priority order:

1. **Honest about the use case.** This crate wins when you already have a
   low-res version of the same image — which in practice means mip pyramids,
   thumbnail-and-original pairs, or scalable image stacks. Calling it
   "delta-mip" sets correct expectations and pre-empts the "why isn't this
   beating PNG cold" complaint that `delta-codec` would invite.
2. **Unambiguous.** Search engines disambiguate "delta mip" cleanly.
   "Delta codec" gets buried under Delta Lake / delta debugging /
   delta robotics. "DCDC" is unsalvageable for SEO.
3. **No false promises.** Avoids the patent-trap of putting "codec" in the
   name (most "codec" projects in Rust have a much bigger surface — readers
   walk in expecting a full encode/decode stack including pixel format
   conversion, color spaces, ICC profiles, etc; this crate is a residual
   layer and shouldn't pretend otherwise).
4. **Future-proof for the BC variant.** `delta-mip::bc_residual` reads
   correctly because BC textures *are* mips. `delta-mip::pixel_residual`
   also reads correctly.
5. **Available.** crates.io + github.com both free.

**Repo name:** `delta-mip` (under user namespace; an org can be set up
later if there's uptake).
**Crate name:** `delta-mip` (matches repo; `cargo add delta-mip` is the
quick-start one-liner).

### Migration touch-up

Inside `src/lib.rs` the module-level doc currently calls the crate
"Δ-Codec". Keep the Δ glyph in the module narrative — it's a nice grace
note in the docstring — but make the crate name on the cover (`delta-mip`)
the canonical one. The two coexist without confusion: the user types
`delta-mip` to install; the docs use Δ as a shorthand once they're inside.

If the user hates `delta-mip` after sleeping on it: fallback is
**`residual-codec`** (technically most accurate, free everywhere) rather
than `delta-codec` (search collision risk).
