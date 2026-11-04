//! Upgrade CLI binary + libaria-router_ffi from GitHub/Gitee Releases.

use aria_router_config::{ensure_aria_home, lib_dir, load_cli_config};
use futures_util::StreamExt;
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Installed CLI binary name, resolved at compile time so only the target
/// platform's branch exists in the binary (and in coverage).
#[cfg(windows)]
const CLI_BIN_NAME: &str = "aria-router.exe";
#[cfg(not(windows))]
const CLI_BIN_NAME: &str = "aria-router";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseHost {
    GitHub,
    Gitee,
}

/// Test seams for [`run`]: no-ops in production, overridden by unit tests to
/// point at a local release API and a throwaway install target.
#[derive(Debug, Clone, Default)]
pub struct UpgradeOptions {
    /// Overrides the release-list URL (replaces the GitHub/Gitee default).
    pub releases_base: Option<String>,
    /// Overrides the installed CLI path (defaults to `std::env::current_exe()`).
    pub dest_file: Option<PathBuf>,
}

/// Run `aria-router upgrade [version]`.
pub async fn run(version: Option<&str>, current_version: &str) -> io::Result<()> {
    run_with(version, current_version, &UpgradeOptions::default()).await
}

/// [`run`] with explicit overrides, used by tests.
pub async fn run_with(
    version: Option<&str>,
    current_version: &str,
    opts: &UpgradeOptions,
) -> io::Result<()> {
    let cfg = load_cli_config().map_err(|e| io::Error::other(e.to_string()))?;
    let upgrade_url = cfg.upgrade_url.trim();
    if upgrade_url.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "upgrade_url not set; run `aria-router setup` first",
        ));
    }
    let org = upgrade_url.trim_end_matches('/');
    let releases_url = match &opts.releases_base {
        Some(base) => base.clone(),
        None => releases_api_url(detect_host(org)?, org),
    };
    let releases = fetch_releases_url(&releases_url).await?;
    let target = select_release(&releases, version)?;
    let ver = strip_v(&target.tag);
    let current = strip_v(current_version);
    if ver == current {
        println!("already at {ver}");
        return Ok(());
    }

    let asset_os = detect_asset_os()?;
    let cli_name = router_asset_name(&ver, asset_os);
    let ffi_name = format!("libaria-router_ffi_{ver}_{asset_os}.tar.gz");
    let cli_asset = find_asset(&target.assets, &cli_name)?;
    let ffi_asset = find_asset(&target.assets, &ffi_name)?;

    let home = ensure_aria_home().map_err(|e| io::Error::other(e.to_string()))?;
    let staging = home.join(format!("tmp/upgrade-{ver}"));
    prepare_staging(&staging)?;

    let cli_archive = staging.join(&cli_name);
    let ffi_archive = staging.join(&ffi_name);
    download_url(
        &cli_asset.download_url,
        &cli_archive,
        &format!("router {ver}"),
    )
    .await?;
    download_url(
        &ffi_asset.download_url,
        &ffi_archive,
        &format!("libaria-router_ffi {ver}"),
    )
    .await?;

    let cli_extract = staging.join("router");
    let ffi_extract = staging.join("ffi");
    fs::create_dir_all(&cli_extract)?;
    fs::create_dir_all(&ffi_extract)?;
    extract_archive(&cli_archive, &cli_extract)?;
    extract_archive(&ffi_archive, &ffi_extract)?;

    let dest = install_target(opts)?;
    install_cli_to(&cli_extract, &dest)?;
    install_ffi(&ffi_extract)?;

    let _ = fs::remove_dir_all(&staging);
    println!("upgraded to {ver}");
    println!(
        "libaria-router_ffi installed under {} (set ARIA_ROUTER_FFI_LIB if needed)",
        lib_dir()
            .map_err(|e| io::Error::other(e.to_string()))?
            .display()
    );
    Ok(())
}

/// Recreate the staging directory, clearing a leftover tree from a failed run.
fn prepare_staging(staging: &Path) -> io::Result<()> {
    if staging.exists() {
        fs::remove_dir_all(staging)?;
    }
    fs::create_dir_all(staging)
}

fn detect_host(org_url: &str) -> io::Result<ReleaseHost> {
    let lower = org_url.to_ascii_lowercase();
    if lower.contains("gitee.com") {
        Ok(ReleaseHost::Gitee)
    } else if lower.contains("github.com") {
        Ok(ReleaseHost::GitHub)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unsupported upgrade_url host: {org_url}"),
        ))
    }
}

fn releases_api_url(host: ReleaseHost, org: &str) -> String {
    let owner = org.rsplit('/').next().unwrap_or("ariacompute");
    match host {
        ReleaseHost::GitHub => {
            format!("https://api.github.com/repos/{owner}/router/releases?per_page=30")
        }
        ReleaseHost::Gitee => {
            format!("https://gitee.com/api/v5/repos/{owner}/router/releases?per_page=30")
        }
    }
}

/// Where the CLI binary gets installed: `--dest` override or this executable.
fn install_target(opts: &UpgradeOptions) -> io::Result<PathBuf> {
    match &opts.dest_file {
        Some(p) => Ok(p.clone()),
        None => std::env::current_exe(),
    }
}

/// Fetch + decode the release list from an explicit URL (testable seam).
async fn fetch_releases_url(url: &str) -> io::Result<Vec<ReleaseInfo>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(format!(
            "aria-router-upgrade/{}",
            env!("ARIA_ROUTER_VERSION")
        ))
        .build()
        .map_err(io_err)?;
    let resp = client.get(url).send().await.map_err(io_err)?;
    if !resp.status().is_success() {
        return Err(io::Error::other(format!(
            "releases API {}: {}",
            resp.status(),
            url
        )));
    }
    let raw: Vec<GhRelease> = resp.json().await.map_err(io_err)?;
    Ok(raw
        .into_iter()
        .map(|r| ReleaseInfo {
            tag: r.tag_name,
            prerelease: r.prerelease,
            draft: r.draft,
            assets: r
                .assets
                .into_iter()
                .map(|a| ReleaseAsset {
                    name: a.name,
                    download_url: a.browser_download_url,
                })
                .collect(),
        })
        .collect())
}

pub fn select_release<'a>(
    releases: &'a [ReleaseInfo],
    version: Option<&str>,
) -> io::Result<&'a ReleaseInfo> {
    match version {
        None => releases
            .iter()
            .filter(|r| !r.draft && !r.prerelease)
            .filter(|r| parse_semver(&r.tag).is_some())
            .max_by_key(|r| parse_semver(&r.tag).unwrap_or((0, 0, 0)))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no stable release found")),
        Some(v) => {
            let want = normalize_tag(v);
            releases
                .iter()
                .find(|r| !r.draft && (normalize_tag(&r.tag) == want || r.tag == v))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, format!("release {v} not found"))
                })
        }
    }
}

pub fn parse_semver(tag: &str) -> Option<(u64, u64, u64)> {
    let s = strip_v(tag);
    let core = s.split(['-', '+']).next().unwrap_or("");
    if core.is_empty() {
        return None;
    }
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn normalize_tag(v: &str) -> String {
    let t = v.trim();
    if t.starts_with('v') || t.starts_with('V') {
        t.to_string()
    } else {
        format!("v{t}")
    }
}

pub fn strip_v(v: &str) -> String {
    let t = v.trim();
    t.strip_prefix('v')
        .or_else(|| t.strip_prefix('V'))
        .unwrap_or(t)
        .to_string()
}

pub fn detect_asset_os() -> io::Result<&'static str> {
    // Compile-time selection: only the target's name is part of the binary, so
    // the other platforms' spellings never count as dead code.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok("linux_x86_64");
    }
    #[cfg(all(target_os = "linux", not(target_arch = "x86_64")))]
    {
        return Ok("linux_arm64");
    }
    #[cfg(target_os = "macos")]
    {
        return Ok("macos");
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Ok("windows_x86_64");
    }
    #[allow(unreachable_code)]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "unsupported platform {}/{} for upgrade",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    ))
}

pub fn router_asset_name(version: &str, asset_os: &str) -> String {
    if asset_os.starts_with("windows") {
        format!("aria-router_{version}_{asset_os}.zip")
    } else {
        format!("aria-router_{version}_{asset_os}.tar.gz")
    }
}

fn find_asset<'a>(assets: &'a [ReleaseAsset], name: &str) -> io::Result<&'a ReleaseAsset> {
    assets.iter().find(|a| a.name == name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("release asset not found: {name}"),
        )
    })
}

async fn download_url(url: &str, path: &Path, label: &str) -> io::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(format!(
            "aria-router-upgrade/{}",
            env!("ARIA_ROUTER_VERSION")
        ))
        .build()
        .map_err(io_err)?;
    let resp = client.get(url).send().await.map_err(io_err)?;
    if !resp.status().is_success() {
        return Err(io::Error::other(format!(
            "download failed {}: {}",
            resp.status(),
            url
        )));
    }
    stream_response_to_file(resp, path, label).await?;
    Ok(())
}

async fn stream_response_to_file(
    resp: reqwest::Response,
    path: &Path,
    label: &str,
) -> io::Result<u64> {
    if !label.is_empty() {
        eprintln!("downloading {label}…");
    }
    let mut file = fs::File::create(path)?;
    let mut stream = resp.bytes_stream();
    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(io_err)?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
    }
    file.flush()?;
    Ok(downloaded)
}

fn extract_archive(archive: &Path, dest: &Path) -> io::Result<()> {
    let name = archive
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dest)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown archive type: {name}"),
        ))
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let out_path = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        // Entries live under `dest`, so creating the parent is always safe; a
        // missing parent simply means the archive root.
        fs::create_dir_all(out_path.parent().unwrap_or(dest))?;
        let mut out = fs::File::create(&out_path)?;
        io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(dest)?;
    Ok(())
}

/// Install the CLI binary from `extract_dir` to `dest` (atomically when the
/// filesystem allows a rename, else by copy).
fn install_cli_to(extract_dir: &Path, dest: &Path) -> io::Result<()> {
    let bin_name = CLI_BIN_NAME;
    let src = find_named_file(extract_dir, bin_name)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{bin_name} not in archive"),
        )
    })?;
    let dest_dir = dest
        .parent()
        .ok_or_else(|| io::Error::other("install target has no parent"))?;
    let tmp = dest_dir.join(format!(".aria-router-upgrade-{}.tmp", std::process::id()));
    fs::copy(&src, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tmp)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp, perms)?;
    }
    // The temp file lives next to the target, so this rename is atomic on the
    // same filesystem. A failure (e.g. a locked binary) is reported as-is; the
    // leftover temp file is overwritten by the next upgrade attempt.
    fs::rename(&tmp, dest)?;
    Ok(())
}

fn install_ffi(extract_dir: &Path) -> io::Result<()> {
    let lib = lib_dir().map_err(|e| io::Error::other(e.to_string()))?;
    fs::create_dir_all(&lib)?;
    let names = [
        "libaria-router_ffi.so",
        "libaria-router_ffi.dylib",
        "libaria-router_ffi.a",
        "aria-router_ffi.dll",
        "aria-router_ffi.dll.lib",
        "aria-router_ffi.lib",
    ];
    let mut any = false;
    for name in names {
        if let Some(src) = find_named_file(extract_dir, name)? {
            fs::copy(&src, lib.join(name))?;
            any = true;
        }
    }
    if !any {
        for entry in walk_files(extract_dir)? {
            let name = entry
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if name.contains("aria-router_ffi") || name.contains("libaria-router_ffi") {
                fs::copy(&entry, lib.join(&name))?;
                any = true;
            }
        }
    }
    if !any {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no libaria-router_ffi library found in archive",
        ));
    }
    Ok(())
}

fn find_named_file(root: &Path, name: &str) -> io::Result<Option<PathBuf>> {
    for path in walk_files(root)? {
        if path.file_name().and_then(|s| s.to_str()) == Some(name) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn walk_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    walk(root, &mut out)?;
    Ok(out)
}

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_releases() -> Vec<ReleaseInfo> {
        vec![
            ReleaseInfo {
                tag: "v0.8.0-rc1".into(),
                prerelease: true,
                draft: false,
                assets: vec![],
            },
            ReleaseInfo {
                tag: "v0.7.1".into(),
                prerelease: false,
                draft: false,
                assets: vec![],
            },
            ReleaseInfo {
                tag: "v0.8.0".into(),
                prerelease: false,
                draft: false,
                assets: vec![],
            },
            ReleaseInfo {
                tag: "v0.7.2".into(),
                prerelease: false,
                draft: false,
                assets: vec![
                    ReleaseAsset {
                        name: "aria-router_0.7.2_linux_x86_64.tar.gz".into(),
                        download_url: "https://example/e".into(),
                    },
                    ReleaseAsset {
                        name: "libaria-router_ffi_0.7.2_linux_x86_64.tar.gz".into(),
                        download_url: "https://example/f".into(),
                    },
                ],
            },
            ReleaseInfo {
                tag: "v0.9.1".into(),
                prerelease: false,
                draft: false,
                assets: vec![],
            },
        ]
    }

    #[test]
    fn selects_latest_stable_skipping_prerelease() {
        let r = sample_releases();
        let got = select_release(&r, None).unwrap();
        assert_eq!(got.tag, "v0.9.1");
    }

    #[test]
    fn selects_newest_by_semver_not_api_order() {
        let r = vec![
            ReleaseInfo {
                tag: "v0.8.0".into(),
                prerelease: false,
                draft: false,
                assets: vec![],
            },
            ReleaseInfo {
                tag: "v0.9.1".into(),
                prerelease: false,
                draft: false,
                assets: vec![],
            },
            ReleaseInfo {
                tag: "v0.10.0".into(),
                prerelease: false,
                draft: false,
                assets: vec![],
            },
        ];
        assert_eq!(select_release(&r, None).unwrap().tag, "v0.10.0");
        let rev: Vec<_> = r.into_iter().rev().collect();
        assert_eq!(select_release(&rev, None).unwrap().tag, "v0.10.0");
    }

    #[test]
    fn parse_semver_core() {
        assert_eq!(parse_semver("v0.9.1"), Some((0, 9, 1)));
        assert_eq!(parse_semver("0.8.0"), Some((0, 8, 0)));
        assert_eq!(parse_semver("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_semver("not-a-version"), None);
    }

    #[test]
    fn selects_by_version_with_or_without_v() {
        let r = sample_releases();
        assert_eq!(select_release(&r, Some("0.7.1")).unwrap().tag, "v0.7.1");
        assert_eq!(select_release(&r, Some("v0.7.2")).unwrap().tag, "v0.7.2");
    }

    #[test]
    fn normalize_and_strip_tag() {
        assert_eq!(normalize_tag("0.7.2"), "v0.7.2");
        assert_eq!(normalize_tag("v0.7.2"), "v0.7.2");
        assert_eq!(strip_v("v0.7.2"), "0.7.2");
        assert_eq!(strip_v("0.7.2"), "0.7.2");
    }

    #[test]
    fn router_asset_names() {
        assert_eq!(
            router_asset_name("0.7.2", "linux_x86_64"),
            "aria-router_0.7.2_linux_x86_64.tar.gz"
        );
        assert_eq!(
            router_asset_name("0.7.2", "windows_x86_64"),
            "aria-router_0.7.2_windows_x86_64.zip"
        );
    }

    #[test]
    fn github_and_gitee_api_paths() {
        assert!(
            releases_api_url(ReleaseHost::GitHub, "https://github.com/ariacompute")
                .contains("api.github.com/repos/ariacompute/router/releases")
        );
        assert!(
            releases_api_url(ReleaseHost::Gitee, "https://gitee.com/ariacompute")
                .contains("gitee.com/api/v5/repos/ariacompute/router/releases")
        );
    }

    #[test]
    fn missing_version_errors() {
        let r = sample_releases();
        assert!(select_release(&r, Some("9.9.9")).is_err());
    }

    // ---- end-to-end + archive helpers -------------------------------------

    use std::collections::HashMap;
    use std::sync::Arc;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn with_home<T>(f: impl FnOnce(&Path) -> T) -> T {
        // Same lock as `crate::tests`: `ARIA_COMPUTE_HOME` is process-global.
        let _g = crate::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ARIA_COMPUTE_HOME", dir.path());
        let out = f(dir.path());
        std::env::remove_var("ARIA_COMPUTE_HOME");
        out
    }

    fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut builder = tar::Builder::new(&mut gz);
            for (name, body) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(body.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, name, *body).unwrap();
            }
            builder.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    fn zip_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;

        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in files {
                writer.start_file(*name, opts).unwrap();
                writer.write_all(body).unwrap();
            }
            writer.finish().unwrap();
        }
        buf.into_inner()
    }

    /// Serve a release list (`/releases`) plus asset downloads (`/assets/{name}`).
    async fn spawn_release_server(
        version: &str,
        tag: Option<&str>,
        files: Vec<(String, Vec<u8>)>,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let stored: Arc<HashMap<String, Vec<u8>>> = Arc::new(files.into_iter().collect());
        let assets: Vec<serde_json::Value> = stored
            .keys()
            .map(|n| {
                serde_json::json!({
                    "name": n,
                    "browser_download_url": format!("{base}/assets/{n}")
                })
            })
            .collect();
        let releases = match tag {
            Some(tag) => serde_json::json!([{
                "tag_name": tag,
                "draft": false,
                "prerelease": false,
                "assets": assets,
            }]),
            None => serde_json::json!([]),
        };
        let _ = version;
        let app = axum::Router::new()
            .route(
                "/releases",
                axum::routing::get(move || {
                    let body = releases.clone();
                    async move { axum::Json(body) }
                }),
            )
            .route(
                "/assets/{name}",
                axum::routing::get(
                    move |axum::extract::Path(name): axum::extract::Path<String>| {
                        let files = stored.clone();
                        async move {
                            match files.get(&name) {
                                Some(body) => (axum::http::StatusCode::OK, body.clone()),
                                None => (axum::http::StatusCode::NOT_FOUND, Vec::new()),
                            }
                        }
                    },
                ),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    fn write_cli_config(upgrade_url: &str) {
        aria_router_config::save_cli_config(&aria_router_config::RouterCliConfig {
            upgrade_url: upgrade_url.to_string(),
        })
        .unwrap();
    }

    fn archive_names(version: &str) -> (String, String) {
        let os = detect_asset_os().unwrap();
        (
            router_asset_name(version, os),
            format!("libaria-router_ffi_{version}_{os}.tar.gz"),
        )
    }

    #[test]
    fn upgrades_cli_and_ffi_end_to_end() {
        with_home(|home| {
            let home = home.to_path_buf();
            block_on(async move {
                let version = "9.9.9";
                let (cli_name, ffi_name) = archive_names(version);
                let cli_bytes = tar_gz(&[("aria-router", b"#!/bin/sh\necho new\n")]);
                let ffi_bytes = tar_gz(&[("lib/libaria-router_ffi.so", b"ELF")]);
                let base = spawn_release_server(
                    version,
                    Some("v9.9.9"),
                    vec![(cli_name, cli_bytes), (ffi_name, ffi_bytes)],
                )
                .await;
                write_cli_config("https://github.com/acme/router");

                let dest = home.join("bin/aria-router");
                fs::create_dir_all(dest.parent().unwrap()).unwrap();
                fs::write(&dest, "old").unwrap();

                run_with(
                    None,
                    "0.1.0",
                    &UpgradeOptions {
                        releases_base: Some(format!("{base}/releases")),
                        dest_file: Some(dest.clone()),
                    },
                )
                .await
                .unwrap();

                assert_eq!(fs::read(&dest).unwrap(), b"#!/bin/sh\necho new\n");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
                    assert_eq!(mode, 0o755);
                }
                let lib = lib_dir().unwrap().join("libaria-router_ffi.so");
                assert_eq!(fs::read(&lib).unwrap(), b"ELF");
                // Unknown asset names are reported as 404 by the fake release API.
                let missing = download_url(
                    &format!("{base}/assets/nope.tar.gz"),
                    &home.join("nope.bin"),
                    "missing",
                )
                .await;
                assert!(missing.is_err());

                // Staging directory is cleaned up.
                assert!(!home.join("tmp").join(format!("upgrade-{version}")).exists());
            });
        });
    }

    #[test]
    fn upgrade_reports_already_current() {
        with_home(|_| {
            block_on(async {
                let base = spawn_release_server("1.2.3", Some("v1.2.3"), vec![]).await;
                write_cli_config("https://github.com/acme/router");
                run_with(
                    None,
                    "1.2.3",
                    &UpgradeOptions {
                        releases_base: Some(format!("{base}/releases")),
                        dest_file: None,
                    },
                )
                .await
                .unwrap();
            });
        });
    }

    #[test]
    fn upgrade_requires_configuration() {
        with_home(|_| {
            // No CLI config → empty upgrade_url.
            let err = block_on(run_with(None, "0.1.0", &UpgradeOptions::default())).unwrap_err();
            assert!(err.to_string().contains("upgrade_url not set"), "{err}");

            // Unsupported host.
            write_cli_config("https://example.org/acme/router");
            let err = block_on(run_with(None, "0.1.0", &UpgradeOptions::default())).unwrap_err();
            assert!(err.to_string().contains("unsupported upgrade_url"), "{err}");
        });
    }

    #[test]
    fn upgrade_reports_missing_release_and_assets() {
        with_home(|home| {
            let dest = home.join("aria-router");
            block_on(async {
                write_cli_config("https://github.com/acme/router");
                // Empty release list → no stable release.
                let base = spawn_release_server("2.0.0", None, vec![]).await;
                let err = run_with(
                    None,
                    "0.1.0",
                    &UpgradeOptions {
                        releases_base: Some(format!("{base}/releases")),
                        dest_file: Some(dest.clone()),
                    },
                )
                .await
                .unwrap_err();
                assert_eq!(err.kind(), io::ErrorKind::NotFound);

                // Release without the expected assets.
                let base = spawn_release_server("2.0.0", Some("v2.0.0"), vec![]).await;
                let err = run_with(
                    None,
                    "0.1.0",
                    &UpgradeOptions {
                        releases_base: Some(format!("{base}/releases")),
                        dest_file: Some(dest.clone()),
                    },
                )
                .await
                .unwrap_err();
                assert!(err.to_string().contains("release asset not found"), "{err}");
            });
        });
    }

    #[test]
    fn fetch_releases_url_reports_http_and_decode_errors() {
        block_on(async {
            // Not found.
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let app = axum::Router::new().route(
                "/bad-status",
                axum::routing::get(|| async { (axum::http::StatusCode::OK, "nope") }),
            );
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let err = fetch_releases_url(&format!("http://127.0.0.1:{port}/missing"))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("releases API"), "{err}");

            let err = fetch_releases_url(&format!("http://127.0.0.1:{port}/bad-status"))
                .await
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::Other);

            let err = fetch_releases_url("http://127.0.0.1:1/releases")
                .await
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::Other);
        });
    }

    #[test]
    fn download_url_reports_status_and_connection_errors() {
        block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let dest = dir.path().join("out.bin");
            let err = download_url("http://127.0.0.1:1/x", &dest, "cli")
                .await
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::Other);

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let app = axum::Router::new().route(
                "/missing",
                axum::routing::get(|| async { (axum::http::StatusCode::NOT_FOUND, "gone") }),
            );
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let err = download_url(&format!("http://127.0.0.1:{port}/missing"), &dest, "cli")
                .await
                .unwrap_err();
            assert!(err.to_string().contains("download failed 404"), "{err}");
        });
    }

    #[test]
    fn extract_archive_dispatches_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        fs::create_dir_all(&dest).unwrap();

        let zip_path = dir.path().join("pkg.zip");
        // Includes a directory entry (created as-is) and a traversal entry
        // (`enclosed_name` rejects it, so it is skipped).
        fs::write(
            &zip_path,
            zip_archive(&[("a/b.txt", b"hello"), ("sub/", b""), ("../evil", b"x")]),
        )
        .unwrap();
        extract_archive(&zip_path, &dest).unwrap();
        assert_eq!(fs::read(dest.join("a/b.txt")).unwrap(), b"hello");
        assert!(dest.join("sub").is_dir());
        assert!(!dir.path().join("evil").exists());

        let tgz_path = dir.path().join("pkg.tgz");
        fs::write(&tgz_path, tar_gz(&[("c.txt", b"tar")])).unwrap();
        extract_archive(&tgz_path, &dest).unwrap();
        assert_eq!(fs::read(dest.join("c.txt")).unwrap(), b"tar");

        let unknown = dir.path().join("pkg.rar");
        fs::write(&unknown, b"x").unwrap();
        let err = extract_archive(&unknown, &dest).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // Corrupt archives are reported as invalid data.
        let corrupt = dir.path().join("pkg.zip");
        fs::write(&corrupt, b"not a zip").unwrap();
        let err = extract_archive(&corrupt, &dest).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        let corrupt_tar = dir.path().join("pkg.tar.gz");
        fs::write(&corrupt_tar, b"not gzip").unwrap();
        assert!(extract_archive(&corrupt_tar, &dest).is_err());
    }

    #[test]
    fn prepare_staging_clears_a_leftover_tree() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("tmp/upgrade-1.2.3");
        fs::create_dir_all(staging.join("nested")).unwrap();
        fs::write(staging.join("nested/stale"), b"old").unwrap();
        prepare_staging(&staging).unwrap();
        assert!(staging.is_dir());
        assert!(!staging.join("nested").exists(), "stale tree must be gone");
    }

    #[test]
    fn install_cli_requires_binary_and_replaces_target() {
        let dir = tempfile::tempdir().unwrap();
        let extract = dir.path().join("extract");
        fs::create_dir_all(&extract).unwrap();
        let dest = dir.path().join("installed/aria-router");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();

        let err = install_cli_to(&extract, &dest).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        fs::write(extract.join(CLI_BIN_NAME), b"binary").unwrap();
        install_cli_to(&extract, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"binary");

        // An unusable install target (a directory) is reported, not swallowed.
        let as_dir = dir.path().join("dir-target");
        fs::create_dir_all(&as_dir).unwrap();
        let err = install_cli_to(&extract, &as_dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::IsADirectory);
        // No temp file left behind.
        let leftovers: Vec<_> = fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn install_ffi_prefers_known_names_then_falls_back_to_walk() {
        with_home(|_| {
            let dir = tempfile::tempdir().unwrap();
            // No library at all → NotFound.
            let empty = dir.path().join("empty");
            fs::create_dir_all(&empty).unwrap();
            fs::write(empty.join("readme.txt"), b"x").unwrap();
            let err = install_ffi(&empty).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound);

            // Unknown but recognisable file name → copied via the walk fallback.
            let odd = dir.path().join("odd");
            fs::create_dir_all(odd.join("deep")).unwrap();
            fs::write(odd.join("deep/libaria-router_ffi.so.1"), b"x").unwrap();
            install_ffi(&odd).unwrap();
            let lib = lib_dir().unwrap();
            assert!(lib.join("libaria-router_ffi.so.1").exists());

            // Known name wins.
            let known = dir.path().join("known");
            fs::create_dir_all(&known).unwrap();
            fs::write(known.join("libaria-router_ffi.so"), b"y").unwrap();
            install_ffi(&known).unwrap();
            assert_eq!(fs::read(lib.join("libaria-router_ffi.so")).unwrap(), b"y");
        });
    }

    #[test]
    fn walk_and_find_helpers() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/f.txt"), b"1").unwrap();
        let found = find_named_file(dir.path(), "f.txt").unwrap().unwrap();
        assert!(found.ends_with("f.txt"));
        assert!(find_named_file(dir.path(), "missing.txt")
            .unwrap()
            .is_none());
        assert_eq!(walk_files(dir.path()).unwrap().len(), 1);
        // Reading a missing directory surfaces the OS error.
        assert!(walk_files(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn run_without_cli_config_is_rejected_before_any_network_call() {
        with_home(|_| {
            // Covers the `run` entry point (production calls it from `main`).
            let err = block_on(run(None, "0.1.0")).unwrap_err();
            assert!(err.to_string().contains("upgrade_url not set"), "{err}");
        });
    }

    #[test]
    fn detect_host_maps_known_sites() {
        assert_eq!(
            detect_host("https://gitee.com/acme").unwrap(),
            ReleaseHost::Gitee
        );
        assert_eq!(
            detect_host("https://github.com/acme").unwrap(),
            ReleaseHost::GitHub
        );
        let err = detect_host("https://example.org/acme").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn install_target_defaults_to_the_running_executable() {
        assert_eq!(
            install_target(&UpgradeOptions::default()).unwrap(),
            std::env::current_exe().unwrap()
        );
        let explicit = PathBuf::from("/tmp/somewhere/aria-router");
        assert_eq!(
            install_target(&UpgradeOptions {
                releases_base: None,
                dest_file: Some(explicit.clone()),
            })
            .unwrap(),
            explicit
        );
    }

    #[test]
    fn platform_helpers_are_stable() {
        let os = detect_asset_os().unwrap();
        assert!(matches!(
            os,
            "linux_x86_64" | "linux_arm64" | "macos" | "windows_x86_64"
        ));
        assert_eq!(strip_v("V1.2.3"), "1.2.3");
        assert_eq!(strip_v("1.2.3"), "1.2.3");
        assert_eq!(normalize_tag(" V1.2.3 "), "V1.2.3");
        assert_eq!(normalize_tag("1.2.3"), "v1.2.3");
        assert_eq!(parse_semver("v1"), Some((1, 0, 0)));
        assert_eq!(parse_semver("v"), None);
        assert_eq!(parse_semver(""), None);
        assert_eq!(parse_semver("vx"), None);
        assert_eq!(parse_semver("v1.2.3+build"), Some((1, 2, 3)));
        assert!(select_release(&[], None).is_err());
    }
}
