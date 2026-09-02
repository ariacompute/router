fn main() {
    println!("cargo:rerun-if-env-changed=ARIA_ROUTER_VERSION");
    let raw = std::env::var("ARIA_ROUTER_VERSION").unwrap_or_else(|_| {
        std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION set by cargo")
    });
    let version = raw.strip_prefix('v').unwrap_or(raw.as_str());
    println!("cargo:rustc-env=ARIA_ROUTER_VERSION={version}");
}
