use aria_router_config::{
    clear_default_config, default_config_path, default_keys_path, default_serve_account_path,
    default_users_path, resolve_keys_path, resolve_serve_account_path, resolve_users_path,
    RouterDocument,
};
use aria_router_http::{
    data_router, ensure_extensions_startable, mgmt_router, mgmt_router_with_dashboard,
    resolve_dashboard_dir, validate_bfvk, AppState, LocalUserStore, ServeAccountStore,
};
use std::path::PathBuf;
use std::io::{self, BufRead, Write};
use std::sync::Arc;

const ROUTER_VERSION: &str = env!("ARIA_ROUTER_VERSION");

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn print_usage() {
    println!(
        "\
aria-router {ROUTER_VERSION}

Credentials (two sections — do not mix):
  [1/2] Local (router Dashboard)  — username/password; API keys sk-aria_…
  [2/2] OAuth (Aria Compute)      — ariacompute.com/cn; API keys bfvk-…

aria-router setup [--status|--clear]
  Local flags:  --admin-user --admin-password --allow-register --require-api-key
  OAuth flags:  --serve-site com|cn --serve-api-key bfvk-…
aria-router validate [--config <file>]
aria-router serve [--config <file>] [--bind HOST:PORT] [--mgmt-bind HOST:PORT] [--no-dashboard]
aria-router -h | --help | help
aria-router -v | --version | version
"
    );
}

fn prompt(label: &str) -> io::Result<String> {
    eprint!("{label}");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn resolve_config(args: &mut Vec<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(p) = take_flag(args, "--config") {
        return Ok(p);
    }
    let path = default_config_path()?;
    if !path.exists() {
        return Err("missing --config (run aria-router setup)".into());
    }
    Ok(path.display().to_string())
}

fn cmd_setup(args: &mut Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    if args.iter().any(|a| a == "--status") {
        return setup_status();
    }
    if args.iter().any(|a| a == "--clear") {
        return setup_clear();
    }

    eprintln!("── [1/2] Local (router Dashboard) ─────────────────────────");
    eprintln!("  Local users: Dashboard username/password.");
    eprintln!("  Local API keys: sk-aria_… (Dashboard → Keys only; CLI does not mint).");
    eprintln!();

    let raw = take_flag(args, "--template").unwrap_or_else(|| {
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

    let admin_user = take_flag(args, "--admin-user").unwrap_or_else(|| {
        let u = prompt("local admin username [admin]: ").unwrap_or_default();
        if u.is_empty() {
            "admin".into()
        } else {
            u
        }
    });
    let admin_pass = take_flag(args, "--admin-password").unwrap_or_else(|| {
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

    let allow_register = if let Some(v) = take_flag(args, "--allow-register") {
        matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "y" | "yes")
    } else {
        let ans = prompt("allow Dashboard self-registration for local users? [Y/n]: ")?;
        !matches!(ans.to_ascii_lowercase().as_str(), "n" | "no")
    };

    let require_api_key = if args.iter().any(|a| a == "--require-api-key") {
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

    let mut serve_site = take_flag(args, "--serve-site");
    let mut serve_key = take_flag(args, "--serve-api-key");
    if serve_site.is_none() && serve_key.is_none() {
        let ans = prompt("configure OAuth API key now? [y/N]: ")?;
        if matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes") {
            let site = prompt("Serve site [1] https://ariacompute.com  [2] https://ariacompute.cn: ")?;
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

    let written = aria_router_config::write_default_config_with(
        &kind,
        true,
        require_api_key,
        allow_register,
    )?;
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
        let serve_path = default_serve_account_path()?;
        let mut store = if serve_path.exists() {
            ServeAccountStore::load(&serve_path).map_err(|e| e.to_string())?
        } else {
            ServeAccountStore::empty(serve_path.clone())
        };
        store.set_site(&site).map_err(|e| e.to_string())?;
        store
            .set_api_key(&key, Some("aria-router"))
            .map_err(|e| e.to_string())?;
        println!("OAuth serve account key saved to {}", serve_path.display());
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
    let sp = default_serve_account_path()?;
    let spath = if path.exists() {
        let doc = RouterDocument::load_path(&path).ok();
        doc.and_then(|d| d.global.serve_account_path)
            .map(|p| resolve_serve_account_path(&p).unwrap_or(sp.clone()))
            .unwrap_or(sp)
    } else {
        sp
    };
    if spath.exists() {
        let store = ServeAccountStore::load(&spath).map_err(|e| e.to_string())?;
        let pubu = store.public();
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
                "  serve_api_key: configured ({})",
                pubu.api_key_prefix.as_deref().unwrap_or("bfvk-…")
            );
        } else {
            println!("  serve_api_key: missing");
        }
    } else {
        println!("  serve_api_key: missing");
    }
    Ok(())
}

fn setup_clear() -> Result<(), Box<dyn std::error::Error>> {
    let path = clear_default_config()?;
    println!("cleared {}", path.display());
    let ans = prompt("also delete Local router-keys.json and router-users.json? [y/N]: ")?;
    if matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes") {
        for p in [default_keys_path()?, default_users_path()?] {
            if p.exists() {
                std::fs::remove_file(&p)?;
                println!("cleared {}", p.display());
            }
        }
    }
    let ans = prompt("also delete OAuth router-serve.json? [y/N]: ")?;
    if matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes") {
        let p = default_serve_account_path()?;
        if p.exists() {
            std::fs::remove_file(&p)?;
            println!("cleared {}", p.display());
        }
    }
    Ok(())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty()
        || args
            .iter()
            .any(|a| a == "-h" || a == "--help" || a == "help")
    {
        print_usage();
        return Ok(());
    }
    if args
        .iter()
        .any(|a| a == "-v" || a == "--version" || a == "version")
    {
        println!("aria-router {ROUTER_VERSION}");
        return Ok(());
    }
    let cmd = args.remove(0);
    match cmd.as_str() {
        "setup" => cmd_setup(&mut args)?,
        "validate" => {
            let config = resolve_config(&mut args)?;
            RouterDocument::load_path(&config)?;
            println!("ok");
        }
        "serve" => {
            let config = resolve_config(&mut args)?;
            let doc = RouterDocument::load_path(&config)?;
            ensure_extensions_startable(&doc)?;
            let bind = take_flag(&mut args, "--bind").unwrap_or_else(|| doc.data_bind());
            let mgmt = take_flag(&mut args, "--mgmt-bind")
                .unwrap_or_else(|| "127.0.0.1:8080".into());
            let no_dashboard = take_switch(&mut args, "--no-dashboard");
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
        other => return Err(format!("unknown command {other}").into()),
    }
    Ok(())
}

fn take_flag(args: &mut Vec<String>, name: &str) -> Option<String> {
    if let Some(i) = args.iter().position(|a| a == name) {
        args.remove(i);
        if i < args.len() {
            return Some(args.remove(i));
        }
    }
    None
}

fn take_switch(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(i) = args.iter().position(|a| a == name) {
        args.remove(i);
        true
    } else {
        false
    }
}
