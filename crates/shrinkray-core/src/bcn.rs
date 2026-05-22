//! v0.7.1 — BCn block-compression en/decode + mip-chain regen.
//!
//! The AI restore round-trip goes:
//!   .ubulk mip bytes (BCn-compressed)
//!     ↓ decode_to_rgba8           [this module]
//!   RGBA8 buffer at (w, h)
//!     ↓ inference::upscale_4x_rgba  [v0.7.0]
//!   RGBA8 buffer at (4w, 4h)
//!     ↓ encode_with_mips           [this module]
//!   Vec<MipLevel>: 4w×4h → 2w×2h → ... → 4×4 BCn blocks
//!     ↓ texture_strip splice       [v0.7.1+]
//!   New .uasset/.uexp/.ubulk
//!
//! image_dds drives both directions. For encoding, its `SurfaceRgba8::encode`
//! generates the entire mip chain in one call (we just hand it the top mip),
//! so the mip-chain regen logic lives in image_dds and we just unpack the
//! per-level slices out of the returned Surface.

use anyhow::{anyhow, bail, Context, Result};
use image_dds::{ImageFormat, Mipmaps, Quality, Surface, SurfaceRgba8};

/// Pixel format identifier shrinkray uses end-to-end. We translate to/from
/// the UE `PF_*` strings (which appear in StripMipsApplier's parsed header
/// and in TextureStripRecord) and image_dds's `ImageFormat` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BcFormat {
    /// PF_DXT1 — BC1 RGB (no alpha). 4 bpp.
    Bc1,
    /// PF_DXT5 — BC3 RGB + interpolated alpha. 8 bpp. Pamali's hair mask format.
    Bc3,
    /// PF_BC5 — two-channel signed/unsigned for normal maps. 8 bpp.
    Bc5,
    /// PF_BC7 — modern high-quality RGB(A). 8 bpp. UE5 default for color.
    Bc7,
}

impl BcFormat {
    /// Parse a UE pixel format string into a [`BcFormat`]. Returns `None` for
    /// formats we don't yet en/decode (BC4, BC6H, anything uncompressed).
    pub fn from_ue_pixel_format(pf: &str) -> Option<Self> {
        match pf {
            "PF_DXT1" => Some(BcFormat::Bc1),
            "PF_DXT5" => Some(BcFormat::Bc3),
            "PF_BC5" => Some(BcFormat::Bc5),
            "PF_BC7" => Some(BcFormat::Bc7),
            _ => None,
        }
    }

    fn to_image_dds(self) -> ImageFormat {
        match self {
            BcFormat::Bc1 => ImageFormat::BC1RgbaUnorm,
            BcFormat::Bc3 => ImageFormat::BC3RgbaUnorm,
            BcFormat::Bc5 => ImageFormat::BC5RgUnorm,
            BcFormat::Bc7 => ImageFormat::BC7RgbaUnorm,
        }
    }

    /// Bytes per 4×4 block. Used to validate `.ubulk` mip sizes against
    /// expected dims before we hand them to the decoder.
    pub fn bytes_per_block(self) -> usize {
        match self {
            BcFormat::Bc1 => 8,
            BcFormat::Bc3 | BcFormat::Bc5 | BcFormat::Bc7 => 16,
        }
    }
}

/// Decode a single BCn-compressed mip into RGBA8 pixels.
///
/// `mip_bytes` is the raw block-compressed data for ONE mip level (no DDS
/// header, no surface metadata) — exactly what `texture_strip` reads out of
/// .ubulk at the mip's `OffsetInFile`/`SizeOnDisk` range.
pub fn decode_to_rgba8(
    format: BcFormat,
    width: u32,
    height: u32,
    mip_bytes: &[u8],
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        bail!("decode_to_rgba8: zero dimension (w={width}, h={height})");
    }
    let blocks_x = ((width + 3) / 4) as usize;
    let blocks_y = ((height + 3) / 4) as usize;
    let expected = blocks_x * blocks_y * format.bytes_per_block();
    if mip_bytes.len() != expected {
        bail!(
            "decode_to_rgba8: mip byte length {} doesn't match expected {} for {:?} at {}x{}",
            mip_bytes.len(),
            expected,
            format,
            width,
            height,
        );
    }
    let surface = Surface {
        width,
        height,
        depth: 1,
        layers: 1,
        mipmaps: 1,
        image_format: format.to_image_dds(),
        data: mip_bytes,
    };
    let rgba = surface
        .decode_rgba8()
        .map_err(|e| anyhow!("image_dds decode failed: {e}"))?;
    Ok(rgba.data)
}

/// One mip level of a BCn-encoded texture: dimensions + raw block bytes.
#[derive(Debug, Clone)]
pub struct BcMip {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// Encode an RGBA8 top mip into a full BCn-compressed mip chain.
///
/// Returns one [`BcMip`] per generated level, top-mip first. The mip chain
/// terminates when both dimensions reach the BC block minimum (4×4).
pub fn encode_with_mips(
    format: BcFormat,
    width: u32,
    height: u32,
    rgba8: &[u8],
) -> Result<Vec<BcMip>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| anyhow!("encode_with_mips: dims overflow"))?;
    if rgba8.len() != expected {
        bail!(
            "encode_with_mips: rgba8 length {} doesn't match w×h×4 = {} (w={width}, h={height})",
            rgba8.len(),
            expected,
        );
    }
    if width == 0 || height == 0 {
        bail!("encode_with_mips: zero dimension");
    }

    let input = SurfaceRgba8 {
        width,
        height,
        depth: 1,
        layers: 1,
        mipmaps: 1,
        data: rgba8,
    };
    let encoded = input
        .encode(format.to_image_dds(), Quality::Fast, Mipmaps::GeneratedAutomatic)
        .map_err(|e| anyhow!("image_dds encode failed: {e}"))?;

    // Walk the returned Surface, slicing out each mip's bytes in order.
    let mut mips = Vec::with_capacity(encoded.mipmaps as usize);
    let mut cursor = 0usize;
    for mip_index in 0..encoded.mipmaps {
        let mw = mip_dim(width, mip_index);
        let mh = mip_dim(height, mip_index);
        let blocks_x = ((mw + 3) / 4) as usize;
        let blocks_y = ((mh + 3) / 4) as usize;
        let len = blocks_x * blocks_y * format.bytes_per_block();
        if cursor + len > encoded.data.len() {
            bail!(
                "encode_with_mips: mip {mip_index} ({mw}x{mh}) would read past surface data \
                 (cursor={cursor}, want={len}, total={})",
                encoded.data.len(),
            );
        }
        mips.push(BcMip {
            width: mw,
            height: mh,
            bytes: encoded.data[cursor..cursor + len].to_vec(),
        });
        cursor += len;
    }
    if cursor != encoded.data.len() {
        // Not strictly an error — image_dds might pad — but worth surfacing.
        anyhow::ensure!(
            cursor == encoded.data.len(),
            "encode_with_mips: residual surface bytes after walking mips (cursor={}, total={})",
            cursor,
            encoded.data.len(),
        );
    }
    Ok(mips)
}

fn mip_dim(base: u32, mip_index: u32) -> u32 {
    // UE's mip dim = max(1, base >> mip_index). image_dds caps mip chains at
    // 4×4 (BC block min), so this matches what UE expects from the cooker.
    (base >> mip_index).max(1)
}

/// Generic helper used by callers that don't want to hand-roll
/// `BcFormat::from_ue_pixel_format` + Option handling.
pub fn parse_ue_format(pf: &str) -> Result<BcFormat> {
    BcFormat::from_ue_pixel_format(pf)
        .with_context(|| format!("BCn format not supported by shrinkray: {pf}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: generate a deterministic RGBA gradient at given dims.
    fn rgba_gradient(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                out.push((x % 256) as u8);
                out.push((y % 256) as u8);
                out.push(((x + y) % 256) as u8);
                out.push(255);
            }
        }
        out
    }

    #[test]
    fn ue_pf_string_round_trip() {
        assert_eq!(BcFormat::from_ue_pixel_format("PF_DXT5"), Some(BcFormat::Bc3));
        assert_eq!(BcFormat::from_ue_pixel_format("PF_BC5"), Some(BcFormat::Bc5));
        assert_eq!(BcFormat::from_ue_pixel_format("PF_BC7"), Some(BcFormat::Bc7));
        assert_eq!(BcFormat::from_ue_pixel_format("PF_DXT1"), Some(BcFormat::Bc1));
        assert_eq!(BcFormat::from_ue_pixel_format("PF_UNKNOWN"), None);
    }

    #[test]
    fn bytes_per_block_matches_ue() {
        assert_eq!(BcFormat::Bc1.bytes_per_block(), 8);
        assert_eq!(BcFormat::Bc3.bytes_per_block(), 16);
        assert_eq!(BcFormat::Bc5.bytes_per_block(), 16);
        assert_eq!(BcFormat::Bc7.bytes_per_block(), 16);
    }

    #[test]
    fn encode_bc3_generates_full_mip_chain() {
        // 64×64 generates 7 mips down to 1×1 — same as UE's cooker. Mips
        // below 4×4 (logical) are block-padded to 4×4 on disk, so the byte
        // sizes flatten out at 16 bytes per level for BC3.
        let rgba = rgba_gradient(64, 64);
        let mips = encode_with_mips(BcFormat::Bc3, 64, 64, &rgba).expect("encode ok");
        let dims: Vec<(u32, u32)> = mips.iter().map(|m| (m.width, m.height)).collect();
        assert_eq!(
            dims,
            vec![(64, 64), (32, 32), (16, 16), (8, 8), (4, 4), (2, 2), (1, 1)]
        );
        // Top mip BC3 = (64/4)² blocks × 16 bytes = 4096 bytes.
        assert_eq!(mips[0].bytes.len(), 16 * 16 * 16);
        // 4×4 and below all block-pad to one 4×4 block = 16 bytes for BC3.
        for tail_mip in &mips[4..] {
            assert_eq!(
                tail_mip.bytes.len(),
                16,
                "sub-block mip {:?} should be padded to one block",
                (tail_mip.width, tail_mip.height)
            );
        }
    }

    #[test]
    fn encode_bc7_round_trip_via_decode() {
        // Encode a small gradient, decode it back, check the dims survive.
        // BC7 is lossy even for alpha channels — we don't assert pixel
        // identity, just shape + that the round-trip doesn't catastrophically
        // diverge.
        let rgba = rgba_gradient(16, 16);
        let mips = encode_with_mips(BcFormat::Bc7, 16, 16, &rgba).expect("encode ok");
        let top = &mips[0];
        let decoded = decode_to_rgba8(BcFormat::Bc7, top.width, top.height, &top.bytes)
            .expect("decode ok");
        assert_eq!(decoded.len(), (top.width * top.height * 4) as usize);
        // BC7 alpha can drift by a few values; require it stay near opaque.
        for chunk in decoded.chunks_exact(4) {
            assert!(
                chunk[3] >= 250,
                "alpha drifted too far from 255 (got {})",
                chunk[3]
            );
        }
    }

    #[test]
    fn decode_rejects_mismatched_byte_length() {
        let bogus = vec![0u8; 7]; // BC1 wants 8 bytes for one 4×4 block.
        let err = decode_to_rgba8(BcFormat::Bc1, 4, 4, &bogus).unwrap_err();
        assert!(
            err.to_string().contains("doesn't match"),
            "expected length-mismatch diag, got: {err}"
        );
    }

    #[test]
    fn encode_rejects_mismatched_rgba_length() {
        let too_short = vec![0u8; 10];
        let err = encode_with_mips(BcFormat::Bc3, 4, 4, &too_short).unwrap_err();
        assert!(err.to_string().contains("doesn't match"), "{err}");
    }

    #[test]
    fn parse_ue_format_surfaces_unknown_formats_clearly() {
        let err = parse_ue_format("PF_R8").unwrap_err();
        assert!(err.to_string().contains("PF_R8"), "{err}");
    }
}
