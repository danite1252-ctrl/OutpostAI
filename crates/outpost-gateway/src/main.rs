use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use outpost_gateway::config::{default_data_dir, DEFAULT_OLLAMA_URL};
use outpost_gateway::{AppState, Config};
use tracing_subscriber::EnvFilter;

/// Outpost local API gateway.
#[derive(Parser)]
#[command(name = "outpost-gateway", version, about)]
struct Args {
    /// Folder for the database and account files.
    #[arg(long, env = "OUTPOST_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Address to listen on, e.g. 0.0.0.0:7420 for other devices. Without it, Outpost
    /// uses the "Allow other devices on my network" setting (127.0.0.1:7420 when off).
    #[arg(long, env = "OUTPOST_BIND")]
    bind: Option<SocketAddr>,

    /// Ollama's address.
    #[arg(long, env = "OUTPOST_OLLAMA_URL", default_value = DEFAULT_OLLAMA_URL)]
    ollama_url: String,

    /// Print the OpenAPI spec as JSON and exit.
    #[arg(long)]
    print_openapi: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.print_openapi {
        println!("{}", outpost_gateway::openapi::spec_json());
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("outpost_gateway=info,outpost_core=info,tower_http=info")),
        )
        .init();

    let mut config = Config::local_default();
    config.data_dir = args.data_dir.unwrap_or_else(default_data_dir);
    config.ollama_url = args.ollama_url;
    if let Some(bind) = args.bind {
        config.bind = bind;
    }
    tracing::info!(data_dir = %config.data_dir.display(), "opening Outpost data");
    let mut state = AppState::open(config).await?;
    if args.bind.is_none() {
        state.apply_network_setting().await?;
    }

    let bind = state.config.bind;
    if !bind.ip().is_loopback() {
        tracing::warn!(
            %bind,
            "listening on the network: app keys work from other devices, PIN sign-in stays on this PC. Plain HTTP, so use a network you trust"
        );
    }

    outpost_gateway::serve_state(state).await
}
