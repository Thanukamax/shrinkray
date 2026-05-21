//! Texture restore-class classifier.
//!
//! Decides how a given texture should be reversed once stripped:
//! - `AiUpscale` — fine to regenerate via Real-ESRGAN-class upscalers. Diffuse,
//!   roughness, metallic, AO. No backup needed; the stripped pak holds the
//!   smaller mip chain and v0.7's `apply_restore_ai` regenerates the missing
//!   mips at install/restore time.
//! - `ExactBackup` — must be restored byte-exact from a stored mip-tail
//!   payload. Normal maps (BC5 unit vectors), HDR sources, displacement.
//! - `NoStrip` — don't touch in the first place. Lookup tables, editor icons,
//!   anything where the entire data range is meaningful per-pixel.
//!
//! Routing is keyed on UE's `CompressionSettings` enum first (authoritative
//! when the cook populates it) and falls back to a name-prefix heuristic for
//! cooks that drop the property. Override via [`Policy`] for conservative
//! ("always backup") or aggressive ("always AI") global stances.

use serde::{Deserialize, Serialize};

/// Restore strategy chosen for a single texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreClass {
    /// Drop mips, regenerate via AI on restore. Real disk savings, lossy
    /// reconstruction. Default for diffuse/roughness/metallic/AO.
    AiUpscale,
    /// Drop mips, back up the stripped tail bytes, restore exact on demand.
    /// Default for normals / HDR / displacement.
    ExactBackup,
    /// Skip entirely — don't strip this texture.
    NoStrip,
}

/// User-facing policy override applied on top of per-texture routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// Per-texture defaults: AI for diffuse/roughness/etc, backup for normals,
    /// skip for data textures.
    #[default]
    Smart,
    /// Always back up the original mip tail (strongest reversibility, biggest
    /// backup footprint). Useful for AAA / online games where any AI-derived
    /// pixel risks anti-cheat hash divergence.
    Conservative,
    /// Always route to AI restore (smallest backup, accepts lossy normals).
    /// Useful only for singleplayer / asset-flip cleanups.
    Aggressive,
    /// Never strip — useful for "audit mode" preview without committing to a
    /// restore plan yet.
    NeverStrip,
}

/// Inputs the classifier needs to make a routing decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureFacts {
    /// `Texture2D`, `TextureCube`, `VirtualTexture2D`, etc. Used to short-circuit
    /// cube maps (no AI path for those yet).
    pub class_name: String,
    /// Export / asset name. Drives the name-prefix fallback when
    /// `compression_settings` is absent.
    pub name: String,
    /// `TC_*` value from the asset's `CompressionSettings` UPROPERTY, if the
    /// cook serialized it. Many UE4 cooks omit this.
    pub compression_settings: Option<String>,
    /// Pixel format string (`PF_BC5`, `PF_BC7`, etc.). Used as a last-ditch
    /// signal — BC5 strongly correlates with normal maps even when neither
    /// flag nor name hints survived.
    pub pixel_format: String,
}

/// Decide the restore class for a single texture.
///
/// Decision order (first hit wins):
/// 1. Policy override (`NeverStrip` / `Aggressive` / `Conservative`).
/// 2. `compression_settings` if present.
/// 3. Name prefix / suffix heuristics.
/// 4. Pixel-format-based fallback (`PF_BC5` ⇒ likely normal).
/// 5. Default: `AiUpscale`.
pub fn classify(facts: &TextureFacts, policy: Policy) -> RestoreClass {
    match policy {
        Policy::NeverStrip => return RestoreClass::NoStrip,
        Policy::Conservative => return RestoreClass::ExactBackup,
        Policy::Aggressive => {
            // Even in aggressive mode we still hard-skip lookup tables — AI on
            // those produces garbage shader inputs, not just slightly-off pixels.
            if is_data_texture(facts) {
                return RestoreClass::NoStrip;
            }
            return RestoreClass::AiUpscale;
        }
        Policy::Smart => {}
    }

    // 2. Authoritative CompressionSettings flag.
    if let Some(cs) = facts.compression_settings.as_deref() {
        if let Some(decision) = classify_compression_settings(cs) {
            return decision;
        }
    }

    // 3. Name heuristics. Lowercase once so we don't do it repeatedly.
    let name_lc = facts.name.to_ascii_lowercase();
    if is_normal_name(&name_lc) {
        return RestoreClass::ExactBackup;
    }
    if is_data_name(&name_lc) {
        return RestoreClass::NoStrip;
    }

    // 4. Pixel-format last-ditch. BC5 is the dedicated normal-map format —
    // when neither flag nor name survived but the cook chose BC5, treat it as
    // a normal. False positives here are rare (BC5 for non-normal data is
    // unusual) and the cost is just a small backup payload, not a broken game.
    if facts.pixel_format.eq_ignore_ascii_case("PF_BC5") {
        return RestoreClass::ExactBackup;
    }

    RestoreClass::AiUpscale
}

fn classify_compression_settings(cs: &str) -> Option<RestoreClass> {
    // UE's `TC_*` enum. We match case-insensitively because cooks have been
    // observed serializing either form.
    let cs = cs.trim();
    let cs_lc = cs.to_ascii_lowercase();
    match cs_lc.as_str() {
        // Math-encoded / precision-sensitive.
        "tc_normalmap" => Some(RestoreClass::ExactBackup),
        "tc_vectordisplacementmap" | "tc_displacementmap" => Some(RestoreClass::ExactBackup),
        "tc_hdr" | "tc_hdr_compressed" | "tc_hdrcompressed" => Some(RestoreClass::ExactBackup),
        "tc_alpha" => Some(RestoreClass::ExactBackup),
        // Skip-entirely classes.
        "tc_editoricon" => Some(RestoreClass::NoStrip),
        "tc_lookuptable" => Some(RestoreClass::NoStrip),
        // AI-restorable classes.
        "tc_default" | "tc_basecolor" | "tc_diffuse" => Some(RestoreClass::AiUpscale),
        "tc_grayscale" | "tc_masks" => Some(RestoreClass::AiUpscale),
        "tc_roughness" | "tc_metallic" | "tc_ambientocclusion" | "tc_specular" => {
            Some(RestoreClass::AiUpscale)
        }
        _ => None,
    }
}

fn is_normal_name(name_lc: &str) -> bool {
    // Conservative prefixes / suffixes. We avoid bare "n" — too many false
    // positives ("n_arms" was actually a diffuse in one game we checked).
    name_lc.starts_with("n_")
        || name_lc.ends_with("_n")
        || name_lc.ends_with("_normal")
        || name_lc.ends_with("_norm")
        || name_lc.ends_with("_nm")
        || name_lc.contains("_normalmap")
}

fn is_data_name(name_lc: &str) -> bool {
    name_lc.starts_with("lut_")
        || name_lc.ends_with("_lut")
        || name_lc.contains("_lookup")
        || name_lc.contains("colorgrade")
}

fn is_data_texture(facts: &TextureFacts) -> bool {
    let name_lc = facts.name.to_ascii_lowercase();
    if is_data_name(&name_lc) {
        return true;
    }
    matches!(
        facts.compression_settings.as_deref().map(|s| s.to_ascii_lowercase()),
        Some(ref s) if s == "tc_lookuptable" || s == "tc_editoricon"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(name: &str, cs: Option<&str>, pf: &str) -> TextureFacts {
        TextureFacts {
            class_name: "Texture2D".into(),
            name: name.into(),
            compression_settings: cs.map(Into::into),
            pixel_format: pf.into(),
        }
    }

    #[test]
    fn normalmap_routes_to_exact_backup() {
        let f = facts("T_brick", Some("TC_Normalmap"), "PF_BC5");
        assert_eq!(classify(&f, Policy::Smart), RestoreClass::ExactBackup);
    }

    #[test]
    fn diffuse_routes_to_ai() {
        let f = facts("T_brick_d", Some("TC_Default"), "PF_BC7");
        assert_eq!(classify(&f, Policy::Smart), RestoreClass::AiUpscale);
    }

    #[test]
    fn roughness_routes_to_ai() {
        let f = facts("T_brick_r", Some("TC_Roughness"), "PF_BC4");
        assert_eq!(classify(&f, Policy::Smart), RestoreClass::AiUpscale);
    }

    #[test]
    fn lookup_table_skipped() {
        let f = facts("LUT_grading", Some("TC_LookupTable"), "PF_B8G8R8A8");
        assert_eq!(classify(&f, Policy::Smart), RestoreClass::NoStrip);
    }

    #[test]
    fn n_prefix_falls_back_to_backup_when_flag_absent() {
        let f = facts("N_hairMark", None, "PF_BC5");
        assert_eq!(classify(&f, Policy::Smart), RestoreClass::ExactBackup);
    }

    #[test]
    fn bc5_pixel_format_implies_normal_when_signals_absent() {
        // Pamali's `N_*` textures usually drop the CompressionSettings prop;
        // BC5 is the last signal we have and it's reliable.
        let f = facts("MysteryTex", None, "PF_BC5");
        assert_eq!(classify(&f, Policy::Smart), RestoreClass::ExactBackup);
    }

    #[test]
    fn unknown_falls_back_to_ai() {
        let f = facts("RandomThing", None, "PF_BC7");
        assert_eq!(classify(&f, Policy::Smart), RestoreClass::AiUpscale);
    }

    #[test]
    fn conservative_policy_forces_backup_for_diffuse() {
        let f = facts("T_brick_d", Some("TC_Default"), "PF_BC7");
        assert_eq!(classify(&f, Policy::Conservative), RestoreClass::ExactBackup);
    }

    #[test]
    fn aggressive_policy_uses_ai_for_normals() {
        let f = facts("T_brick_n", Some("TC_Normalmap"), "PF_BC5");
        assert_eq!(classify(&f, Policy::Aggressive), RestoreClass::AiUpscale);
    }

    #[test]
    fn aggressive_policy_still_skips_lookup_tables() {
        let f = facts("LUT_grading", Some("TC_LookupTable"), "PF_B8G8R8A8");
        assert_eq!(classify(&f, Policy::Aggressive), RestoreClass::NoStrip);
    }

    #[test]
    fn never_strip_policy_skips_everything() {
        let f = facts("T_brick_d", Some("TC_Default"), "PF_BC7");
        assert_eq!(classify(&f, Policy::NeverStrip), RestoreClass::NoStrip);
    }

    #[test]
    fn case_insensitive_compression_settings() {
        let f_a = facts("T_x", Some("tc_normalmap"), "PF_BC5");
        let f_b = facts("T_x", Some("TC_NormalMap"), "PF_BC5");
        let f_c = facts("T_x", Some("TC_NORMALMAP"), "PF_BC5");
        assert_eq!(classify(&f_a, Policy::Smart), RestoreClass::ExactBackup);
        assert_eq!(classify(&f_b, Policy::Smart), RestoreClass::ExactBackup);
        assert_eq!(classify(&f_c, Policy::Smart), RestoreClass::ExactBackup);
    }
}
