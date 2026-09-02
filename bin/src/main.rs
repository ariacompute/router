use ariarouter_config::{clear_default_config, default_config_path, RouterDocument};
use ariarouter_http::{
    data_router, ensure_extensions_startable, mgmt_router, mgmt_router_with_dashboard,
    resolve_dashboard_dir, AppState,
};
use std::path::PathBuf;
use std::io::{self, BufRead, Write};
use std::sync::Arc;

/// Embedded at compile time; release builds set `ARIAROUTER_VERSION` from the git tag.
const ROUTER_VERSION: &str = env!("ARIAROUTER_VERSION");

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
ariarouter {ROUTER_VERSION}

ariarouter setup [--status|--clear]
ariarouter validate [--config <file>]
ariarouter serve [--config <file>] [--bind HOST:PORT] [--mgmt-bind HOST:PORT] [--no-dashboard]
ariarouter -h | --help | help
ariarouter -v | --version | version

Cache:
  ~/.ariacompute/ariarouter.yml   (default --config; written by setup)

setup                Write a starter YAML v0.3 document to ariarouter.yml
  --status           Show default config path and validate if present
  --clear            Remove ariarouter.yml
validate             Load + validate YAML (default: ~/.ariacompute/ariarouter.yml)
serve                Start data + management HTTP servers
  --config           YAML path (default: ~/.ariacompute/ariarouter.yml)
  --bind             Data-plane address (default: first listener, else 0.0.0.0:8899)
  --mgmt-bind        Management address (default: 127.0.0.1:8080)
  --no-dashboard     JSON management API only (no SPA)
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
        return Err("missing --config (run ariarouter setup)".into());
    }
    Ok(path.display().to_string())
}

fn cmd_setup(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.iter().any(|a| a == "--status") {
        let path = default_config_path()?;
        println!("config: {}", path.display());
        if path.exists() {
            let doc = RouterDocument::load_path(&path)?;
            println!("ok");
            println!("require_api_key: {}", doc.global.require_api_key);
            let kp = doc
                .global
                .keys_path
                .clone()
                .unwrap_or_else(|| "~/.ariacompute/ariarouter-keys.json".into());
            println!("keys_path: {kp}");
            let resolved = ariarouter_config::resolve_keys_path(&kp)?;
            if resolved.exists() {
                let store = ariarouter_http::load_keys_for_status(&resolved)?;
                let (a, r) = store;
                println!("api_keys: active={a} revoked={r}");
            } else {
                println!("api_keys: (file missing)");
            }
        } else {
            println!("(missing; run ariarouter setup)");
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--clear") {
        let path = clear_default_config()?;
        println!("cleared {}", path.display());
        let ans = prompt("also delete ariarouter-keys.json? [y/N]: ")?;
        if matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes") {
            let kp = ariarouter_config::default_keys_path()?;
            if kp.exists() {
                std::fs::remove_file(&kp)?;
                println!("cleared {}", kp.display());
            }
        }
        return Ok(());
    }

    let raw = prompt("template [semantic|agent] (default: semantic): ")?;
    let kind = if raw.is_empty() {
        "semantic".to_string()
    } else {
        raw.to_ascii_lowercase()
    };
    if kind != "semantic" && kind != "agent" {
        return Err(format!("invalid template: {kind}").into());
    }
    let req_key = prompt("require API key on data plane? [y/N]: ")?;
    let require_api_key = matches!(req_key.to_ascii_lowercase().as_str(), "y" | "yes");
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
    let written = ariarouter_config::write_default_config_with(&kind, true, require_api_key)?;
    println!("wrote {}", written.display());
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
        println!("ariarouter {ROUTER_VERSION}");
        return Ok(());
    }
    let cmd = args.remove(0);
    match cmd.as_str() {
        "setup" => cmd_setup(&args)?,
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
