use aria_router_config::RouterDocument;
use aria_router_http::{data_router, ensure_extensions_startable, mgmt_router, AppState};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!(
            "aria-router validate --config <file>\naria-router serve --config <file> [--bind HOST:PORT] [--mgmt-bind HOST:PORT]"
        );
        return Ok(());
    }
    let cmd = args.remove(0);
    let config = take_flag(&mut args, "--config").ok_or("missing --config")?;
    match cmd.as_str() {
        "validate" => {
            RouterDocument::load_path(&config)?;
            println!("ok");
        }
        "serve" => {
            let doc = RouterDocument::load_path(&config)?;
            ensure_extensions_startable(&doc)?;
            let bind = take_flag(&mut args, "--bind").unwrap_or_else(|| doc.data_bind());
            let mgmt = take_flag(&mut args, "--mgmt-bind")
                .unwrap_or_else(|| "127.0.0.1:8080".into());
            let state = Arc::new(AppState::new(doc));
            let data = data_router(state.clone());
            let admin = mgmt_router(state);
            println!("data {bind}  mgmt {mgmt}");
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
