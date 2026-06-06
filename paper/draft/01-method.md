# Method — full section

**Status:** skeleton. Cross-reference outline §3.
**Owner:** to be filled after measurement run informs which design choices to defend.

## 3.1 Notation

- `T_h` = original top-mip, RGBA8, dimensions `w_h × h_h`
- `T_l` = low-mip on disk after strip, dimensions `w_l × h_l` (typically `(w_h/4, h_h/4)`)
- `P` = predictor, `P(T_l) → T̂_h` at dimensions `w_h × h_h`
- `r` = residual = `T_h − T̂_h`, signed i16 per channel
- `q` = quantized residual = `round(r / s)` where `s` is `quant_step`
- `Z(q)` = zstd-compressed quantized residual
- Bitstream = `(magic, version, P_id, s, w_l, h_l, w_h, h_h, T_l, Z(q), [sha256(T_h)])`

## 3.2 Encode algorithm

```
encode(T_h, T_l, P, s, record_hash):
  T̂_h ← P(T_l, w_h, h_h)
  for i in 0..|T_h|:
    r[i] ← (T_h[i] as i16) − (T̂_h[i] as i16)
    q[i] ← round_to_nearest(r[i] / s)
  Z ← zstd(q.as_bytes(), level=19)
  bs ← Bitstream{
    magic: DCDC, version: 1, predictor: P.id(), quant: s,
    low: (w_l, h_l, T_l), high: (w_h, h_h),
    residual_zst: Z,
    sha256: sha256(T_h) if record_hash else None,
  }
  return bs
```

## 3.3 Decode algorithm

```
decode(bs, P):
  require P.id() == bs.predictor   // else residual lands on wrong baseline
  T̂_h ← P(bs.T_l, bs.w_h, bs.h_h)
  q ← unzstd(bs.residual_zst).as_i16_array()
  for i in 0..|q|:
    r̂[i] ← q[i] * bs.quant
    T_h[i] ← saturate_u8(T̂_h[i] as i16 + r̂[i])
  if bs.sha256: verify sha256(T_h) == bs.sha256
  return T_h
```

**Byte-exact guarantee:** at `s=1`, `q[i] = r[i]` (no rounding loss since residuals are integer-valued), so `r̂[i] = r[i]` and `T̂_h[i] + r[i] = T_h[i]`. Saturation is a no-op because the sum is always in `[0, 255]` by construction.

## 3.4 BC-byte variant (G1, bc_residual.rs)

- Same algorithm but residual computed on BC-encoded bytes:
  - `B_h ← BC_encode(T_h, format)`
  - `B̂_h ← BC_encode(P(T_l), format)`
  - `r ← B_h − B̂_h`
- Recovery: `BC_encode(P(T_l)) + r̂ → B_h` byte-for-byte
- Requires **deterministic BC encoder** — `image_dds`'s BC modes are deterministic under fixed quality settings. Verified by G4 test (`tests/g4_bc_determinism.rs`).

## 3.5 Predictor interface

`trait Predictor`:
- `fn predict(low_rgba, w_l, h_l, w_h, h_h) → Vec<u8>` (RGBA8)
- `fn id() → PredictorId`

Implementations:
- `BilinearPredictor` — baseline, deterministic, no learning
- `EsrganX4Predictor` (forthcoming) — `ort` ONNX runtime, Real-ESRGAN-x4 ONNX model

## 3.6 Adaptive codec-space selector (G13, probe.rs)

- High-pass energy `E_hp = mean(|center − mean(4-neighbours)|)` over RGB channels
- Patch variance `V_p` over 8×8 patch means (tiebreaker)
- Routing rule: `E_hp < 25 → pixel space; else BC-byte space`
- Threshold derived from synthetic samples (smooth ~0.5, textured ~6.5, high-freq noise ~85)
- Cost: O(pixels), negligible vs encode

## 3.7 Quantization tradeoff

- `s=1` → byte-exact (lossless)
- `s>1` → uniform quantization, residual smaller, reconstruction PSNR finite
- No predictor change needed across `s` — knob is pure on the residual
- Open question: nonuniform / channel-adaptive `s` (TODO §5 in outline)

## 3.8 Failure modes

- Predictor mismatch → garbage. Detected by `P_id` field.
- BC encoder nondeterminism (different SIMD, different vendor lib) → byte-exact BC variant breaks. Pinned to `image_dds` deterministic path.
- Saturation at extreme residuals (>255 swing) — impossible since `r ∈ [-255, 255]` and we add to predicted ∈ `[0, 255]` so sum ∈ `[-255, 510]` which saturates correctly to `[0, 255]` only when prediction is wrong by >255 — impossible for any reasonable predictor on RGBA8 input.

---

(Section 4 evaluation, section 5 limitations, section 6 conclusion — see outline)
