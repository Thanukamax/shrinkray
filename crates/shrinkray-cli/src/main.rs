//! shrinkray CLI.
//!
//! v0.4 ships the `audit` subcommand (read-only). Future:
//! `analyze`, `backup`, `restore`, `strip`, `recompress` plumbing.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "shrinkray",
    version,
    about = "Cut Unreal Engine game folder size by trimming what you don't need",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Read-only bloat audit of a UE game install.
    ///
    /// Walks the folder and surfaces structural inefficiencies:
    /// patch overlay accumulation, stale version directories, sharded
    /// pak collections, oversized chunks, encryption status, editor
    /// leftovers, launcher language satellites. Never writes a byte.
    Audit {
        /// Path to the game install root.
        path: PathBuf,

        /// Output as JSON instead of Markdown.
        #[arg(long)]
        json: bool,

        /// Write report to this file (default: stdout).
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Audit { path, json, out } => run_audit(path, json, out),
    }
}

fn run_audit(path: PathBuf, json: bool, out: Option<PathBuf>) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("path does not exist: {}", path.display());
    }
    if !path.is_dir() {
        anyhow::bail!("path is not a directory: {}", path.display());
    }

    let report = shrinkray_audit::audit(&path)
        .with_context(|| format!("audit of {}", path.display()))?;

    let rendered = if json {
        serde_json::to_string_pretty(&report).context("serialize JSON")?
    } else {
        shrinkray_audit::render_markdown(&report)
    };

    match out {
        Some(out_path) => {
            std::fs::write(&out_path, &rendered)
                .with_context(|| format!("write {}", out_path.display()))?;
            let format = if json { "JSON" } else { "Markdown" };
            eprintln!(
                "wrote {} report ({} finding(s)) to {}",
                format,
                report.findings.len(),
                out_path.display()
            );
        }
        None => {
            // Use print (not println) so JSON output stays exactly as serialized.
            print!("{}", rendered);
            if !rendered.ends_with('\n') {
                println!();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_audit_with_path() {
        let cli = Cli::try_parse_from(["shrinkray", "audit", "/games/MyGame"]).unwrap();
        match cli.command {
            Commands::Audit { path, json, out } => {
                assert_eq!(path, PathBuf::from("/games/MyGame"));
                assert!(!json);
                assert!(out.is_none());
            }
        }
    }

    #[test]
    fn parses_audit_with_json_flag() {
        let cli =
            Cli::try_parse_from(["shrinkray", "audit", "/games/MyGame", "--json"]).unwrap();
        match cli.command {
            Commands::Audit { json, .. } => assert!(json),
        }
    }

    #[test]
    fn parses_audit_with_out_file() {
        let cli = Cli::try_parse_from([
            "shrinkray",
            "audit",
            "/games/MyGame",
            "--out",
            "report.md",
        ])
        .unwrap();
        match cli.command {
            Commands::Audit { out, .. } => {
                assert_eq!(out, Some(PathBuf::from("report.md")));
            }
        }
    }

    #[test]
    fn rejects_audit_without_path() {
        let r = Cli::try_parse_from(["shrinkray", "audit"]);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_unknown_subcommand() {
        let r = Cli::try_parse_from(["shrinkray", "frobnicate"]);
        assert!(r.is_err());
    }

    #[test]
    fn run_audit_on_empty_dir_writes_markdown_report() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("report.md");
        run_audit(tmp.path().to_path_buf(), false, Some(out.clone())).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("# Bloat Audit"));
        assert!(content.contains("No findings"));
    }

    #[test]
    fn run_audit_emits_json_when_requested() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("report.json");
        run_audit(tmp.path().to_path_buf(), true, Some(out.clone())).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        assert!(parsed.get("root").is_some());
        assert!(parsed.get("aggregate").is_some());
        assert!(parsed.get("findings").is_some());
    }

    #[test]
    fn run_audit_errors_on_missing_path() {
        let missing = PathBuf::from("/this/path/definitely/does/not/exist/9zxk");
        let r = run_audit(missing, false, None);
        assert!(r.is_err());
        let msg = format!("{:#}", r.unwrap_err());
        assert!(msg.contains("does not exist"));
    }

    #[test]
    fn run_audit_errors_on_non_directory() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let r = run_audit(tmp.path().to_path_buf(), false, None);
        assert!(r.is_err());
        let msg = format!("{:#}", r.unwrap_err());
        assert!(msg.contains("not a directory"));
    }
}
