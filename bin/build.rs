fn main() {
    println!("cargo:rerun-if-env-changed=ARIA_ROUTER_VERSION");
    // Precedence: explicit ARIA_ROUTER_VERSION (CI sets this from the release
    // tag) > git tag pointing at HEAD (e.g. v0.10.0 -> 0.10.0) >
    // CARGO_PKG_VERSION. This mirrors http/build.rs so the CLI and the
    // dashboard footer agree on the version.
    let raw = if let Ok(v) = std::env::var("ARIA_ROUTER_VERSION") {
        v
    } else if let Some(tag) = git_tag_version() {
        tag
    } else {
        std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION set by cargo")
    };
    let version = raw.strip_prefix('v').unwrap_or(raw.as_str());
    println!("cargo:rustc-env=ARIA_ROUTER_VERSION={version}");
}

/// Version from a git tag pointing at HEAD (e.g. "v0.10.0"), or None.
fn git_tag_version() -> Option<String> {
    let out = std::process::Command::new("git")
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
