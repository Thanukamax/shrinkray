//! Rust IPC client for the bundled .NET sidecar.
//!
//! Protocol: newline-delimited JSON on stdin/stdout. One request per line, one
//! response per line, matched by `id`. The sidecar is single-threaded and replies
//! in send order, so we don't need response correlation beyond ID echo.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
struct Request<'a> {
    id: String,
    cmd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Response {
    #[allow(dead_code)]
    id: Option<String>,
    ok: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PingResult {
    pub version: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AssetEntry {
    pub path: String,
    pub size: i64,
    pub extension: String,
    pub compression: String,
    pub encrypted: bool,
    pub is_package: bool,
    pub is_payload: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ListAssetsResult {
    pub pak_path: String,
    pub mount_point: String,
    pub entry_count: i32,
    pub encrypted: bool,
    pub game: String,
    pub entries: Vec<AssetEntry>,
    pub truncated: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ExportInfo {
    pub name: String,
    pub class_name: String,
    pub serial_size: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ImportInfo {
    pub object_name: String,
    pub class_name: String,
    pub outer_package: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CustomVersionEntry {
    pub key: String,
    pub version: i32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MipDescriptor {
    pub index: i32,
    pub width: i32,
    pub height: i32,
    pub byte_size: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TextureInfo {
    pub name: String,
    pub class_name: String,
    pub pixel_format: String,
    pub mip_count: i32,
    pub mips: Vec<MipDescriptor>,
    pub total_bytes: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InspectAssetResult {
    pub pak_path: String,
    pub asset_path: String,
    pub name_count: i32,
    pub import_count: i32,
    pub export_count: i32,
    pub file_version_ue: String,
    pub custom_versions: Vec<CustomVersionEntry>,
    pub exports: Vec<ExportInfo>,
    pub imports: Vec<ImportInfo>,
    #[serde(default)]
    pub textures: Vec<TextureInfo>,
}

/// One mip level — width, height, on-disk bytes. v0.7.2: the planner returns
/// the full mip array per texture so the frontend can re-project locally
/// when the user changes max_dim, instead of round-tripping to the sidecar.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StripMipsLevel {
    pub w: i32,
    pub h: i32,
    pub bytes: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StripMipsItem {
    pub asset_path: String,
    pub export_name: String,
    pub class_name: String,
    pub pixel_format: String,
    pub current_mip0_dim: i32,
    pub kept_mip0_dim: i32,
    pub drop_mip_count: i32,
    pub kept_mip_count: i32,
    pub save_bytes: i64,
    pub original_bytes: i64,
    /// `TC_*` UPROPERTY when the cook serialized it. Drives the
    /// `shrinkray_core::classifier` routing decision (normal-map exemption,
    /// AI vs backup restore class). Many UE4 cooks omit this; downstream
    /// uses name + pixel-format fallbacks.
    #[serde(default)]
    pub compression_settings: Option<String>,
    /// v0.7.2: full mip pyramid for client-side re-projection on max_dim
    /// changes. `#[serde(default)]` so old planner responses without this
    /// field still deserialize (legacy paths fall back to a sidecar call).
    #[serde(default)]
    pub mips: Vec<StripMipsLevel>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ClassCount {
    pub class_name: String,
    pub count: i32,
}

/// v0.6 apply-side request — per-texture targets to strip.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StripTarget {
    pub asset_path: String,
    pub max_dim: i32,
}

/// One modified or original file payload returned by the applier. Bytes are
/// base64-encoded so they fit cleanly in the JSON IPC line.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StripAppliedFile {
    pub pak_path: String,
    pub bytes_base64: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StripAppliedTexture {
    pub asset_path: String,
    pub export_name: String,
    pub drop_mip_count: i32,
    pub kept_mip_count: i32,
    pub original_top_dim: i32,
    pub kept_top_dim: i32,
    pub saved_bytes: i64,
    /// UE pixel-format string (e.g. "PF_DXT5", "PF_BC5"). Always present —
    /// the applier can't strip a texture without parsing FTexturePlatformData.
    #[serde(default)]
    pub pixel_format: String,
    /// UTexture.CompressionSettings UPROPERTY (e.g. "TC_Default",
    /// "TC_Normalmap", "TC_Grayscale"). None when the cook didn't serialize
    /// it — most modern cooks do, even when the value equals the default.
    #[serde(default)]
    pub compression_settings: Option<String>,
    pub files: Vec<StripAppliedFile>,
    pub original_files: Vec<StripAppliedFile>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StripSkipped {
    pub asset_path: String,
    pub reason: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ApplyStripMipsResult {
    pub pak_path: String,
    pub engine_version: String,
    pub applied: Vec<StripAppliedTexture>,
    pub skipped: Vec<StripSkipped>,
    pub total_saved_bytes: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PlanStripMipsResult {
    pub pak_path: String,
    pub max_dim: i32,
    pub scanned_assets: i32,
    pub texture_count: i32,
    pub items: Vec<StripMipsItem>,
    pub total_save_bytes: i64,
    pub total_texture_bytes: i64,
    pub truncated: bool,
    #[serde(default)]
    pub class_histogram: Vec<ClassCount>,
}

pub struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: AtomicU64,
}

impl Sidecar {
    /// Spawn the sidecar binary at `path`. On Linux this is a self-contained ELF.
    pub fn spawn(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherit stderr so panics/exceptions are visible in dev. In production we may
            // want to capture and forward.
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("failed to spawn sidecar at {}", path.display()))?;
        let stdin = child.stdin.take().context("no stdin on sidecar")?;
        let stdout = BufReader::new(child.stdout.take().context("no stdout on sidecar")?);
        Ok(Self { child, stdin, stdout, next_id: AtomicU64::new(1) })
    }

    /// Locate the sidecar binary. Search order:
    ///   1. `SHRINKRAY_SIDECAR_BIN` env var (overrides everything; used in tests and CI)
    ///   2. `<exe_dir>/binaries/sidecar/shrinkray-sidecar` (packaged Tauri build)
    ///   3. `<repo>/src-tauri/binaries/sidecar/shrinkray-sidecar` (post-`scripts/build-sidecar.sh`)
    ///   4. `<repo>/sidecar/ShrinkraySidecar/bin/Debug/net8.0/linux-x64/shrinkray-sidecar`
    ///      (post-`dotnet build`, the day-to-day dev path)
    pub fn locate() -> Result<PathBuf> {
        if let Ok(p) = std::env::var("SHRINKRAY_SIDECAR_BIN") {
            return Ok(PathBuf::from(p));
        }
        let bin_name = if cfg!(windows) { "shrinkray-sidecar.exe" } else { "shrinkray-sidecar" };

        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let c = dir.join("binaries/sidecar").join(bin_name);
                if c.exists() { return Ok(c); }
            }
        }

        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for rel in [
            "../../src-tauri/binaries/sidecar/shrinkray-sidecar",
            "../../sidecar/ShrinkraySidecar/publish/shrinkray-sidecar",
            "../../sidecar/ShrinkraySidecar/bin/Release/net8.0/linux-x64/shrinkray-sidecar",
            "../../sidecar/ShrinkraySidecar/bin/Debug/net8.0/linux-x64/shrinkray-sidecar",
        ] {
            let c = manifest.join(rel);
            if c.exists() { return Ok(c); }
        }

        bail!(
            "could not locate sidecar binary: set SHRINKRAY_SIDECAR_BIN or run scripts/build-sidecar.sh"
        )
    }

    pub fn ping(&mut self) -> Result<PingResult> {
        let result = self.call("ping", None)?;
        Ok(serde_json::from_value(result)?)
    }

    pub fn list_assets(&mut self, pak_path: impl AsRef<Path>) -> Result<ListAssetsResult> {
        self.list_assets_with(pak_path, None, None)
    }

    pub fn list_assets_with(
        &mut self,
        pak_path: impl AsRef<Path>,
        limit: Option<u32>,
        game: Option<&str>,
    ) -> Result<ListAssetsResult> {
        let mut args = serde_json::Map::new();
        args.insert(
            "pak_path".into(),
            Value::String(pak_path.as_ref().display().to_string()),
        );
        if let Some(l) = limit {
            args.insert("limit".into(), Value::from(l));
        }
        if let Some(g) = game {
            args.insert("game".into(), Value::String(g.into()));
        }
        let result = self.call("list_assets", Some(Value::Object(args)))?;
        Ok(serde_json::from_value(result)?)
    }

    pub fn inspect_asset(
        &mut self,
        pak_path: impl AsRef<Path>,
        asset_path: &str,
        game: Option<&str>,
    ) -> Result<InspectAssetResult> {
        let mut args = serde_json::Map::new();
        args.insert(
            "pak_path".into(),
            Value::String(pak_path.as_ref().display().to_string()),
        );
        args.insert("asset_path".into(), Value::String(asset_path.into()));
        if let Some(g) = game {
            args.insert("game".into(), Value::String(g.into()));
        }
        let result = self.call("inspect_asset", Some(Value::Object(args)))?;
        Ok(serde_json::from_value(result)?)
    }

    /// v0.6 real apply path. For each target asset, the sidecar extracts the
    /// triple from the pak via CUE4Parse, opens it via UAssetAPI, attempts the
    /// byte-level mip-tail strip on Export.Extras, regenerates .ubulk, and
    /// returns modified bytes (base64) alongside the original bytes for backup.
    ///
    /// rc1 caveat: targets that hit the in-flight per-mip parser path land in
    /// `skipped` with a diagnostic reason. The framework is wired end-to-end
    /// (extraction → UAssetAPI load → Extras parse → .ubulk regen → write
    /// back); subsequent v0.6.x releases pin down the last few bytes of the
    /// UE4.22 cooked layout to flip skipped → applied.
    pub fn apply_strip_mips(
        &mut self,
        pak_path: impl AsRef<Path>,
        targets: &[StripTarget],
        game: Option<&str>,
        engine_version: Option<&str>,
    ) -> Result<ApplyStripMipsResult> {
        self.apply_strip_mips_with_progress(pak_path, targets, game, engine_version, |_| {})
    }

    /// v0.7.2: streamed-progress variant. `on_progress` fires once per
    /// texture the sidecar finishes — payload shape matches
    /// `ProgressEmitter.Emit(...)` in C# (`current`, `total`, `asset_path`,
    /// `status`, `saved_bytes`, optional `reason`). UI uses it to render a
    /// live counter + progress bar instead of the previous black-box "applying…".
    pub fn apply_strip_mips_with_progress<F>(
        &mut self,
        pak_path: impl AsRef<Path>,
        targets: &[StripTarget],
        game: Option<&str>,
        engine_version: Option<&str>,
        on_progress: F,
    ) -> Result<ApplyStripMipsResult>
    where
        F: FnMut(&Value),
    {
        let mut args = serde_json::Map::new();
        args.insert(
            "pak_path".into(),
            Value::String(pak_path.as_ref().display().to_string()),
        );
        args.insert("targets".into(), serde_json::to_value(targets)?);
        if let Some(g) = game {
            args.insert("game".into(), Value::String(g.into()));
        }
        if let Some(ev) = engine_version {
            args.insert("engine_version".into(), Value::String(ev.into()));
        }
        let result = self.call_with_progress(
            "apply_strip_mips",
            Some(Value::Object(args)),
            on_progress,
        )?;
        Ok(serde_json::from_value(result)?)
    }

    /// Walk every readable package in a pak and project the savings from
    /// capping each texture's top mip dimension to `max_dim`. Read-only.
    /// `game` overrides the CUE4Parse engine version (default GAME_UE5_LATEST);
    /// passing the wrong version causes typed casts (UTexture2D etc.) to fail
    /// and the planner returns zero textures even on cooked content.
    pub fn plan_strip_mips(
        &mut self,
        pak_path: impl AsRef<Path>,
        max_dim: i32,
        limit: Option<i32>,
        game: Option<&str>,
    ) -> Result<PlanStripMipsResult> {
        let mut args = serde_json::Map::new();
        args.insert(
            "pak_path".into(),
            Value::String(pak_path.as_ref().display().to_string()),
        );
        args.insert("max_dim".into(), Value::Number(max_dim.into()));
        if let Some(l) = limit {
            args.insert("limit".into(), Value::Number(l.into()));
        }
        if let Some(g) = game {
            args.insert("game".into(), Value::String(g.into()));
        }
        let result = self.call("plan_strip_mips", Some(Value::Object(args)))?;
        Ok(serde_json::from_value(result)?)
    }

    fn call(&mut self, cmd: &str, args: Option<Value>) -> Result<Value> {
        self.call_with_progress(cmd, args, |_| {})
    }

    /// v0.7.2: like [`Self::call`] but reads any number of intermediate
    /// `{ id, progress: ... }` lines from stdout before the terminal
    /// `{ id, ok, result/error }` line. `on_progress` is called once per
    /// progress payload — Tauri commands hand it a closure that emits a
    /// frontend event, tests hand it a noop, the no-arg [`Self::call`]
    /// hands it a noop too so existing callers don't have to change.
    pub fn call_with_progress<F>(
        &mut self,
        cmd: &str,
        args: Option<Value>,
        mut on_progress: F,
    ) -> Result<Value>
    where
        F: FnMut(&Value),
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = Request { id: id.to_string(), cmd, args };
        let line = serde_json::to_string(&req)?;
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;

        // Distinguish progress vs terminal-response lines by which key the
        // sidecar populated. Progress lines carry `progress`; terminal lines
        // carry `ok` (with `result` or `error`). We keep reading lines until
        // we see the terminal one.
        loop {
            let mut buf = String::new();
            let n = self.stdout.read_line(&mut buf)?;
            if n == 0 {
                bail!("sidecar closed stdout unexpectedly");
            }
            let trimmed = buf.trim_end();
            // Cheap pre-parse: is this a progress line or a result line?
            // Parsing as Value first avoids two `from_str` calls in the
            // common case.
            let v: Value = serde_json::from_str(trimmed)
                .with_context(|| format!("malformed sidecar line: {buf}"))?;
            if let Some(progress) = v.get("progress") {
                on_progress(progress);
                continue;
            }
            // Treat anything without a `progress` field as the terminal
            // response. If `ok` is missing entirely we surface a clear error
            // rather than panicking.
            let resp: Response = serde_json::from_value(v)
                .with_context(|| format!("malformed sidecar response: {buf}"))?;
            if !resp.ok {
                return Err(anyhow!(
                    "sidecar error: {}",
                    resp.error.unwrap_or_else(|| "<no error message>".into())
                ));
            }
            return resp
                .result
                .ok_or_else(|| anyhow!("sidecar returned ok=true with no result"));
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        // Closing stdin signals EOF to the sidecar; it exits on its own.
        // We don't wait — the child becomes a zombie until reaped by init,
        // acceptable for short-lived dev/test usage. Production should call shutdown().
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests;
