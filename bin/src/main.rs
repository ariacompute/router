use aria_router_config::{
    clear_default_config, default_config_path, default_keys_path, default_users_path,
    resolve_keys_path, resolve_users_path, RouterDocument,
};
use aria_router_http::{
    data_router, ensure_extensions_startable, mgmt_router, mgmt_router_serve_dashboard,
    AppState, KeyStore, LocalUserStore,
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
    /// Write router.yml and create admin user
    Setup {
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

fn cmd_setup(
    status: bool,
    clear: bool,
    template: Option<String>,
    admin_user: Option<String>,
    admin_password: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if status {
        return setup_status();
    }
    if clear {
        return setup_clear();
    }

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
        let u = prompt("admin username [admin]: ").unwrap_or_default();
        if u.is_empty() {
            "admin".into()
        } else {
            u
        }
    });
    let admin_pass = admin_password.unwrap_or_else(|| {
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
        let ans = prompt(&format!("{} exists; overwrite? [y/N]: ", path.display()))?;
        matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes")
    } else {
        true
    };
    if path.exists() && !overwrite {
        println!("kept {}", path.display());
        return Ok(());
    }

    // Defaults: allow_register=true, require_api_key=true (edit YAML or Dashboard later).
    let written = aria_router_config::write_default_config_with(&kind, true, true, true)?;
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
                pubu.api_key_prefix.as_deref().unwrap_or("bfvk-…")
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
        } => cmd_setup(status, clear, template, admin_user, admin_password)?,
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
        Command::Version => {
            println!("aria-router {ROUTER_VERSION}");
        }
    }
    Ok(())
}
