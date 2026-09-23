use std::net::SocketAddr;
use std::path::PathBuf;

use openwebide_bridge::{ServerConfig, run_server};
use tokio::net::TcpListener;

struct Args {
    host: String,
    port: u16,
    workspace: PathBuf,
    allowed_origins: Vec<String>,
    allowed_hosts: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut host = std::env::var("OPENWEBIDE_BRIDGE_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let mut port: u16 = std::env::var("OPENWEBIDE_BRIDGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);
    let mut workspace = std::env::var("OPENWEBIDE_BRIDGE_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut allowed_origins = Vec::new();
    if let Ok(env_origins) = std::env::var("OPENWEBIDE_BRIDGE_ALLOWED_ORIGINS") {
        for orig in env_origins.split(',') {
            let trimmed = orig.trim();
            if !trimmed.is_empty() {
                allowed_origins.push(trimmed.to_string());
            }
        }
    }

    let mut allowed_hosts = Vec::new();
    if let Ok(env_hosts) = std::env::var("OPENWEBIDE_BRIDGE_ALLOWED_HOSTS") {
        for h in env_hosts.split(',') {
            let trimmed = h.trim();
            if !trimmed.is_empty() {
                allowed_hosts.push(trimmed.to_string());
            }
        }
    }

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "OpenWebIDE Bridge Daemon\n\n\
                     Usage: openwebide-bridge [OPTIONS]\n\n\
                     Options:\n\
                       --host <HOST>             Host to bind on (default: 127.0.0.1, env: OPENWEBIDE_BRIDGE_HOST)\n\
                       -p, --port <PORT>         Port to bind on (default: 3001, env: OPENWEBIDE_BRIDGE_PORT)\n\
                       -w, --workspace <DIR>     Workspace root directory (default: current dir, env: OPENWEBIDE_BRIDGE_WORKSPACE)\n\
                       --allowed-origin <ORIGIN> Allowed CORS Origin (repeatable, env: OPENWEBIDE_BRIDGE_ALLOWED_ORIGINS)\n\
                       --allowed-host <HOSTNAME> Allowed Host header (repeatable, env: OPENWEBIDE_BRIDGE_ALLOWED_HOSTS)\n\
                       --help                    Show this help message\n"
                );
                std::process::exit(0);
            }
            "--host" => {
                host = args
                    .next()
                    .ok_or_else(|| "missing value for --host".to_string())?;
            }
            "-p" | "--port" => {
                let p_str = args
                    .next()
                    .ok_or_else(|| "missing value for --port".to_string())?;
                port = p_str
                    .parse()
                    .map_err(|e| format!("invalid port '{p_str}': {e}"))?;
            }
            "-w" | "--workspace" => {
                let w_str = args
                    .next()
                    .ok_or_else(|| "missing value for --workspace".to_string())?;
                workspace = PathBuf::from(w_str);
            }
            "--allowed-origin" => {
                let orig = args
                    .next()
                    .ok_or_else(|| "missing value for --allowed-origin".to_string())?;
                allowed_origins.push(orig.trim().to_string());
            }
            "--allowed-host" => {
                let h = args
                    .next()
                    .ok_or_else(|| "missing value for --allowed-host".to_string())?;
                allowed_hosts.push(h.trim().to_string());
            }
            other => {
                return Err(format!("unknown argument: {other}"));
            }
        }
    }

    Ok(Args {
        host,
        port,
        workspace,
        allowed_origins,
        allowed_hosts,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };

    let workspace_root = match openwebide_bridge::paths::canonical_root(&args.workspace) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    let mut config = ServerConfig::new(workspace_root);
    for orig in args.allowed_origins {
        if !config
            .allowed_origins
            .iter()
            .any(|o| o.eq_ignore_ascii_case(&orig))
        {
            config.allowed_origins.push(orig);
        }
    }
    for h in args.allowed_hosts {
        if !config
            .allowed_hosts
            .iter()
            .any(|host| host.eq_ignore_ascii_case(&h))
        {
            config.allowed_hosts.push(h);
        }
    }

    let addr_str = format!("{}:{}", args.host, args.port);
    let addr: SocketAddr = addr_str
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid socket address {addr_str}: {e}"))?;

    let listener = TcpListener::bind(addr).await?;
    println!("OpenWebIDE Bridge daemon listening on ws://{addr} (HTTP POST /exec enabled)");
    println!("Workspace root: {}", config.workspace_root.display());
    println!("Allowed origins: {}", config.allowed_origins.join(", "));
    println!("Allowed hosts: {}", config.allowed_hosts.join(", "));

    run_server(listener, config).await;

    Ok(())
}
