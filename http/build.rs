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

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dashboard_dir = Path::new(&manifest).join("..").join("dashboard");
    let dist_dir = dashboard_dir.join("dist");
    let out_path = out_file();

    // Ensure the dist exists; auto-build it on a clean checkout.
    if !dist_dir.join("index.html").exists() {
        println!(
            "cargo:warning=dashboard/dist missing; attempting `npm --prefix dashboard run build`"
        );
        match Command::new("npm")
            .args(["--prefix", dashboard_dir.to_str().unwrap(), "run", "build"])
            .status()
        {
            Ok(status) if status.success() => {}
            _ => {
                println!(
                    "cargo:warning=auto-build of dashboard failed; binary ships without an embedded dashboard. Run `npm --prefix dashboard run build`."
                );
                write_embed(&out_path, EMPTY_EMBED);
                return;
            }
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
