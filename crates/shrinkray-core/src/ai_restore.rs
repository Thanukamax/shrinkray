//! v0.7 scaffold — AI-driven texture restore planner.
//!
//! This module produces the routing plan only. Actual ONNX inference (Real-ESRGAN
//! x4 or similar) lands in v0.7. The plan is what the v0.6 strip path needs to
//! decide which textures get backed up byte-exact vs which can be regenerated
//! later — the inference itself is post-strip work.
//!
//! Scaffold shape — what's here vs what's not:
//! - Here: classifier-driven routing of a [`TextureFacts`] list into per-texture
//!   restore strategies, with policy override and per-class counts.
//! - Not here: the ONNX runtime ([`ort`](https://crates.io/crates/ort)) crate,
//!   the model fetcher (a separate `scripts/fetch-ai-models.sh`), or the
//!   per-mip upscale + BC re-encode pipeline. Those land in v0.7 proper.

use serde::{Deserialize, Serialize};

use crate::classifier::{classify, Policy, RestoreClass, TextureFacts};

/// One texture's place in the restore plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTexture {
    pub asset_path: String,
    pub export_name: String,
    pub class: RestoreClass,
    /// Pixel format, surfaced so the v0.7 executor can pick the right BC
    /// re-encoder per texture.
    pub pixel_format: String,
}

/// Aggregate plan returned by [`plan_restore_ai`]. The frontend renders this as
/// "stripping 60 textures, restoring 40 via AI, backing up 18, skipping 2."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreAiPlan {
    pub policy: Policy,
    pub textures: Vec<PlannedTexture>,
    pub ai_count: u32,
    pub backup_count: u32,
    pub skip_count: u32,
    /// Whether the AI-upscale execution path is wired up yet. v0.7 scaffold
    /// always returns `false`; the field exists so the frontend can render an
    /// honest "AI restore coming in v0.7" affordance without 404ing on the call.
    pub executor_ready: bool,
    pub executor_phase: String,
    pub executor_notes: Vec<String>,
}

/// Produce a routing plan for a candidate texture set.
pub fn plan_restore_ai(facts_list: &[TextureFacts], policy: Policy) -> RestoreAiPlan {
    let mut textures = Vec::with_capacity(facts_list.len());
    let (mut ai, mut bk, mut sk) = (0u32, 0u32, 0u32);
    for f in facts_list {
        let class = classify(f, policy);
        match class {
            RestoreClass::AiUpscale => ai += 1,
            RestoreClass::ExactBackup => bk += 1,
            RestoreClass::NoStrip => sk += 1,
        }
        textures.push(PlannedTexture {
            asset_path: String::new(),
            export_name: f.name.clone(),
            class,
            pixel_format: f.pixel_format.clone(),
        });
    }
    RestoreAiPlan {
        policy,
        textures,
        ai_count: ai,
        backup_count: bk,
        skip_count: sk,
        executor_ready: false,
        executor_phase: "v0.7".into(),
        executor_notes: vec![
            "ONNX runtime crate (`ort`) not yet wired".into(),
            "Real-ESRGAN x4 model fetcher at scripts/fetch-ai-models.sh".into(),
            "BC re-encoder picks (intel_tex_2) deferred to v0.7".into(),
        ],
    }
}

/// Convenience overload — same as [`plan_restore_ai`] with `Policy::Smart`.
pub fn plan_restore_ai_smart(facts_list: &[TextureFacts]) -> RestoreAiPlan {
    plan_restore_ai(facts_list, Policy::Smart)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, cs: Option<&str>, pf: &str) -> TextureFacts {
        TextureFacts {
            class_name: "Texture2D".into(),
            name: name.into(),
            compression_settings: cs.map(Into::into),
            pixel_format: pf.into(),
        }
    }

    #[test]
    fn plan_routes_mixed_set_correctly() {
        let set = vec![
            f("T_brick_d", Some("TC_Default"), "PF_BC7"),
            f("T_brick_n", Some("TC_Normalmap"), "PF_BC5"),
            f("LUT_grading", Some("TC_LookupTable"), "PF_B8G8R8A8"),
            f("T_grass_d", Some("TC_Default"), "PF_BC1"),
            f("N_hairMark", None, "PF_BC5"),
        ];
        let plan = plan_restore_ai_smart(&set);
        assert_eq!(plan.ai_count, 2);
        assert_eq!(plan.backup_count, 2);
        assert_eq!(plan.skip_count, 1);
        assert!(!plan.executor_ready);
        assert_eq!(plan.executor_phase, "v0.7");
        assert!(plan
            .executor_notes
            .iter()
            .any(|n| n.contains("ONNX")));
    }

    #[test]
    fn conservative_policy_zeros_ai_count() {
        let set = vec![
            f("T_brick_d", Some("TC_Default"), "PF_BC7"),
            f("T_grass_d", Some("TC_Default"), "PF_BC1"),
        ];
        let plan = plan_restore_ai(&set, Policy::Conservative);
        assert_eq!(plan.ai_count, 0);
        assert_eq!(plan.backup_count, 2);
    }

    #[test]
    fn empty_input_returns_empty_plan() {
        let plan = plan_restore_ai_smart(&[]);
        assert_eq!(plan.textures.len(), 0);
        assert_eq!(plan.ai_count + plan.backup_count + plan.skip_count, 0);
    }
}
