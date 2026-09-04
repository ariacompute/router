use aria_router_config::{
    clear_default_config, default_config_path, default_keys_path, default_users_path,
    resolve_keys_path, resolve_users_path, RouterDocument,
};
use aria_router_http::{
    data_router, ensure_extensions_startable, mgmt_router, mgmt_router_with_dashboard,
    resolve_dashboard_dir, validate_bfvk, AppState, KeyStore, LocalUserStore,
};
use clap::{ArgAction, Parser, Subcommand};
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
    /// Write router.yml (Local + optional OAuth)
    Setup {
        /// Show config status
        #[arg(long)]
        status: bool,
        /// Remove router.yml (optional local/OAuth files)
        #[arg(long)]
        clear: bool,
        /// Template: semantic | agent
        #[arg(long)]
        template: Option<String>,
        /// Local admin username
        #[arg(long)]
        admin_user: Option<String>,
        /// Local admin password
        #[arg(long)]
        admin_password: Option<String>,
        /// Allow Dashboard self-registration (true|false)
        #[arg(long)]
        allow_register: Option<String>,
        /// Require local API key on data plane
        #[arg(long, action = ArgAction::SetTrue)]
        require_api_key: bool,
        /// OAuth site URL
        #[arg(long)]
        serve_site: Option<String>,
        /// OAuth Serve API key (bfvk-…)
        #[arg(long)]
        serve_api_key: Option<String>,
    },
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
    /// Print version
    Version,
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

fn cmd_setup(
    status: bool,
    clear: bool,
    template: Option<String>,
    admin_user: Option<String>,
    admin_password: Option<String>,
    allow_register_flag: Option<String>,
    require_api_key_flag: bool,
    serve_site: Option<String>,
    serve_api_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if status {
        return setup_status();
    }
    if clear {
        return setup_clear();
    }

    eprintln!("── [1/2] Local (router Dashboard) ─────────────────────────");
    eprintln!("  Local users: Dashboard username/password.");
    eprintln!("  Local API keys: sk-aria_… (Dashboard → Keys only; CLI does not mint).");
    eprintln!();

    let raw = template.unwrap_or_else(|| {
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

    let admin_user = admin_user.unwrap_or_else(|| {
        let u = prompt("local admin username [admin]: ").unwrap_or_default();
        if u.is_empty() {
            "admin".into()
        } else {
            u
        }
    });
    let admin_pass = admin_password.unwrap_or_else(|| {
        let p1 = prompt("local admin password: ").unwrap_or_default();
        let p2 = prompt("confirm password: ").unwrap_or_default();
        if p1 != p2 {
            eprintln!("passwords do not match");
            std::process::exit(1);
        }
        p1
    });
    if admin_pass.len() < 8 {
        return Err("password must be at least 8 characters".into());
    }

    let allow_register = if let Some(v) = allow_register_flag {
        matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "y" | "yes")
    } else {
        let ans = prompt("allow Dashboard self-registration for local users? [Y/n]: ")?;
        !matches!(ans.to_ascii_lowercase().as_str(), "n" | "no")
    };

    let require_api_key = if require_api_key_flag {
        true
    } else {
        let ans = prompt("require local API key (sk-aria_) on data plane? [y/N]: ")?;
        matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes")
    };
    eprintln!("  (note) After serve: Dashboard → register/login → Keys → mint sk-aria_.");
    eprintln!();

    eprintln!("── [2/2] OAuth (Aria Compute) ───────────────────────");
    eprintln!("  Optional. Cloud account on ariacompute.com or .cn.");
    eprintln!("  Serve API keys: bfvk-… (NOT sk-aria_).");
    eprintln!("  OAuth link: Dashboard → Account (CLI only pastes the key).");
    eprintln!();

    let mut serve_site = serve_site;
    let mut serve_key = serve_api_key;
    if serve_site.is_none() && serve_key.is_none() {
        let ans = prompt("configure OAuth API key now? [y/N]: ")?;
        if matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes") {
            let site =
                prompt("Serve site [1] https://ariacompute.com  [2] https://ariacompute.cn: ")?;
            serve_site = Some(site);
            let key = prompt("Serve API key (bfvk-…): ")?;
            serve_key = Some(key);
        }
    }

    let path = default_config_path()?;
    let overwrite = if path.exists() {
        let ans = prompt(&format!("{} exists; overwrite? [y/N]: ", path.display()))?;
        matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes")
    } else {
        true
    };
    if path.exists() && !overwrite {
        println!("kept {}", path.display());
        return Ok(());
    }

    let written =
        aria_router_config::write_default_config_with(&kind, true, require_api_key, allow_register)?;
    println!("wrote {}", written.display());

    let users_path = default_users_path()?;
    match LocalUserStore::create_admin(&users_path, &admin_user, &admin_pass) {
        Ok(_) => println!("Local admin user '{admin_user}' created"),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exist") {
                eprintln!("Local users already present; kept existing (use --clear to reset)");
            } else {
                return Err(msg.into());
            }
        }
    }

    if let (Some(site), Some(key)) = (serve_site, serve_key) {
        validate_bfvk(&key).map_err(|e| e.to_string())?;
        let keys_path = default_keys_path()?;
        let mut store =
            KeyStore::load(&keys_path).unwrap_or_else(|_| KeyStore::empty(keys_path.clone()));
        store.oauth_set_site(&site).map_err(|e| e.to_string())?;
        store
            .oauth_set_api_key(&key, Some("aria-router"))
            .map_err(|e| e.to_string())?;
        println!("OAuth key saved to {}", keys_path.display());
    }

    Ok(())
}

fn setup_status() -> Result<(), Box<dyn std::error::Error>> {
    let path = default_config_path()?;
    println!("Local (router Dashboard):");
    println!("  config: {}", path.display());
    if path.exists() {
        let doc = RouterDocument::load_path(&path)?;
        println!("  ok");
        println!("  require_api_key: {}", doc.global.require_api_key);
        println!("  allow_register: {}", doc.global.allow_register);
        let kp = doc
            .global
            .keys_path
            .clone()
            .unwrap_or_else(|| "~/.ariacompute/router-keys.json".into());
        println!("  keys_path: {kp}");
        let resolved = resolve_keys_path(&kp)?;
        if resolved.exists() {
            let (a, r) = aria_router_http::load_keys_for_status(&resolved)?;
            println!("  local_api_keys: active={a} revoked={r}");
        } else {
            println!("  local_api_keys: (file missing)");
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
            println!("  users: admin={admin} user={user}");
        } else {
            println!("  users: (file missing)");
        }
    } else {
        println!("  (missing; run aria-router setup)");
    }

    println!("OAuth (Aria Compute):");
    let kp = if path.exists() {
        let doc = RouterDocument::load_path(&path).ok();
        doc.and_then(|d| d.global.keys_path)
            .unwrap_or_else(|| "~/.ariacompute/router-keys.json".into())
    } else {
        "~/.ariacompute/router-keys.json".into()
    };
    let kpath = resolve_keys_path(&kp)?;
    if kpath.exists() {
        let store = KeyStore::load(&kpath).map_err(|e| e.to_string())?;
        let pubu = store.oauth_public();
        println!("  site: {}", pubu.site.as_deref().unwrap_or("(none)"));
        if let Some(u) = &pubu.user {
            println!(
                "  linked_user: {}",
                u.email.as_deref().unwrap_or("(no email)")
            );
        } else if pubu.api_key_configured {
            println!("  linked_user: (not linked — key only)");
        } else {
            println!("  linked_user: (none)");
        }
        if pubu.api_key_configured {
            println!(
                "  oauth_api_key: configured ({})",
                pubu.api_key_prefix.as_deref().unwrap_or("bfvk-…")
            );
        } else {
            println!("  oauth_api_key: missing");
        }
    } else {
        println!("  oauth_api_key: missing");
    }
    Ok(())
}

fn setup_clear() -> Result<(), Box<dyn std::error::Error>> {
    let path = clear_default_config()?;
    println!("cleared {}", path.display());
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
        Command::Setup {
            status,
            clear,
            template,
            admin_user,
            admin_password,
            allow_register,
            require_api_key,
            serve_site,
            serve_api_key,
        } => cmd_setup(
            status,
            clear,
            template,
            admin_user,
            admin_password,
            allow_register,
            require_api_key,
            serve_site,
            serve_api_key,
        )?,
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
            ensure_extensions_startable(&doc)?;
            let bind = bind.unwrap_or_else(|| doc.data_bind());
            let mgmt = mgmt_bind;
            let state = Arc::new(AppState::with_path(doc, PathBuf::from(&config)));
            let data = data_router(state.clone());
            let admin = if no_dashboard {
                println!("data {bind}  mgmt {mgmt}");
                mgmt_router(state)
            } else if let Some(dir) = resolve_dashboard_dir() {
                println!("data {bind}  mgmt {mgmt}");
                println!("dashboard http://{mgmt}/");
                mgmt_router_with_dashboard(state, dir)
            } else {
                println!("data {bind}  mgmt {mgmt}");
                eprintln!("dashboard assets missing (npm --prefix dashboard run build); API only");
                mgmt_router(state)
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
        Command::Version => {
            println!("aria-router {ROUTER_VERSION}");
        }
    }
    Ok(())
}
