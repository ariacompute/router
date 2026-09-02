fn main() {
    println!("cargo:rerun-if-env-changed=ARIAROUTER_VERSION");
    let raw = std::env::var("ARIAROUTER_VERSION").unwrap_or_else(|_| {
        std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION set by cargo")
    });
    let version = raw.strip_prefix('v').unwrap_or(raw.as_str());
    println!("cargo:rustc-env=ARIAROUTER_VERSION={version}");
}
