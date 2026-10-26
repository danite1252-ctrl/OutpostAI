use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use outpost_gateway::config::{default_data_dir, DEFAULT_OLLAMA_URL};
use outpost_gateway::{AppState, Config};

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

    /// Test run: make a dataset for the owner from this folder of pictures, caption
    /// it, queue its training, then serve as usual.
    #[arg(long, value_name = "FOLDER")]
    queue_training: Option<PathBuf>,

    /// The test character's name (with --queue-training).
    #[arg(long, default_value = "Character")]
    train_name: String,

    /// Its trigger word (with --queue-training); made from the name if left out.
    #[arg(long)]
    train_trigger: Option<String>,

    /// Training steps (with --queue-training).
    #[arg(long, default_value_t = 2000)]
    train_steps: u32,

    /// Act as a local MCP server for apps like Claude Desktop: read MCP messages on
    /// stdin and pass them to the running Outpost (needs --key or OUTPOST_KEY).
    #[arg(long)]
    mcp_stdio: bool,

    /// An Outpost app key, for --mcp-stdio.
    #[arg(long, env = "OUTPOST_KEY", hide_env_values = true)]
    key: Option<String>,

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

    if args.mcp_stdio {
        // stdout carries MCP messages only: no logging here.
        let key = args
            .key
            .ok_or_else(|| anyhow::anyhow!("--mcp-stdio needs an app key: --key op_... or OUTPOST_KEY"))?;
        let port = args.bind.map(|b| b.port()).unwrap_or(outpost_gateway::config::DEFAULT_PORT);
        return outpost_gateway::mcp::stdio_bridge(format!("http://127.0.0.1:{port}/mcp"), key).await;
    }

    // Logs to the console and <data>/logs; crash reports to <data>/crash-reports.
    let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
    outpost_gateway::diagnostics::init(
        &data_dir,
        "gateway",
        "outpost_gateway=info,outpost_core=info,outpost_engines=info,tower_http=info",
    );

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

    if let Some(folder) = args.queue_training {
        tokio::spawn(outpost_gateway::train::queue_from_folder(
            state.clone(),
            folder,
            args.train_name,
            args.train_trigger,
            args.train_steps,
        ));
    }
    let result = outpost_gateway::serve_state(state).await;
    outpost_gateway::diagnostics::clean_exit();
    result
}
