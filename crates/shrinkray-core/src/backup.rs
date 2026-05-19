//! Step 2 — differential backup + restore.
//!
//! shrinkray refuses to modify a game folder unless a `shrinkray_backup/`
//! sibling exists with a manifest produced by us. Every recorded operation
//! ("about to delete X" / "about to replace X with new bytes") saves the
//! original payload to `shrinkray_backup/payloads/NNNN.bin` and appends an
//! entry to `manifest.json`. `restore()` walks the manifest in reverse and
//! puts every original byte back, then verifies via SHA256.
//!
//! The "differential" name is forward-looking: today every entry stores the
//! *whole* file (sufficient for L10N delete + pak re-emit in Step 3). Step 4
//! introduces sub-file byte ranges when texture/audio recompression starts
//! editing inside `.uexp`/`.ubulk`.
//
// Step 2 exposes the API; only `load` + `status` + `restore` are wired into
// lib.rs Tauri commands. `Backup::new` + `record_*` are dead in the lib build
// until Step 3 plugs them into the L10N stripper / pak trimmer — silence the
// dead-code warnings module-wide for now.
#![allow(dead_code)]

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const BACKUP_DIR: &str = "shrinkray_backup";
const MANIFEST_FILE: &str = "manifest.json";
const PAYLOADS_DIR: &str = "payloads";
const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackupMode {
    /// Only the bytes shrinkray is about to overwrite are saved.
    Differential,
    /// Full copy of the source tree (not implemented in Step 2 — TODO).
    Full,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Delete,
    Replace,
    /// Path did not exist before — shrinkray is about to create it (e.g.
    /// WAV→Opus produces a new `.opus` file). On restore we delete it.
    Create,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// POSIX-style path relative to the game folder root.
    pub path: String,
    pub op: Op,
    pub original_sha256: String,
    pub original_size: u64,
    pub new_sha256: Option<String>,
    pub new_size: Option<u64>,
    /// Path to the saved original bytes, relative to BACKUP_DIR.
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub shrinkray_version: String,
    pub created_at: u64,
    pub root: String,
    pub mode: BackupMode,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Default, Serialize)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub failures: Vec<RestoreFailure>,
}

#[derive(Debug, Serialize)]
pub struct RestoreFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct Backup {
    root: PathBuf,
    backup_dir: PathBuf,
    manifest: Manifest,
}

impl Backup {
    /// Create a fresh backup directory at `<root>/../shrinkray_backup/`.
    /// Refuses if the directory already exists with an existing manifest —
    /// load that one instead.
    pub fn new(root: &Path, mode: BackupMode) -> Result<Self> {
        if mode == BackupMode::Full {
            bail!("full backup mode is not implemented yet (Step 2 supports differential only)");
        }
        let root = root.canonicalize().with_context(|| format!("canonicalize {}", root.display()))?;
        let backup_dir = backup_dir_for(&root);
        if backup_dir.join(MANIFEST_FILE).exists() {
            bail!("backup already exists at {} — call Backup::load instead", backup_dir.display());
        }
        fs::create_dir_all(backup_dir.join(PAYLOADS_DIR))
            .with_context(|| format!("create {}", backup_dir.display()))?;
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            shrinkray_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: now_unix(),
            root: root.to_string_lossy().into_owned(),
            mode,
            entries: Vec::new(),
        };
        let me = Self { root, backup_dir, manifest };
        me.persist_manifest()?;
        Ok(me)
    }

    /// Load an existing backup. Returns an error if no manifest is found.
    pub fn load(root: &Path) -> Result<Self> {
        let root = root.canonicalize().with_context(|| format!("canonicalize {}", root.display()))?;
        let backup_dir = backup_dir_for(&root);
        let manifest_path = backup_dir.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            bail!("no shrinkray_backup at {} — nothing to restore", backup_dir.display());
        }
        let raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?;
        let manifest: Manifest = serde_json::from_str(&raw)
            .with_context(|| format!("parse {}", manifest_path.display()))?;
        if manifest.version != MANIFEST_VERSION {
            bail!(
                "manifest version {} not supported (this build expects {})",
                manifest.version,
                MANIFEST_VERSION,
            );
        }
        Ok(Self { root, backup_dir, manifest })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn entries(&self) -> &[Entry] {
        &self.manifest.entries
    }

    #[allow(dead_code)] // consumed by Step 3 (ProtectedFile wrappers)
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn backup_dir(&self) -> &Path {
        &self.backup_dir
    }

    /// Record an upcoming full-file replacement. Reads the current file's
    /// bytes, saves them as a payload, appends a manifest entry. Caller is
    /// then responsible for actually writing `new_bytes` to `path`.
    pub fn record_full_replace(&mut self, path: &Path, new_bytes: &[u8]) -> Result<()> {
        let abs = self.absolutize(path)?;
        let original = fs::read(&abs).with_context(|| format!("read {}", abs.display()))?;
        let original_sha256 = sha256_hex(&original);
        let original_size = original.len() as u64;
        let payload = self.write_payload(&original)?;
        let entry = Entry {
            path: rel_posix(&abs, &self.root)?,
            op: Op::Replace,
            original_sha256,
            original_size,
            new_sha256: Some(sha256_hex(new_bytes)),
            new_size: Some(new_bytes.len() as u64),
            payload,
        };
        self.manifest.entries.push(entry);
        self.persist_manifest()
    }

    /// Record that the caller is about to create a brand-new file. No payload
    /// is saved because there are no original bytes. On restore the created
    /// file is deleted. Errors if `path` already exists — use record_full_replace
    /// for overwrite cases.
    pub fn record_create(&mut self, path: &Path) -> Result<()> {
        let abs = self.absolutize(path)?;
        if abs.exists() {
            bail!("record_create called for {} but the path already exists", abs.display());
        }
        let entry = Entry {
            path: rel_posix(&abs, &self.root)?,
            op: Op::Create,
            original_sha256: String::new(),
            original_size: 0,
            new_sha256: None,
            new_size: None,
            payload: String::new(),
        };
        self.manifest.entries.push(entry);
        self.persist_manifest()
    }

    /// Record an upcoming file deletion. Saves the original bytes, appends a
    /// manifest entry. Caller then deletes `path`.
    pub fn record_delete(&mut self, path: &Path) -> Result<()> {
        let abs = self.absolutize(path)?;
        let original = fs::read(&abs).with_context(|| format!("read {}", abs.display()))?;
        let original_sha256 = sha256_hex(&original);
        let original_size = original.len() as u64;
        let payload = self.write_payload(&original)?;
        let entry = Entry {
            path: rel_posix(&abs, &self.root)?,
            op: Op::Delete,
            original_sha256,
            original_size,
            new_sha256: None,
            new_size: None,
            payload,
        };
        self.manifest.entries.push(entry);
        self.persist_manifest()
    }

    /// Walk the manifest in reverse and write every saved payload back to its
    /// original path. Hashes each restored file and flags any mismatch — but
    /// does NOT roll back partial restores; the manifest survives so a second
    /// invocation can continue.
    pub fn restore(&self) -> Result<RestoreReport> {
        let mut report = RestoreReport::default();
        for entry in self.manifest.entries.iter().rev() {
            let abs = match self.absolutize(Path::new(&entry.path)) {
                Ok(p) => p,
                Err(_) => {
                    // Path component navigated above the root — treat as fatal failure for this entry.
                    report.failures.push(RestoreFailure {
                        path: entry.path.clone(),
                        reason: "path escapes root".into(),
                    });
                    continue;
                }
            };
            if let Err(e) = self.restore_entry(&abs, entry) {
                report.failures.push(RestoreFailure {
                    path: entry.path.clone(),
                    reason: e.to_string(),
                });
                continue;
            }
            report.restored.push(entry.path.clone());
        }
        Ok(report)
    }

    fn restore_entry(&self, abs: &Path, entry: &Entry) -> Result<()> {
        // Create entries: undo a creation by deleting the file. No payload
        // to verify since there were no original bytes.
        if entry.op == Op::Create {
            if abs.exists() {
                fs::remove_file(abs).with_context(|| format!("remove {}", abs.display()))?;
            }
            return Ok(());
        }

        let payload_path = self.backup_dir.join(&entry.payload);
        let bytes = fs::read(&payload_path)
            .with_context(|| format!("read payload {}", payload_path.display()))?;
        let payload_sha = sha256_hex(&bytes);
        if payload_sha != entry.original_sha256 {
            bail!(
                "payload {} hash mismatch — backup is corrupt (expected {}, got {})",
                payload_path.display(),
                entry.original_sha256,
                payload_sha,
            );
        }
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        fs::write(abs, &bytes).with_context(|| format!("write {}", abs.display()))?;
        let written = fs::read(abs).with_context(|| format!("re-read {}", abs.display()))?;
        let written_sha = sha256_hex(&written);
        if written_sha != entry.original_sha256 {
            bail!(
                "post-restore hash mismatch at {} (expected {}, got {})",
                abs.display(),
                entry.original_sha256,
                written_sha,
            );
        }
        Ok(())
    }

    fn absolutize(&self, path: &Path) -> Result<PathBuf> {
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        // Defensive: reject paths that escape the root using ../ components.
        let mut depth: i32 = 0;
        for c in path.components() {
            use std::path::Component;
            match c {
                Component::ParentDir => depth -= 1,
                Component::Normal(_) => depth += 1,
                _ => {}
            }
            if depth < 0 {
                return Err(anyhow!("path {} escapes root", path.display()));
            }
        }
        Ok(joined)
    }

    fn write_payload(&self, bytes: &[u8]) -> Result<String> {
        let idx = self.manifest.entries.len() + 1;
        let name = format!("{:04}.bin", idx);
        let rel = format!("{}/{}", PAYLOADS_DIR, name);
        let abs = self.backup_dir.join(&rel);
        fs::write(&abs, bytes).with_context(|| format!("write payload {}", abs.display()))?;
        Ok(rel)
    }

    fn persist_manifest(&self) -> Result<()> {
        let path = self.backup_dir.join(MANIFEST_FILE);
        let tmp = self.backup_dir.join(format!("{}.tmp", MANIFEST_FILE));
        let json = serde_json::to_string_pretty(&self.manifest)?;
        fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("rename to {}", path.display()))?;
        Ok(())
    }
}

/// Returns `<parent of root>/shrinkray_backup/<root basename>/`, so a single
/// parent directory can hold backups for multiple sibling game folders.
fn backup_dir_for(root: &Path) -> PathBuf {
    let parent = root.parent().unwrap_or_else(|| Path::new("."));
    let basename = root.file_name().unwrap_or_else(|| std::ffi::OsStr::new("root"));
    parent.join(BACKUP_DIR).join(basename)
}

fn rel_posix(abs: &Path, root: &Path) -> Result<String> {
    let stripped = abs.strip_prefix(root)
        .with_context(|| format!("{} is outside {}", abs.display(), root.display()))?;
    let parts: Vec<String> = stripped
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Public lookup so the frontend can show "is this folder backed up?" without
/// loading the manifest. Returns None if the backup dir or manifest is absent.
pub fn status(root: &Path) -> Option<BackupStatus> {
    let root = root.canonicalize().ok()?;
    let dir = backup_dir_for(&root);
    let manifest_path = dir.join(MANIFEST_FILE);
    if !manifest_path.exists() {
        return None;
    }
    let raw = fs::read_to_string(&manifest_path).ok()?;
    let m: Manifest = serde_json::from_str(&raw).ok()?;
    Some(BackupStatus {
        backup_dir: dir.to_string_lossy().into_owned(),
        created_at: m.created_at,
        shrinkray_version: m.shrinkray_version,
        mode: m.mode,
        entry_count: m.entries.len(),
    })
}

#[derive(Debug, Serialize)]
pub struct BackupStatus {
    pub backup_dir: String,
    pub created_at: u64,
    pub shrinkray_version: String,
    pub mode: BackupMode,
    pub entry_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_game(tmp: &Path) -> PathBuf {
        let root = tmp.join("GameFolder");
        fs::create_dir_all(root.join("Content/L10N/fr")).unwrap();
        fs::create_dir_all(root.join("Content/Paks")).unwrap();
        fs::write(root.join("Content/L10N/fr/voice.uasset"), b"french_voice_bytes").unwrap();
        fs::write(root.join("Content/Paks/pakchunk0.pak"), b"original_pak_bytes_12345").unwrap();
        root
    }

    #[test]
    fn creates_manifest_on_init() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let backup = Backup::new(&root, BackupMode::Differential).unwrap();
        assert!(backup.backup_dir().join("manifest.json").exists());
        assert!(backup.backup_dir().join("payloads").exists());
        assert_eq!(backup.manifest().version, 1);
        assert_eq!(backup.manifest().entries.len(), 0);
        assert_eq!(backup.manifest().mode, BackupMode::Differential);
    }

    #[test]
    fn refuses_double_init() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let _ = Backup::new(&root, BackupMode::Differential).unwrap();
        assert!(Backup::new(&root, BackupMode::Differential).is_err());
    }

    #[test]
    fn full_mode_not_implemented() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let err = Backup::new(&root, BackupMode::Full).unwrap_err();
        assert!(err.to_string().contains("full backup"));
    }

    #[test]
    fn replace_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let pak = root.join("Content/Paks/pakchunk0.pak");
        let new_bytes = b"smaller_pak";

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        backup.record_full_replace(&pak, new_bytes).unwrap();
        fs::write(&pak, new_bytes).unwrap();

        assert_eq!(fs::read(&pak).unwrap(), new_bytes);

        let report = backup.restore().unwrap();
        assert_eq!(report.restored, vec!["Content/Paks/pakchunk0.pak"]);
        assert!(report.failures.is_empty());
        assert_eq!(fs::read(&pak).unwrap(), b"original_pak_bytes_12345");
    }

    #[test]
    fn delete_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let voice = root.join("Content/L10N/fr/voice.uasset");

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        backup.record_delete(&voice).unwrap();
        fs::remove_file(&voice).unwrap();
        assert!(!voice.exists());

        let report = backup.restore().unwrap();
        assert!(report.failures.is_empty());
        assert!(voice.exists());
        assert_eq!(fs::read(&voice).unwrap(), b"french_voice_bytes");
    }

    #[test]
    fn restore_recreates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let voice = root.join("Content/L10N/fr/voice.uasset");

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        backup.record_delete(&voice).unwrap();
        fs::remove_dir_all(root.join("Content/L10N/fr")).unwrap();
        assert!(!voice.exists());

        let report = backup.restore().unwrap();
        assert!(report.failures.is_empty());
        assert!(voice.exists());
    }

    #[test]
    fn restore_flags_corrupt_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let voice = root.join("Content/L10N/fr/voice.uasset");

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        backup.record_delete(&voice).unwrap();
        fs::remove_file(&voice).unwrap();
        // Corrupt the saved payload.
        let payload = backup.backup_dir().join(&backup.entries()[0].payload);
        fs::write(&payload, b"garbage").unwrap();

        let report = backup.restore().unwrap();
        assert_eq!(report.restored.len(), 0);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].reason.contains("hash mismatch"));
    }

    #[test]
    fn load_reads_existing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());

        {
            let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
            backup.record_delete(&root.join("Content/L10N/fr/voice.uasset")).unwrap();
        }

        let loaded = Backup::load(&root).unwrap();
        assert_eq!(loaded.entries().len(), 1);
        assert_eq!(loaded.entries()[0].op, Op::Delete);
    }

    #[test]
    fn load_fails_without_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        assert!(Backup::load(&root).is_err());
    }

    #[test]
    fn status_returns_none_when_unbacked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        assert!(status(&root).is_none());
    }

    #[test]
    fn status_returns_summary_when_backed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        {
            let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
            backup.record_delete(&root.join("Content/L10N/fr/voice.uasset")).unwrap();
        }
        let st = status(&root).expect("status should be Some");
        assert_eq!(st.entry_count, 1);
        assert_eq!(st.mode, BackupMode::Differential);
    }

    #[test]
    fn rel_posix_uses_forward_slashes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let abs = root.join("a").join("b").join("c.txt");
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, b"x").unwrap();
        let s = rel_posix(&abs, &root).unwrap();
        assert_eq!(s, "a/b/c.txt");
    }

    #[test]
    fn record_create_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let new_file = root.join("Content/L10N/fr/voice.opus");

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        backup.record_create(&new_file).unwrap();
        fs::write(&new_file, b"newly_created_opus_bytes").unwrap();
        assert!(new_file.exists());

        let report = backup.restore().unwrap();
        assert_eq!(report.restored.len(), 1);
        assert!(report.failures.is_empty());
        assert!(!new_file.exists());
    }

    #[test]
    fn record_create_refuses_existing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let existing = root.join("Content/L10N/fr/voice.uasset");
        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let err = backup.record_create(&existing).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn create_plus_delete_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let wav = root.join("Content/L10N/fr/voice.uasset"); // pretend this is the source
        let opus = root.join("Content/L10N/fr/voice.opus");  // and this is the converted output

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        backup.record_delete(&wav).unwrap();
        fs::remove_file(&wav).unwrap();
        backup.record_create(&opus).unwrap();
        fs::write(&opus, b"opus_payload").unwrap();

        let report = backup.restore().unwrap();
        assert_eq!(report.restored.len(), 2);
        assert!(report.failures.is_empty());
        // After restore: original wav recreated, opus removed.
        assert!(wav.exists());
        assert!(!opus.exists());
        assert_eq!(fs::read(&wav).unwrap(), b"french_voice_bytes");
    }

    #[test]
    fn multiple_entries_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game(tmp.path());
        let voice = root.join("Content/L10N/fr/voice.uasset");
        let pak = root.join("Content/Paks/pakchunk0.pak");

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        backup.record_delete(&voice).unwrap();
        backup.record_full_replace(&pak, b"smaller").unwrap();
        fs::remove_file(&voice).unwrap();
        fs::write(&pak, b"smaller").unwrap();

        let report = backup.restore().unwrap();
        assert_eq!(report.restored.len(), 2);
        assert!(report.failures.is_empty());
        assert!(voice.exists());
        assert_eq!(fs::read(&pak).unwrap(), b"original_pak_bytes_12345");
    }
}
