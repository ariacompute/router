//! Build script: embed `dashboard/dist` into the binary so that a plain
//! `cargo build` ships a self-contained aria-router release that serves the
//! dashboard without relying on an external `dashboard/dist` directory at runtime.
//!
//! If `dashboard/dist` is missing we attempt `npm --prefix dashboard run build`
//! once. If that fails (or npm is unavailable) we emit an empty embed and a
//! warning, so the build still succeeds (the binary then falls back to an
//! on-disk dist or serves the API only).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EMPTY_EMBED: &str = "\
pub struct EmbeddedFile { pub path: &'static str, pub data: &'static [u8] }
pub static DASHBOARD_FILES: &[EmbeddedFile] = &[];
pub static DASHBOARD_INDEX: &[u8] = &[];
pub static DASHBOARD_HAS: bool = false;
";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run when the explicit version override changes so the embedded
    // version string stays in sync (matches bin/build.rs).
    println!("cargo:rerun-if-env-changed=ARIA_ROUTER_VERSION");

    // Build metadata for the dashboard footer (version@commit), mirroring
    // harness/ariaterm's git tag + short commit hash scheme. Exposed via the
    // public /v1/router/version endpoint so the SPA can render it bottom-left.
    //
    // Precedence: explicit ARIA_ROUTER_VERSION (set by CI from the release
    // tag) > git tag pointing at HEAD (e.g. v0.10.0 -> 0.10.0) >
    // CARGO_PKG_VERSION. The git-tag step lets a local `cargo build` on a
    // tagged commit reflect the release version without manual env setup.
    let version = if let Ok(v) = std::env::var("ARIA_ROUTER_VERSION") {
        v
    } else if let Some(tag) = git_tag_version() {
        tag
    } else {
        std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string())
    };
    let version = version.strip_prefix('v').unwrap_or(&version).to_string();
    println!("cargo:rustc-env=ARIA_ROUTER_VERSION={version}");
    let commit = git_short_commit();
    println!("cargo:rustc-env=ARIA_ROUTER_COMMIT={commit}");
    if let Some(git_dir) = git_dir() {
        let head = Path::new(&git_dir).join("HEAD");
        if head.exists() {
            println!("cargo:rerun-if-changed={}", head.display());
        }
        let refs = Path::new(&git_dir).join("refs/heads");
        if refs.exists() {
            println!("cargo:rerun-if-changed={}", refs.display());
        }
        let tags = Path::new(&git_dir).join("refs/tags");
        if tags.exists() {
            println!("cargo:rerun-if-changed={}", tags.display());
        }
        let packed = Path::new(&git_dir).join("packed-refs");
        if packed.exists() {
            println!("cargo:rerun-if-changed={}", packed.display());
        }
    }

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dashboard_dir = Path::new(&manifest).join("..").join("dashboard");
    let dist_dir = dashboard_dir.join("dist");
    let out_path = out_file();

    // Ensure the dist exists; auto-build it on a clean checkout. We only warn
    // when the build genuinely fails — a successful auto-build is silent so
    // `cargo build` stays quiet on fresh checkouts.
    if !dist_dir.join("index.html").exists() {
        if let Err(e) = build_dashboard(&dashboard_dir) {
            println!(
                "cargo:warning=auto-build of dashboard failed ({e}); binary ships without an embedded dashboard. Run `npm --prefix dashboard install && npm --prefix dashboard run build`."
            );
            write_embed(&out_path, EMPTY_EMBED);
            return;
        }
    }

    let resolved = match dist_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            write_embed(&out_path, EMPTY_EMBED);
            return;
        }
    };

    println!("cargo:rerun-if-changed={}", resolved.display());

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect(&resolved, &resolved, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    out.push_str(
        "pub struct EmbeddedFile { pub path: &'static str, pub data: &'static [u8] }\n",
    );
    out.push_str("pub static DASHBOARD_FILES: &[EmbeddedFile] = &[\n");
    for (rel, abs) in &files {
        out.push_str(&format!(
            "    EmbeddedFile {{ path: {:?}, data: include_bytes!({:?}) }},\n",
            rel,
            abs.display().to_string()
        ));
    }
    out.push_str("];\n");

    if let Some((_, idx)) = files.iter().find(|(rel, _)| rel == "index.html") {
        out.push_str(&format!(
            "pub static DASHBOARD_INDEX: &[u8] = include_bytes!({:?});\n",
            idx.display().to_string()
        ));
    } else {
        out.push_str("pub static DASHBOARD_INDEX: &[u8] = &[];\n");
    }
    out.push_str(&format!("pub static DASHBOARD_HAS: bool = {};\n", !files.is_empty()));

    write_embed(&out_path, &out);
}

fn out_file() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("dashboard_embed.rs")
}

/// Build the dashboard SPA into `dist/`. Installs npm dependencies first when
/// `node_modules` is absent, so a clean checkout (or one with deps removed)
/// builds without requiring a manual `npm install` beforehand.
fn build_dashboard(dashboard_dir: &Path) -> Result<(), String> {
    if !dashboard_dir.join("package.json").exists() {
        return Err("dashboard/package.json not found".into());
    }
    if !dashboard_dir.join("node_modules").exists() {
        let status = Command::new("npm")
            .args(["install", "--no-audit", "--no-fund"])
            .current_dir(dashboard_dir)
            .status()
            .map_err(|e| format!("npm install: {e}"))?;
        if !status.success() {
            return Err("npm install failed".into());
        }
    }
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(dashboard_dir)
        .status()
        .map_err(|e| format!("npm run build: {e}"))?;
    if !status.success() {
        return Err("npm run build failed".into());
    }
    Ok(())
}

/// Short (7-char) git commit hash of the source tree, or "unknown" when git
/// is unavailable or this is not a git checkout (e.g. a source tarball).
fn git_short_commit() -> String {
    match Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
    {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                "unknown".to_string()
            } else {
                s
            }
        }
        _ => "unknown".to_string(),
    }
}

/// Version derived from a git tag pointing at HEAD (e.g. "v0.10.0"), or `None`
/// when the current commit isn't tagged. Strips nothing here; the caller strips
/// a leading "v". Returns `None` on any git error so callers fall back safely.
fn git_tag_version() -> Option<String> {
    let out = Command::new("git")
        .args(["tag", "--points-at", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let tag = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())?;
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

/// Absolute path to the `.git` directory, or `None` if not in a git checkout.
fn git_dir() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}


fn write_embed(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, contents).expect("write embedded dashboard source");
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let abs = path.canonicalize().unwrap_or(path);
                out.push((rel, abs));
            }
        }
    }
}
