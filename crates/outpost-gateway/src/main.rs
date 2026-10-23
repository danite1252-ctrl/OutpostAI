use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use outpost_gateway::config::{default_data_dir, DEFAULT_OLLAMA_URL};
use outpost_gateway::Config;
use tracing_subscriber::EnvFilter;

/// Outpost local API gateway.
#[derive(Parser)]
#[command(name = "outpost-gateway", version, about)]
struct Args {
    /// Folder for the database and account files.
    #[arg(long, env = "OUTPOST_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Address to listen on.
    #[arg(long, env = "OUTPOST_BIND", default_value = "127.0.0.1:7420")]
    bind: SocketAddr,

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

    if !args.bind.ip().is_loopback() {
        tracing::warn!(
            bind = %args.bind,
            "listening beyond this PC; pairing is not built yet, so only do this on a network you trust"
        );
    }

    let config = Config {
        data_dir: args.data_dir.unwrap_or_else(default_data_dir),
        bind: args.bind,
        ollama_url: args.ollama_url,
    };
    outpost_gateway::serve(config).await
}
