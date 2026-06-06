# delta-codec

A residual-coded image codec with one knob — `quant_step` — that selects
between byte-exact lossless restore and a smaller lossy bitstream, with no
change to the on-disk format. The narrow design goal: ship the smallest
artifact that still reconstructs the original *byte-for-byte* against an
external hash, while leaving a path to ship something even smaller if the
consumer accepts loss. This is a research preview; see Status below.

## What it is

For each top mip you want to compress, the encoder stores a low mip plus a
residual — the per-pixel (or per-BC-byte) delta between the original and
whatever a *predictor* would have reconstructed from the low mip alone.
Decode runs the same predictor and adds the residual back.

- `quant_step == 1` → the residual is stored without quantization. With a
  deterministic predictor + deterministic BC encoder, decode reproduces the
  original byte-for-byte (verified by a SHA-256 receipt baked into the
  bitstream).
- `quant_step > 1` → the residual is uniform-quantized before entropy
  coding. The reconstruction stays within `±(step/2)` per channel; the
  bitstream gets correspondingly smaller.

Two residual spaces are supported:

- **Pixel-space** (`encode_texture` / `decode_texture`) — residual is over
  RGBA8. Best when the predictor is a faithful super-resolver and the
  reconstruction is close to the original in pixel space.
- **BC-byte-space** (`encode_bc_residual` / `decode_bc_residual`) — residual
  is over the BC1/3/5/7-encoded byte stream. Best when pixel-space
  prediction error is large, but BC quantization absorbs most of it
  inside the block search. With a deterministic BC encoder, this variant
  recovers the BC bytes byte-for-byte, which is the strict on-disk
  property downstream consumers (anti-cheat, file-integrity, content
  delivery) tend to care about.

A cheap probe (`probe_codec_space`, O(pixels) high-pass energy) recommends
one or the other per texture.

## What it is not

- **Not a general image codec.** Don't reach for this over PNG/JPEG XL/AVIF
  if you don't already have a low-resolution version of the same image you
  can ship as part of the payload. The premise is "small image + residual",
  not "compress this image cold".
- **Not a video codec.** No motion compensation, no temporal prediction.
  Residual coding here is single-frame.
- **Not faster than zstd alone.** Encode walks every pixel twice (predictor
  + delta). The win is *ratio*, not throughput.
- **Not a novel entropy coder.** zstd does the back-end work. The crate
  measures Shannon entropy of the residual stream so callers can tell how
  close zstd is to the information-theoretic floor for their content; if
  the answer is "not close", swap in a better backend.
- **Not predictor-agnostic by magic.** The decoder must hold the exact same
  predictor the encoder used. The crate ships a bilinear baseline; you wire
  in whatever neural model you want and pin its identity into the bitstream
  via `PredictorId::Onnx { sha256 }`.

## Quick example

```toml
[dependencies]
delta-codec = "0.1"
```

```rust
use delta_codec::{
    BilinearPredictor, box_downsample_2x, encode_texture, decode_texture,
};

// You have a top mip in RGBA8 and a low mip you'd ship anyway.
// (If you don't have a low mip, box_downsample_2x is provided.)
let (top_w, top_h) = (256, 256);
let top_rgba: Vec<u8> = /* your RGBA8 buffer, len = w*h*4 */;
let (low_rgba, low_w, low_h) = box_downsample_2x(&top_rgba, top_w, top_h)?;

let mut predictor = BilinearPredictor;
let bitstream = encode_texture(
    &mut predictor,
    &top_rgba, top_w, top_h,
    low_rgba, low_w, low_h,
    /* quant_step = */ 1,    // 1 = lossless
    /* record_hash = */ true,
)?;

// Ship `bitstream` (serde-serializable). On decode:
let mut predictor = BilinearPredictor;
let restored = decode_texture(&mut predictor, &bitstream)?;
assert_eq!(restored, top_rgba); // byte-exact when q=1 and hash check passes
```

## Predictor interface

Bring your own. The trait is small.

```rust
pub trait Predictor {
    fn predict(
        &mut self,
        low_rgba: &[u8], low_w: u32, low_h: u32,
        top_w: u32, top_h: u32,
    ) -> anyhow::Result<Vec<u8>>;
    fn id(&self) -> PredictorId;
}
```

Pin your model identity via `PredictorId::Onnx { sha256 }`. The decoder
refuses to run if the predictor's id doesn't match the bitstream's; this
catches the silent-corruption case where someone tries to decode with a
different model than was used to encode.

The crate ships `BilinearPredictor` as a transparent baseline + test
fixture. It's not the predictor you want in production — bilinear leaves
large residuals on natural-image content. Wire a 4× upscaler (Real-ESRGAN,
SwinIR, etc.) via your own ONNX runtime and you should see the residual
collapse on predictable content.

## The `quant_step` knob

One byte in the bitstream, value 1..=255.

- `1` — lossless. Residual stored as signed i16 per channel. Decode is
  byte-exact in the residual's coding space (RGBA for `encode_texture`, BC
  bytes for `encode_bc_residual`). The optional SHA-256 receipt catches any
  drift.
- `2..=255` — lossy. Residual divided by `step` (rounded), reconstructed
  via multiply. Max per-channel reconstruction error ≤ `step/2`. Higher
  `step` = smaller residual, less fidelity. The bitstream format does not
  change; one decoder reads everything.

This is the "ONE bitstream" claim: a single artifact format covers both the
byte-exact case (q=1 + hash receipt) and the size-optimised case (q>1).
Whether a given consumer cares about byte-exactness is their decision,
not the codec's.

## Adaptive routing

```rust
use delta_codec::{probe_codec_space, CodecSpace};

let rec = probe_codec_space(&top_rgba, top_w, top_h);
match rec.recommended {
    CodecSpace::Pixel    => /* use encode_texture */,
    CodecSpace::BcByte   => /* use encode_bc_residual */,
}
println!("probe said: {}", rec.basis); // human-readable
```

The probe is a single O(w·h) high-pass pass + a patch-variance tiebreak.
Cheap enough to run per-texture inside a build pipeline. The default
threshold (`HIGH_PASS_THRESHOLD = 25.0`) was tuned against synthetic
benchmark content (smooth gradient / textured gradient / xorshift noise);
re-tune against your own content distribution if your data looks different
from cooked game textures.

## Status

Research preview, v0.1. The thesis — "one bitstream that satisfies both
byte-exact restore and smaller-than-backup distribution" — is implemented
and unit-tested, but the headline measurements against real game-asset
corpora have not been published yet. Numbers ship at:

  *(link to measurements / paper landing page once available)*

Until then, treat this as a falsifiable experiment, not a production codec.
The bitstream format may break between 0.1 and 0.2; magic + version bytes
guard the boundary so old payloads error rather than silently decode wrong.

## License

Apache-2.0. See `LICENSE`. The patent grant matters if you plan to use this
inside a commercial pipeline; we won't sue you over the predictor-+-residual
combination. (We're aware the conceptual idea is decades old in video
coding; the patent grant is over this implementation, not the concept.)

## Contributing

PRs welcome. Two rules:

1. Don't add a dep without explaining why zstd + image_dds + serde aren't
   enough.
2. Don't add a feature whose effect on the bitstream isn't measurable. The
   crate is small on purpose. Adding a knob that the bench can't quantify
   makes the codec worse.

Bug reports: include a minimum reproducer (the codec is pure in-memory, so
a `Vec<u8>` + `(w, h)` is usually enough).
