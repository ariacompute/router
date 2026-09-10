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
use std::io::{self, BufRead, Write};
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

#[derive(Args)]
struct SetupArgs {
    /// Show config status
    #[arg(long)]
    status: bool,
    /// Remove router.yml (optional keys/users files)
    #[arg(long)]
    clear: bool,
    /// Template: semantic | agent
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
    /// Releases org root for `aria-router upgrade` (written to router-cli.yml)
    #[arg(long)]
    upgrade_url: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn prompt(label: &str) -> io::Result<String> {
    eprint!("{label}");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Prompt for a secret: echo `*` per character (no plaintext). Falls back to
/// plain read_line when stdin is not a TTY.
fn prompt_password(label: &str) -> io::Result<String> {
    eprint!("{label}");
    io::stderr().flush()?;
    #[cfg(unix)]
    {
        if unsafe { libc::isatty(libc::STDIN_FILENO) } != 0 {
            let secret = read_password_masked()?;
            eprintln!();
            return Ok(secret);
        }
    }
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(unix)]
fn read_password_masked() -> io::Result<String> {
    use std::io::Read;
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let mut old = MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, old.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let old = unsafe { old.assume_init() };
    let mut raw = old;
    raw.c_lflag &= !(libc::ECHO | libc::ICANON);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return Err(io::Error::last_os_error());
    }

    struct Restore(libc::termios);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0) };
        }
    }
    let _restore = Restore(old);

    let mut out = String::new();
    let mut stdin = stdin.lock();
    let mut buf = [0u8; 1];
    loop {
        let n = stdin.read(&mut buf)?;
        if n == 0 {
            break;
        }
        match buf[0] {
            b'\n' | b'\r' => break,
            0x7f | 0x08 => {
                if out.pop().is_some() {
                    eprint!("\x08 \x08");
                    io::stderr().flush()?;
                }
            }
            c if c >= 0x20 && c != 0x7f => {
                out.push(c as char);
                eprint!("*");
                io::stderr().flush()?;
            }
            _ => {}
        }
    }
    Ok(out)
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

fn prompt_opt(label: &str, flag: Option<String>, interactive: bool) -> Option<String> {
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
    let ans = prompt(label).unwrap_or_default();
    let t = ans.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn stdin_is_tty() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::isatty(libc::STDIN_FILENO) != 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn cmd_setup(args: SetupArgs) -> Result<(), Box<dyn std::error::Error>> {
    if args.status {
        return setup_status();
    }
    if args.clear {
        return setup_clear();
    }

    let template_flag = args.template.is_some();
    let admin_flagged = args.admin_user.is_some() && args.admin_password.is_some();
    // Fully flagged setup (CI): keep gateway defaults unless model flags passed.
    let prompt_models = stdin_is_tty() && !(template_flag && admin_flagged);

    let raw = args.template.unwrap_or_else(|| {
        prompt("template [semantic|agent] (default: semantic): ").unwrap_or_default()
    });
    let kind = if raw.is_empty() {
        "semantic".to_string()
    } else {
        raw.to_ascii_lowercase()
    };
    if kind != "semantic" && kind != "agent" {
        return Err(format!("invalid template: {kind}").into());
    }

    let models = aria_router_config::SetupModelOpts {
        base_url: prompt_opt(
            "gateway base_url [https://tokenhub.tencentmaas.com]: ",
            args.base_url,
            prompt_models,
        ),
        api_key_env: prompt_opt(
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
                let ans = prompt_password(
                    "gateway API key (saved in router.yml; Enter=env only): ",
                )
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
            "ariamodel-small provider_model_id [qwen3.5-flash]: ",
            args.model_small,
            prompt_models,
        ),
        mid_provider_model_id: prompt_opt(
            "ariamodel-mid provider_model_id [glm-5.3]: ",
            args.model_mid,
            prompt_models,
        ),
        large_provider_model_id: prompt_opt(
            "ariamodel-large provider_model_id [deepseek-v4-pro]: ",
            args.model_large,
            prompt_models,
        ),
        agent_endpoint: if kind == "agent" {
            prompt_opt(
                "agent.endpoint [same as base_url / tokenhub]: ",
                args.agent_endpoint,
                prompt_models,
            )
        } else {
            None
        },
        agent_model: if kind == "agent" {
            prompt_opt(
                "agent.model [ariacompute/ariamodel-mid]: ",
                args.agent_model,
                prompt_models,
            )
        } else {
            None
        },
        agent_fallback: if kind == "agent" {
            prompt_opt(
                "agent.fallback [ariacompute/ariamodel-mid]: ",
                args.agent_fallback,
                prompt_models,
            )
        } else {
            None
        },
    };

    let admin_user = args.admin_user.unwrap_or_else(|| {
        let u = prompt("admin username [admin]: ").unwrap_or_default();
        if u.is_empty() {
            "admin".into()
        } else {
            u
        }
    });
    let admin_pass = args.admin_password.unwrap_or_else(|| {
        let p1 = prompt_password("admin password: ").unwrap_or_default();
        let p2 = prompt_password("confirm password: ").unwrap_or_default();
        if p1 != p2 {
            eprintln!("passwords do not match");
            std::process::exit(1);
        }
        p1
    });
    if admin_pass.len() < 8 {
        return Err("password must be at least 8 characters".into());
    }

    let path = default_config_path()?;
    let overwrite = if path.exists() {
        if prompt_models || stdin_is_tty() {
            let ans = prompt(&format!("{} exists; overwrite? [y/N]: ", path.display()))?;
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
    } else if prompt_models || stdin_is_tty() {
        let hint = if existing_cli.upgrade_url.is_empty() {
            default_upgrade
        } else {
            existing_cli.upgrade_url.as_str()
        };
        let ans = prompt(&format!("upgrade_url (default: {hint}): "))?;
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
    let written = aria_router_config::write_default_config_with_opts(
        &kind, true, true, true, &models,
    )?;
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

fn setup_clear() -> Result<(), Box<dyn std::error::Error>> {
    let path = clear_default_config()?;
    println!("cleared {}", path.display());
    if let Some(p) = clear_cli_config()? {
        println!("cleared {}", p.display());
    }
    let ans = prompt("also delete router-keys.json and router-users.json? [y/N]: ")?;
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

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Setup(args) => cmd_setup(*args)?,
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
            let admin = if no_dashboard {
                println!("data {bind}  mgmt {mgmt}");
                mgmt_router(state)
            } else {
                println!("data {bind}  mgmt {mgmt}");
                println!("dashboard http://{mgmt}/");
                mgmt_router_serve_dashboard(state)
            };
            let data_l = tokio::net::TcpListener::bind(&bind).await?;
            let mgmt_l = tokio::net::TcpListener::bind(&mgmt).await?;
            let a = axum::serve(data_l, data);
            let b = axum::serve(mgmt_l, admin);
            tokio::select! {
                r = a => r?,
                r = b => r?,
            }
        }
        Command::Upgrade { version } => {
            upgrade::run(version.as_deref(), ROUTER_VERSION)
                .await
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
        }
        Command::Version => {
            println!("aria-router {ROUTER_VERSION}");
        }
    }
    Ok(())
}
