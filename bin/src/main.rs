mod upgrade;

use aria_router_config::{
    clear_cli_config, clear_default_config, default_config_path, default_keys_path,
    default_upgrade_url_for_site, default_users_path, lib_dir, load_cli_config, resolve_keys_path,
    resolve_users_path, save_cli_config, RouterCliConfig, RouterDocument,
};
use aria_router_http::{
    data_router, mgmt_router, mgmt_router_serve_dashboard, AppState, KeyStore, LocalUserStore,
};
use clap::{ArgAction, Args, Parser, Subcommand};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

const ROUTER_VERSION: &str = env!("ARIA_ROUTER_VERSION");

#[derive(Parser)]
#[command(
    name = "aria-router",
    about = "OpenAI-compatible routing gateway CLI",
    version = ROUTER_VERSION,
    arg_required_else_help = true,
    disable_version_flag = true
)]
struct Cli {
    /// Print version
    #[arg(short = 'v', long = "version", action = ArgAction::Version)]
    _version: (),
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write router.yml and create admin user
    Setup(Box<SetupArgs>),
    /// Validate router YAML
    Validate {
        /// Config path (default: ~/.ariacompute/router.yml)
        #[arg(long)]
        config: Option<String>,
    },
    /// Start data + management HTTP servers
    Serve {
        /// Config path (default: ~/.ariacompute/router.yml)
        #[arg(long)]
        config: Option<String>,
        /// Data-plane bind address
        #[arg(long)]
        bind: Option<String>,
        /// Management-plane bind address
        #[arg(long, default_value = "127.0.0.1:8080")]
        mgmt_bind: String,
        /// Skip serving Dashboard SPA
        #[arg(long)]
        no_dashboard: bool,
    },
    /// Replace this CLI + libaria-router_ffi from Releases
    Upgrade {
        /// Target version (default: latest stable)
        version: Option<String>,
    },
    /// Print version
    Version,
}

#[derive(Args, Clone)]
struct SetupArgs {
    /// Show config status
    #[arg(long)]
    status: bool,
    /// Remove router.yml (optional keys/users files)
    #[arg(long)]
    clear: bool,
    /// Template: semantic | agent | afm-d
    #[arg(long)]
    template: Option<String>,
    /// Admin username
    #[arg(long)]
    admin_user: Option<String>,
    /// Admin password
    #[arg(long)]
    admin_password: Option<String>,
    /// Gateway backend base_url (all providers)
    #[arg(long)]
    base_url: Option<String>,
    /// Env var name for gateway API key (default GATEWAY_API_KEY)
    #[arg(long)]
    api_key_env: Option<String>,
    /// Gateway API key secret (stored in router.yml backend_refs.api_key)
    #[arg(long)]
    api_key: Option<String>,
    /// Upstream provider_model_id for ariamodel-small
    #[arg(long)]
    model_small: Option<String>,
    /// Upstream provider_model_id for ariamodel-mid
    #[arg(long)]
    model_mid: Option<String>,
    /// Upstream provider_model_id for ariamodel-large
    #[arg(long)]
    model_large: Option<String>,
    /// Agent LLM endpoint (agent template only)
    #[arg(long)]
    agent_endpoint: Option<String>,
    /// Agent LLM logical model (agent template only)
    #[arg(long)]
    agent_model: Option<String>,
    /// Agent fallback logical model (agent template only)
    #[arg(long)]
    agent_fallback: Option<String>,
    /// AFM-D System One endpoint (afm-d template only)
    #[arg(long)]
    afm_d_endpoint: Option<String>,
    /// AFM-D fallback logical model (afm-d template only)
    #[arg(long)]
    afm_d_fallback: Option<String>,
    /// Releases org root for `aria-router upgrade` (written to router-cli.yml)
    #[arg(long)]
    upgrade_url: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let mut io = StdIo;
    if let Err(e) = run_with(cli, &mut io).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// Terminal interaction seam: the real CLI uses stdin/stderr, tests inject a
/// scripted implementation so every prompt branch is exercisable in-process.
trait CliIo {
    fn prompt(&mut self, label: &str) -> io::Result<String>;
    fn prompt_secret(&mut self, label: &str) -> io::Result<String>;
    fn is_tty(&self) -> bool;
    /// Completes when the servers should stop. Production waits forever.
    fn serve_shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
}

struct StdIo;

impl CliIo for StdIo {
    fn prompt(&mut self, label: &str) -> io::Result<String> {
        eprint!("{label}");
        io::stderr().flush()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        Ok(line.trim().to_string())
    }

    fn prompt_secret(&mut self, label: &str) -> io::Result<String> {
        eprint!("{label}");
        io::stderr().flush()?;
        // Masked echo needs termios; other platforms read a plain line below.
        #[cfg(unix)]
        {
            if io::stdin().is_terminal() {
                let secret = read_password_masked()?;
                eprintln!();
                return Ok(secret);
            }
        }
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    /// Real terminal detection on every platform. This must not assume a tty:
    /// piped stdin (CI, scripts) has to fall through to the non-interactive
    /// defaults instead of blocking on prompts.
    fn is_tty(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn serve_shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(std::future::pending())
    }
}

/// Read a secret from the terminal with echo disabled, printing `*` per
/// accepted character. Restores the original terminal attributes on drop.
#[cfg(unix)]
fn read_password_masked() -> io::Result<String> {
    use std::os::fd::AsRawFd;

    let stdin = io::stdin();
    let _restore = mask_termios(stdin.as_raw_fd())?;
    let mut out = String::new();
    read_secret(&mut stdin.lock(), &mut io::stderr(), &mut out)?;
    Ok(out)
}

/// Restores the previous terminal attributes when dropped.
#[cfg(unix)]
struct Restore(libc::termios);

#[cfg(unix)]
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0) };
    }
}

/// Disable echo/canonical mode on `fd`, returning the previous settings.
#[cfg(unix)]
fn mask_termios(fd: libc::c_int) -> io::Result<Restore> {
    use std::mem::MaybeUninit;

    let mut old = MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, old.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let old = unsafe { old.assume_init() };
    let mut raw = old;
    raw.c_lflag &= !(libc::ECHO | libc::ICANON);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    set_termios(fd, &raw)?;
    Ok(Restore(old))
}

/// Apply terminal attributes (`tcsetattr`) to `fd`.
#[cfg(unix)]
fn set_termios(fd: libc::c_int, t: &libc::termios) -> io::Result<()> {
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Byte loop shared by the masked prompt: `*` echo, backspace handling, and
/// stop conditions that do not require a real terminal.
///
/// Only the Unix terminal path calls it in a normal build; the tests exercise it
/// on every platform, so `test` keeps it reachable there (no dead-code warning
/// on Windows).
#[cfg(any(unix, test))]
fn read_secret<R: io::Read, W: io::Write>(
    input: &mut R,
    echo: &mut W,
    out: &mut String,
) -> io::Result<()> {
    let mut buf = [0u8; 1];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        match buf[0] {
            b'\n' | b'\r' => break,
            0x7f | 0x08 => {
                if out.pop().is_some() {
                    echo.write_all(b"\x08 \x08")?;
                    echo.flush()?;
                }
            }
            c if c >= 0x20 && c != 0x7f => {
                out.push(c as char);
                echo.write_all(b"*")?;
                echo.flush()?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve_config(config: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(p) = config {
        return Ok(p);
    }
    let path = default_config_path()?;
    if !path.exists() {
        return Err("missing --config (run aria-router setup)".into());
    }
    Ok(path.display().to_string())
}

fn prompt_opt(
    io: &mut dyn CliIo,
    label: &str,
    flag: Option<String>,
    interactive: bool,
) -> Option<String> {
    if let Some(v) = flag {
        let t = v.trim();
        return if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        };
    }
    if !interactive {
        return None;
    }
    let ans = io.prompt(label).unwrap_or_default();
    let t = ans.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn cmd_setup(args: SetupArgs, io: &mut dyn CliIo) -> Result<(), Box<dyn std::error::Error>> {
    if args.status {
        return setup_status();
    }
    if args.clear {
        return setup_clear(io);
    }

    let template_flag = args.template.is_some();
    let admin_flagged = args.admin_user.is_some() && args.admin_password.is_some();
    // Fully flagged setup (CI): keep gateway defaults unless model flags passed.
    let prompt_models = io.is_tty() && !(template_flag && admin_flagged);

    let raw = args.template.unwrap_or_else(|| {
        io.prompt("template [semantic|agent|afm-d] (default: semantic): ")
            .unwrap_or_default()
    });
    let kind = if raw.is_empty() {
        "semantic".to_string()
    } else {
        raw.to_ascii_lowercase()
    };
    if kind != "semantic" && kind != "agent" && kind != "afm-d" {
        return Err(format!("invalid template: {kind}").into());
    }

    // Prompt hints come from the embedded gateway template (not hardcoded here).
    let (tmpl_small, tmpl_mid, tmpl_large) =
        aria_router_config::template_provider_model_ids(&kind)?;

    let models = aria_router_config::SetupModelOpts {
        base_url: prompt_opt(
            io,
            "gateway base_url [https://tokenhub.tencentmaas.com]: ",
            args.base_url,
            prompt_models,
        ),
        api_key_env: prompt_opt(
            io,
            "api_key_env name [GATEWAY_API_KEY]: ",
            args.api_key_env,
            prompt_models,
        ),
        api_key: {
            // Secret → backend_refs.api_key so serve reads it from router.yml.
            if let Some(v) = args.api_key {
                let t = v.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            } else if prompt_models {
                let ans = io
                    .prompt_secret("gateway API key (saved in router.yml; Enter=env only): ")
                    .unwrap_or_default();
                let t = ans.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            } else {
                None
            }
        },
        small_provider_model_id: prompt_opt(
            io,
            &format!("ariamodel-small provider_model_id [{tmpl_small}]: "),
            args.model_small,
            prompt_models,
        ),
        mid_provider_model_id: prompt_opt(
            io,
            &format!("ariamodel-mid provider_model_id [{tmpl_mid}]: "),
            args.model_mid,
            prompt_models,
        ),
        large_provider_model_id: prompt_opt(
            io,
            &format!("ariamodel-large provider_model_id [{tmpl_large}]: "),
            args.model_large,
            prompt_models,
        ),
        agent_endpoint: if kind == "agent" {
            prompt_opt(
                io,
                "agent.endpoint [same as base_url / tokenhub]: ",
                args.agent_endpoint,
                prompt_models,
            )
        } else {
            None
        },
        agent_model: if kind == "agent" {
            prompt_opt(
                io,
                "agent.model [ariacompute/ariamodel-mid]: ",
                args.agent_model,
                prompt_models,
            )
        } else {
            None
        },
        agent_fallback: if kind == "agent" {
            prompt_opt(
                io,
                "agent.fallback [ariacompute/ariamodel-mid]: ",
                args.agent_fallback,
                prompt_models,
            )
        } else {
            None
        },
        afm_d_endpoint: if kind == "afm-d" {
            prompt_opt(
                io,
                "afm-d.endpoint [http://127.0.0.1:8010]: ",
                args.afm_d_endpoint,
                prompt_models,
            )
        } else {
            None
        },
        afm_d_fallback: if kind == "afm-d" {
            prompt_opt(
                io,
                "afm-d.fallback [ariacompute/ariamodel-mid]: ",
                args.afm_d_fallback,
                prompt_models,
            )
        } else {
            None
        },
    };

    let admin_user = args.admin_user.unwrap_or_else(|| {
        let u = io.prompt("admin username [admin]: ").unwrap_or_default();
        if u.is_empty() {
            "admin".into()
        } else {
            u
        }
    });
    let admin_pass = match args.admin_password {
        Some(p) => p,
        None => {
            let p1 = io.prompt_secret("admin password: ").unwrap_or_default();
            let p2 = io.prompt_secret("confirm password: ").unwrap_or_default();
            if p1 != p2 {
                return Err("passwords do not match".into());
            }
            p1
        }
    };
    if admin_pass.len() < 8 {
        return Err("password must be at least 8 characters".into());
    }

    let path = default_config_path()?;
    let overwrite = if path.exists() {
        if prompt_models || io.is_tty() {
            let ans = io.prompt(&format!("{} exists; overwrite? [y/N]: ", path.display()))?;
            matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes")
        } else {
            // Non-interactive flagged setup: overwrite.
            true
        }
    } else {
        true
    };

    let serve_site = if path.exists() {
        RouterDocument::load_path(&path)
            .map(|d| d.global.serve_site.clone())
            .unwrap_or_else(|_| "com".into())
    } else {
        "com".into()
    };
    let default_upgrade = default_upgrade_url_for_site(&serve_site);
    let existing_cli = load_cli_config().unwrap_or_default();
    let upgrade_url = if let Some(v) = args.upgrade_url {
        let t = v.trim();
        if t.is_empty() {
            if existing_cli.upgrade_url.is_empty() {
                default_upgrade.to_string()
            } else {
                existing_cli.upgrade_url.clone()
            }
        } else {
            t.to_string()
        }
    } else if prompt_models || io.is_tty() {
        let hint = if existing_cli.upgrade_url.is_empty() {
            default_upgrade
        } else {
            existing_cli.upgrade_url.as_str()
        };
        let ans = io.prompt(&format!("upgrade_url (default: {hint}): "))?;
        if ans.trim().is_empty() {
            hint.to_string()
        } else {
            ans.trim().to_string()
        }
    } else if existing_cli.upgrade_url.is_empty() {
        default_upgrade.to_string()
    } else {
        existing_cli.upgrade_url.clone()
    };
    let cli_path = save_cli_config(&RouterCliConfig {
        upgrade_url: upgrade_url.clone(),
    })?;
    println!("wrote {} (upgrade_url={upgrade_url})", cli_path.display());

    if path.exists() && !overwrite {
        println!("kept {}", path.display());
        return Ok(());
    }

    // Defaults: allow_register=true, require_api_key=true (edit YAML or Dashboard later).
    let written =
        aria_router_config::write_default_config_with_opts(&kind, true, true, true, &models)?;
    println!("wrote {}", written.display());

    let users_path = default_users_path()?;
    match LocalUserStore::create_admin(&users_path, &admin_user, &admin_pass) {
        Ok(_) => println!("admin user '{admin_user}' created"),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exist") {
                eprintln!("users already present; kept existing (use --clear to reset)");
            } else {
                return Err(msg.into());
            }
        }
    }

    Ok(())
}

fn setup_status() -> Result<(), Box<dyn std::error::Error>> {
    let path = default_config_path()?;
    println!("config: {}", path.display());
    let kp = if path.exists() {
        let doc = RouterDocument::load_path(&path)?;
        println!("require_api_key: {}", doc.global.require_api_key);
        println!("allow_register: {}", doc.global.allow_register);
        let kp = doc
            .global
            .keys_path
            .clone()
            .unwrap_or_else(|| "~/.ariacompute/router-keys.json".into());
        println!("keys_path: {kp}");
        let resolved = resolve_keys_path(&kp)?;
        if resolved.exists() {
            let (a, r) = aria_router_http::load_keys_for_status(&resolved)?;
            println!("local_api_keys: active={a} revoked={r}");
        } else {
            println!("local_api_keys: (file missing)");
        }
        let up = doc
            .global
            .users_path
            .clone()
            .unwrap_or_else(|| "~/.ariacompute/router-users.json".into());
        let ures = resolve_users_path(&up)?;
        if ures.exists() {
            let store = LocalUserStore::load(&ures)?;
            let (admin, user) = store.counts();
            println!("users: admin={admin} user={user}");
        } else {
            println!("users: (file missing)");
        }
        kp
    } else {
        println!("(missing; run aria-router setup)");
        "~/.ariacompute/router-keys.json".into()
    };
    let cli = load_cli_config().unwrap_or_default();
    if cli.upgrade_url.is_empty() {
        println!("upgrade_url: (not set)");
    } else {
        println!("upgrade_url: {}", cli.upgrade_url);
    }
    println!("lib: {}", lib_dir()?.display());
    let kpath = resolve_keys_path(&kp)?;
    if kpath.exists() {
        let store = KeyStore::load(&kpath).map_err(|e| e.to_string())?;
        let pubu = store.oauth_public();
        println!("site: {}", pubu.site.as_deref().unwrap_or("(none)"));
        if let Some(u) = &pubu.user {
            println!(
                "linked_user: {}",
                u.email.as_deref().unwrap_or("(no email)")
            );
        } else if pubu.api_key_configured {
            println!("linked_user: (not linked — key only)");
        } else {
            println!("linked_user: (none)");
        }
        if pubu.api_key_configured {
            println!(
                "oauth_api_key: configured ({})",
                pubu.api_key_prefix.as_deref().unwrap_or("sk-bf-…")
            );
        } else {
            println!("oauth_api_key: missing");
        }
    } else {
        println!("site: (none)");
        println!("linked_user: (none)");
        println!("oauth_api_key: missing");
    }
    Ok(())
}

fn setup_clear(io: &mut dyn CliIo) -> Result<(), Box<dyn std::error::Error>> {
    let path = clear_default_config()?;
    println!("cleared {}", path.display());
    if let Some(p) = clear_cli_config()? {
        println!("cleared {}", p.display());
    }
    let ans = io.prompt("also delete router-keys.json and router-users.json? [y/N]: ")?;
    if matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes") {
        for p in [default_keys_path()?, default_users_path()?] {
            if p.exists() {
                std::fs::remove_file(&p)?;
                println!("cleared {}", p.display());
            }
        }
    }
    Ok(())
}

async fn run_with(cli: Cli, io: &mut dyn CliIo) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Setup(args) => cmd_setup(*args, io)?,
        Command::Validate { config } => {
            let config = resolve_config(config)?;
            RouterDocument::load_path(&config)?;
            println!("ok");
        }
        Command::Serve {
            config,
            bind,
            mgmt_bind,
            no_dashboard,
        } => {
            let config = resolve_config(config)?;
            let doc = RouterDocument::load_path(&config)?;
            let bind = bind.unwrap_or_else(|| doc.data_bind());
            let mgmt = mgmt_bind;
            let state = Arc::new(AppState::with_path(doc, PathBuf::from(&config)));
            let data = data_router(state.clone());
            println!("data {bind}  mgmt {mgmt}");
            let admin = if no_dashboard {
                mgmt_router(state)
            } else {
                println!("dashboard http://{mgmt}/");
                mgmt_router_serve_dashboard(state)
            };
            let data_l = tokio::net::TcpListener::bind(&bind).await?;
            let mgmt_l = tokio::net::TcpListener::bind(&mgmt).await?;
            let (ds, ms) = (io.serve_shutdown(), io.serve_shutdown());
            serve_two(data_l, data, mgmt_l, admin, ds, ms).await?;
        }
        Command::Upgrade { version } => upgrade::run(version.as_deref(), ROUTER_VERSION)
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?,
        Command::Version => {
            println!("aria-router {ROUTER_VERSION}");
        }
    }
    Ok(())
}

/// Serve the data and management listeners concurrently. The first server that
/// finishes (error or graceful shutdown) ends the process; the other is dropped.
async fn serve_two(
    data_l: tokio::net::TcpListener,
    data: axum::Router,
    mgmt_l: tokio::net::TcpListener,
    admin: axum::Router,
    data_shutdown: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    mgmt_shutdown: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
) -> std::io::Result<()> {
    tokio::select! {
        res = serve_one(data_l, data, data_shutdown) => res,
        res = serve_one(mgmt_l, admin, mgmt_shutdown) => res,
    }
}

/// Serve one plane until graceful shutdown. A named function (rather than an
/// inline `async` block) keeps the tail expression instrumented even when the
/// future is dropped before completion.
async fn serve_one(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    shutdown: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `ARIA_COMPUTE_HOME` is process-global; serialise every test that touches
    /// it (including the `upgrade` module's tests, which share this lock).
    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct FakeIo {
        answers: Vec<String>,
        secrets: Vec<String>,
        tty: bool,
        shutdown: bool,
    }

    impl FakeIo {
        fn new(tty: bool, shutdown: bool) -> Self {
            Self {
                answers: Vec::new(),
                secrets: Vec::new(),
                tty,
                shutdown,
            }
        }

        fn answers(mut self, values: &[&str]) -> Self {
            self.answers = values.iter().map(|s| s.to_string()).collect();
            self
        }

        fn secrets(mut self, values: &[&str]) -> Self {
            self.secrets = values.iter().map(|s| s.to_string()).collect();
            self
        }
    }

    impl CliIo for FakeIo {
        fn prompt(&mut self, _label: &str) -> io::Result<String> {
            if self.answers.is_empty() {
                Ok(String::new())
            } else {
                Ok(self.answers.remove(0))
            }
        }

        fn prompt_secret(&mut self, _label: &str) -> io::Result<String> {
            if self.secrets.is_empty() {
                Ok(String::new())
            } else {
                Ok(self.secrets.remove(0))
            }
        }

        fn is_tty(&self) -> bool {
            self.tty
        }

        fn serve_shutdown(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
            if self.shutdown {
                Box::pin(std::future::ready(()))
            } else {
                Box::pin(std::future::pending())
            }
        }
    }

    /// Current-thread runtime for the sync tests (env vars + `run_with` await).
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn cli(command: Command) -> Cli {
        Cli {
            _version: (),
            command,
        }
    }

    fn setup_args() -> SetupArgs {
        SetupArgs {
            status: false,
            clear: false,
            template: Some("semantic".into()),
            admin_user: Some("admin".into()),
            admin_password: Some("password1".into()),
            base_url: None,
            api_key_env: None,
            api_key: None,
            model_small: None,
            model_mid: None,
            model_large: None,
            agent_endpoint: None,
            agent_model: None,
            agent_fallback: None,
            afm_d_endpoint: None,
            afm_d_fallback: None,
            upgrade_url: None,
        }
    }

    /// Run `f` with `ARIA_COMPUTE_HOME` pointing at a throwaway directory.
    fn with_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ARIA_COMPUTE_HOME", dir.path());
        let out = f(dir.path());
        std::env::remove_var("ARIA_COMPUTE_HOME");
        out
    }

    #[test]
    fn version_command_prints_and_exits_ok() {
        with_home(|_| {
            let mut io = FakeIo::new(false, false);
            block_on(run_with(cli(Command::Version), &mut io)).unwrap();
        });
    }

    #[test]
    fn setup_then_validate_then_status_then_clear() {
        with_home(|home| {
            let mut io = FakeIo::new(false, false);
            // Non-interactive (flags only): no prompts at all.
            block_on(run_with(
                cli(Command::Setup(Box::new(setup_args()))),
                &mut io,
            ))
            .unwrap();
            assert!(home.join("router.yml").exists());
            assert!(home.join("router-users.json").exists());
            assert!(home.join("router-cli.yml").exists());

            // A second setup sees the existing users and keeps them.
            block_on(run_with(
                cli(Command::Setup(Box::new(setup_args()))),
                &mut io,
            ))
            .unwrap();

            let cfg = home.join("router.yml").display().to_string();
            block_on(run_with(
                cli(Command::Validate {
                    config: Some(cfg.clone()),
                }),
                &mut io,
            ))
            .unwrap();

            // Status reads the generated YAML, keys + users files.
            let mut status = setup_args();
            status.status = true;
            block_on(run_with(cli(Command::Setup(Box::new(status))), &mut io)).unwrap();

            // Invalid YAML fails loudly.
            let bad = home.join("bad.yml");
            std::fs::write(&bad, "version: v0.3\nnope: 1\n").unwrap();
            let err = block_on(run_with(
                cli(Command::Validate {
                    config: Some(bad.display().to_string()),
                }),
                &mut io,
            ))
            .unwrap_err();
            assert!(err.to_string().contains("nope") || err.to_string().contains("unknown"));

            // Clearing with confirmation removes config + key/user files.
            let mut clear = setup_args();
            clear.clear = true;
            let mut yes = FakeIo::new(false, false).answers(&["y"]);
            block_on(run_with(cli(Command::Setup(Box::new(clear))), &mut yes)).unwrap();
            assert!(!home.join("router.yml").exists());
            assert!(!home.join("router-users.json").exists());
            assert!(!home.join("router-cli.yml").exists());
        });
    }

    #[test]
    fn setup_status_handles_missing_config() {
        with_home(|_| {
            let mut io = FakeIo::new(false, false);
            let mut status = setup_args();
            status.status = true;
            block_on(run_with(cli(Command::Setup(Box::new(status))), &mut io)).unwrap();
        });
    }

    #[test]
    fn setup_clear_skips_missing_credential_files() {
        with_home(|home| {
            // Only the key file exists; the users file must be skipped, not fail.
            std::fs::write(home.join("router-keys.json"), r#"{"keys":[]}"#).unwrap();
            let mut clear = setup_args();
            clear.clear = true;
            let mut yes = FakeIo::new(false, false).answers(&["yes"]);
            block_on(run_with(cli(Command::Setup(Box::new(clear))), &mut yes)).unwrap();
            assert!(!home.join("router-keys.json").exists());
            assert!(!home.join("router-users.json").exists());
        });
    }

    #[test]
    fn upgrade_without_cli_config_reports_the_missing_upgrade_url() {
        with_home(|_| {
            let mut io = FakeIo::new(false, false);
            let err = block_on(run_with(
                cli(Command::Upgrade {
                    version: Some("9.9.9".into()),
                }),
                &mut io,
            ))
            .unwrap_err();
            assert!(err.to_string().contains("upgrade_url not set"), "{err}");
        });
    }

    #[test]
    fn setup_interactive_prompts_cover_all_branches() {
        with_home(|home| {
            // Template, base_url, api_key_env, api key, three model ids, upgrade_url.
            let mut io = FakeIo::new(true, false)
                .answers(&[
                    "agent",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "http://127.0.0.1:9000",
                    "ariacompute/ariamodel-mid",
                    "ariacompute/ariamodel-mid",
                    "ops",
                    "https://gitee.com/acme/router",
                ])
                .secrets(&["", "password1", "password1"]);
            let mut args = SetupArgs {
                template: None,
                admin_user: None,
                admin_password: None,
                ..setup_args()
            };
            args.upgrade_url = None;
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let yaml = std::fs::read_to_string(home.join("router.yml")).unwrap();
            assert!(yaml.contains("http://127.0.0.1:9000"), "{yaml}");
            let cli_cfg = std::fs::read_to_string(home.join("router-cli.yml")).unwrap();
            assert!(cli_cfg.contains("gitee.com/acme/router"), "{cli_cfg}");

            // Re-running asks to overwrite; "n" keeps the existing YAML but still
            // rewrites the CLI config.
            let mut keep = FakeIo::new(true, false)
                .answers(&["semantic", "", "", "", "", "", "ops", "", "n"])
                .secrets(&["", "password1", "password1"]);
            let mut args = SetupArgs {
                template: None,
                admin_user: None,
                admin_password: None,
                ..setup_args()
            };
            args.upgrade_url = None;
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut keep)).unwrap();
        });
    }

    #[test]
    fn setup_rejects_invalid_inputs() {
        with_home(|_| {
            let mut io = FakeIo::new(false, false);

            let mut bad_template = setup_args();
            bad_template.template = Some("bogus".into());
            let err = block_on(run_with(
                cli(Command::Setup(Box::new(bad_template))),
                &mut io,
            ))
            .unwrap_err();
            assert!(err.to_string().contains("invalid template"), "{err}");

            let mut short = setup_args();
            short.admin_password = Some("short".into());
            let err =
                block_on(run_with(cli(Command::Setup(Box::new(short))), &mut io)).unwrap_err();
            assert!(err.to_string().contains("at least 8"), "{err}");

            // Interactive password confirmation mismatch.
            let mut mismatch = FakeIo::new(false, false).secrets(&["password1", "password2"]);
            let mut args = SetupArgs {
                admin_password: None,
                ..setup_args()
            };
            args.admin_user = Some("admin".into());
            let err =
                block_on(run_with(cli(Command::Setup(Box::new(args))), &mut mismatch)).unwrap_err();
            assert!(err.to_string().contains("do not match"), "{err}");
        });
    }

    #[test]
    fn setup_writes_upgrade_url_overrides() {
        with_home(|home| {
            let mut io = FakeIo::new(false, false);
            let mut args = setup_args();
            args.upgrade_url = Some("  https://github.com/acme/router  ".into());
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let cfg = std::fs::read_to_string(home.join("router-cli.yml")).unwrap();
            assert!(cfg.contains("https://github.com/acme/router"), "{cfg}");

            // An empty flag keeps the previously saved value.
            let mut args = setup_args();
            args.upgrade_url = Some("   ".into());
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let cfg = std::fs::read_to_string(home.join("router-cli.yml")).unwrap();
            assert!(cfg.contains("https://github.com/acme/router"), "{cfg}");
        });
    }

    #[test]
    fn serve_binds_both_planes_and_stops_on_shutdown() {
        with_home(|home| {
            let mut io = FakeIo::new(false, false);
            block_on(run_with(
                cli(Command::Setup(Box::new(setup_args()))),
                &mut io,
            ))
            .unwrap();
            let cfg = home.join("router.yml").display().to_string();
            for no_dashboard in [false, true] {
                let mut io = FakeIo::new(false, true);
                block_on(run_with(
                    cli(Command::Serve {
                        config: Some(cfg.clone()),
                        bind: Some("127.0.0.1:0".into()),
                        mgmt_bind: "127.0.0.1:0".into(),
                        no_dashboard,
                    }),
                    &mut io,
                ))
                .unwrap();
            }
        });
    }

    #[test]
    fn serve_requires_a_config() {
        with_home(|_| {
            let mut io = FakeIo::new(false, true);
            let err = block_on(run_with(
                cli(Command::Serve {
                    config: None,
                    bind: None,
                    mgmt_bind: "127.0.0.1:0".into(),
                    no_dashboard: true,
                }),
                &mut io,
            ))
            .unwrap_err();
            assert!(err.to_string().contains("missing --config"), "{err}");
        });
    }

    #[test]
    fn resolve_config_prefers_explicit_path() {
        with_home(|home| {
            assert_eq!(resolve_config(Some("x.yml".into())).unwrap(), "x.yml");
            // No flag and no default file → actionable error.
            assert!(resolve_config(None).is_err());
            std::fs::write(home.join("router.yml"), "version: v0.3\n").unwrap();
            let resolved = resolve_config(None).unwrap();
            assert!(resolved.ends_with("router.yml"), "{resolved}");
        });
    }

    #[test]
    fn prompt_opt_prefers_flag_then_prompt() {
        let mut io = FakeIo::new(true, false).answers(&["  from-prompt  "]);
        assert_eq!(
            prompt_opt(&mut io, "label", Some(" from-flag ".into()), true).as_deref(),
            Some("from-flag")
        );
        assert_eq!(prompt_opt(&mut io, "label", Some("   ".into()), true), None);
        assert_eq!(prompt_opt(&mut io, "label", None, false), None);
        assert_eq!(
            prompt_opt(&mut io, "label", None, true).as_deref(),
            Some("from-prompt")
        );
        // Empty answer → None.
        assert_eq!(prompt_opt(&mut io, "label", None, true), None);
    }

    #[test]
    fn read_secret_handles_editing_and_terminators() {
        let mut out = String::new();
        let mut echo = Vec::new();
        read_secret(&mut &b"abc\x7fd\nrest"[..], &mut echo, &mut out).unwrap();
        assert_eq!(out, "abd");
        assert_eq!(echo, b"***\x08 \x08*");

        // Backspace on an empty buffer is a no-op; control bytes are ignored;
        // EOF terminates without a newline.
        let mut out = String::new();
        let mut echo = Vec::new();
        read_secret(&mut &b"\x08\x01x"[..], &mut echo, &mut out).unwrap();
        assert_eq!(out, "x");
        assert_eq!(echo, b"*");

        // A read error surfaces instead of being swallowed.
        struct Broken;
        impl io::Read for Broken {
            fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("boom"))
            }
        }
        let err = read_secret(&mut Broken, &mut Vec::new(), &mut String::new()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[cfg(unix)]
    #[test]
    fn mask_termios_reports_errors_and_handles_pty() {
        use std::os::fd::AsRawFd;

        // Not a terminal → tcgetattr fails.
        assert!(mask_termios(-1).is_err());
        let null = std::fs::File::open("/dev/null").unwrap();
        assert!(mask_termios(null.as_raw_fd()).is_err());

        // A real pty accepts the attribute switch, and dropping restores it.
        let (master, slave) = open_pty();
        {
            let _restore = mask_termios(slave.as_raw_fd()).expect("mask on pty");
        }
        drop(slave);
        drop(master);

        // tcsetattr on a bad fd fails with an OS error.
        let attrs = unsafe { std::mem::zeroed::<libc::termios>() };
        assert!(set_termios(-1, &attrs).is_err());
    }

    /// Open a pseudo terminal pair (assert-based: a failure here is a broken
    /// environment, not a test assertion).
    #[cfg(unix)]
    fn open_pty() -> (std::fs::File, std::fs::File) {
        use std::os::fd::FromRawFd;

        unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(master >= 0, "posix_openpt failed");
            let name = libc::ptsname(master);
            assert!(!name.is_null(), "ptsname failed");
            assert_eq!(libc::grantpt(master), 0, "grantpt failed");
            assert_eq!(libc::unlockpt(master), 0, "unlockpt failed");
            let slave = libc::open(name, libc::O_RDWR | libc::O_NOCTTY);
            assert!(slave >= 0, "open pts failed");
            (
                std::fs::File::from_raw_fd(master),
                std::fs::File::from_raw_fd(slave),
            )
        }
    }

    #[test]
    fn setup_defaults_template_and_skips_empty_flag_values() {
        with_home(|home| {
            // Empty template answer → "semantic"; `--api-key ""` stores nothing.
            let mut io = FakeIo::new(true, false)
                .answers(&["", "", "", "", "", "", "ops", "n", ""])
                .secrets(&["", "password1", "password1"]);
            let args = SetupArgs {
                template: None,
                admin_user: None,
                admin_password: None,
                upgrade_url: None,
                ..setup_args()
            };
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let yaml = std::fs::read_to_string(home.join("router.yml")).unwrap();
            assert!(yaml.contains("semantic"), "{yaml}");

            let mut args = setup_args();
            args.api_key = Some("   ".into());
            let mut io = FakeIo::new(false, false);
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let yaml = std::fs::read_to_string(home.join("router.yml")).unwrap();
            assert!(!yaml.contains("\"   \""), "{yaml}");
        });
    }

    #[test]
    fn setup_tolerates_missing_secret_answers() {
        with_home(|_| {
            // No scripted secrets: both prompts return "" → equal, then rejected
            // for being too short (covers the empty-answer arm of FakeIo).
            let mut io =
                FakeIo::new(true, false).answers(&["semantic", "", "", "", "", "", "ops", "n", ""]);
            let args = SetupArgs {
                template: None,
                admin_user: None,
                admin_password: None,
                upgrade_url: None,
                ..setup_args()
            };
            let err = block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap_err();
            assert!(err.to_string().contains("at least 8"), "{err}");
        });
    }

    #[test]
    fn fake_io_shutdown_can_stay_pending() {
        use std::task::Poll;

        let io = FakeIo::new(false, false);
        let mut fut = io.serve_shutdown();
        let waker = futures_util::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    }

    #[test]
    fn stdio_serve_shutdown_never_resolves() {
        use std::task::Poll;

        let io = StdIo;
        let mut fut = io.serve_shutdown();
        let waker = futures_util::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        assert!(matches!(fut.as_mut().poll(&mut cx), Poll::Pending));
    }

    #[test]
    fn setup_afm_d_prompts_cover_template_specific_branches() {
        with_home(|home| {
            // Interactive afm-d setup: endpoint + fallback prompts, blank admin
            // user (defaults to "admin"), and an api key typed at the prompt.
            let mut io = FakeIo::new(true, false)
                .answers(&[
                    "afm-d",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "http://127.0.0.1:8010",
                    "ariacompute/ariamodel-mid",
                    "",
                    "",
                ])
                .secrets(&["sk-from-prompt", "password1", "password1"]);
            let args = SetupArgs {
                template: None,
                admin_user: None,
                admin_password: None,
                upgrade_url: None,
                ..setup_args()
            };
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let yaml = std::fs::read_to_string(home.join("router.yml")).unwrap();
            assert!(yaml.contains("http://127.0.0.1:8010"), "{yaml}");
            assert!(yaml.contains("sk-from-prompt"), "{yaml}");
            assert!(home.join("router-users.json").is_file());
        });
    }

    #[test]
    fn setup_flag_paths_for_api_key_and_upgrade_url() {
        with_home(|home| {
            // Explicit --api-key value is trimmed and stored in the YAML.
            let mut args = setup_args();
            args.api_key = Some("  sk-flagged  ".into());
            // Blank --upgrade-url with no saved CLI config → site default.
            args.upgrade_url = Some("   ".into());
            let mut io = FakeIo::new(false, false);
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let yaml = std::fs::read_to_string(home.join("router.yml")).unwrap();
            assert!(yaml.contains("sk-flagged"), "{yaml}");
            let cli_cfg = std::fs::read_to_string(home.join("router-cli.yml")).unwrap();
            assert!(
                cli_cfg.contains("github.com") || cli_cfg.contains("gitee.com"),
                "{cli_cfg}"
            );

            // Interactive re-run: the api key prompt returns a value and the
            // upgrade_url prompt accepts a blank answer (keeps the saved one).
            let mut io = FakeIo::new(true, false)
                .answers(&["semantic", "", "", "", "", "", "ops", "n", ""])
                .secrets(&["sk-prompted", "password1", "password1"]);
            let args = SetupArgs {
                template: None,
                admin_user: None,
                admin_password: None,
                upgrade_url: None,
                ..setup_args()
            };
            block_on(run_with(cli(Command::Setup(Box::new(args))), &mut io)).unwrap();
            let cli_cfg = std::fs::read_to_string(home.join("router-cli.yml")).unwrap();
            assert!(
                cli_cfg.contains("github.com") || cli_cfg.contains("gitee.com"),
                "{cli_cfg}"
            );
        });
    }

    #[test]
    fn setup_reports_unusable_user_store() {
        with_home(|home| {
            std::fs::write(home.join("router-users.json"), "{not json").unwrap();
            let mut io = FakeIo::new(false, false);
            let err = block_on(run_with(
                cli(Command::Setup(Box::new(setup_args()))),
                &mut io,
            ))
            .unwrap_err();
            assert!(!err.to_string().is_empty());
        });
    }

    #[test]
    fn setup_status_reports_key_store_variants() {
        with_home(|home| {
            let mut io = FakeIo::new(false, false);
            block_on(run_with(
                cli(Command::Setup(Box::new(setup_args()))),
                &mut io,
            ))
            .unwrap();

            // Point the generated YAML at the temp files so `--status` can read
            // them (the default `~/…` paths resolve to the real user home).
            let keys_path = home.join("router-keys.json");
            let users_path = home.join("router-users.json");
            let cfg_path = home.join("router.yml");
            let yaml = std::fs::read_to_string(&cfg_path)
                .unwrap()
                .replace(
                    "~/.ariacompute/router-keys.json",
                    keys_path.to_str().unwrap(),
                )
                .replace(
                    "~/.ariacompute/router-users.json",
                    users_path.to_str().unwrap(),
                );
            std::fs::write(&cfg_path, yaml).unwrap();

            let mut status = setup_args();
            status.status = true;
            // A local-only key store: no serve account, no oauth api key.
            let mut store = KeyStore::empty(keys_path.clone());
            store.create("local-only").unwrap();
            block_on(run_with(
                cli(Command::Setup(Box::new(status.clone()))),
                &mut io,
            ))
            .unwrap();

            // An api key without a linked user shows the "key only" state.
            let mut store = KeyStore::empty(keys_path.clone());
            store
                .oauth_set_api_key("sk-bf-0123456789abcdef", None)
                .unwrap();
            block_on(run_with(
                cli(Command::Setup(Box::new(status.clone()))),
                &mut io,
            ))
            .unwrap();

            // A linked serve account with an api key.
            let mut store = KeyStore::empty(keys_path.clone());
            store
                .oauth_set_api_key("sk-bf-abcdef0123456789", None)
                .unwrap();
            let mut raw: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&keys_path).unwrap()).unwrap();
            // The store always serialises a `keys` array, so this cannot be absent.
            let keys = raw["keys"].as_array_mut().expect("key file has keys array");
            for k in keys.iter_mut() {
                k["site"] = serde_json::json!("com");
                k["user"] = serde_json::json!({"id": "usr_1", "email": "ops@example.com"});
            }
            std::fs::write(&keys_path, raw.to_string()).unwrap();
            block_on(run_with(cli(Command::Setup(Box::new(status))), &mut io)).unwrap();

            // Clearing with confirmation removes both files.
            let mut clear = setup_args();
            clear.clear = true;
            let mut yes = FakeIo::new(false, false).answers(&["yes"]);
            block_on(run_with(cli(Command::Setup(Box::new(clear))), &mut yes)).unwrap();
            assert!(!keys_path.exists());
            assert!(!users_path.exists());
        });
    }
}
